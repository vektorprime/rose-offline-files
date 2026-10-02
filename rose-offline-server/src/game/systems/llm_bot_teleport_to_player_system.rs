//! Keeps LLM buddy bots with their assigned player: spawns stored bots when the player
//! joins a zone, brings them along on every login / warp / zone change, catches them up
//! when they fall far behind, revives them, and keeps them in the player's party.
//!
//! None of this needs the REST API or an LLM; the movement / support behaviour itself is
//! `llm_buddy_follow_system`.

use std::{f32::consts::FRAC_PI_4, time::Duration};

use bevy::{
    ecs::query::QueryData,
    math::{Vec2, Vec3, Vec3Swizzles},
    prelude::*,
};
use uuid::Uuid;

use crate::game::{
    api::BotInfo,
    bundles::{client_entity_join_zone, client_entity_leave_zone, CharacterBundle},
    components::{
        AbilityValues, CharacterInfo, ClientEntity, ClientEntitySector, ClientEntityType,
        ClientEntityVisibility, Command, DamageSources, Dead, EquipmentItemDatabase, GameClient,
        HealthPoints, LlmBuddyBot, ManaPoints, MotionData, NextCommand, Party, PartyMembership,
        PassiveRecoveryTime, Position, StatusEffects, Team,
    },
    events::PartyEvent,
    resources::{ClientEntityList, GameData},
    storage::llm_buddy_bot::LlmBuddyBotStorage,
    systems::LlmBotManagerResource,
};

use rose_game_common::components::{MoveMode, MoveSpeed};

/// A following bot further than this from its player (same zone) is teleported to them.
const BUDDY_CATCH_UP_TELEPORT_CM: f32 = 4000.0;
/// How long a buddy bot stays dead before it revives next to its player.
const BUDDY_REVIVE_DELAY: Duration = Duration::from_secs(10);
/// How far from the player a spawned / teleported bot is placed.
const BUDDY_ARRIVAL_OFFSET_CM: f32 = 200.0;

/// A buddy bot, as kept with its player by this system.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct BuddyKeeperQuery<'w> {
    entity: Entity,
    buddy: &'w mut LlmBuddyBot,
    position: &'w mut Position,
    command: &'w mut Command,
    next_command: &'w mut NextCommand,
    party_membership: &'w mut PartyMembership,
    ability_values: &'w AbilityValues,
    client_entity: Option<&'w ClientEntity>,
    client_entity_sector: Option<&'w ClientEntitySector>,
    dead: Option<&'w Dead>,
}

/// Keeps every LLM buddy bot with its assigned player.
///
/// When the player joins a zone (login, warp, zone change, revive) their stored bots are
/// spawned if they are not in the world yet, and live bots are moved next to them,
/// revived if dead, put back into follow mode and into the player's party. Between
/// joins, a following bot that ends up in another zone or very far away is teleported
/// back, and a dead bot revives next to the player after a short delay.
pub fn llm_bot_teleport_to_player_on_login_system(
    mut commands: Commands,
    players_joining: Query<
        Entity,
        (
            Added<ClientEntityVisibility>,
            With<GameClient>,
            Without<LlmBuddyBot>,
        ),
    >,
    mut players: Query<
        (
            Entity,
            &CharacterInfo,
            &Position,
            Option<&ClientEntity>,
            &mut PartyMembership,
        ),
        (With<GameClient>, Without<LlmBuddyBot>),
    >,
    mut bots: Query<BuddyKeeperQuery, With<LlmBuddyBot>>,
    party_query: Query<&Party>,
    bot_manager: Res<LlmBotManagerResource>,
    mut client_entity_list: ResMut<ClientEntityList>,
    game_data: Res<GameData>,
    mut party_events: MessageWriter<PartyEvent>,
) {
    let joined_players: Vec<Entity> = players_joining.iter().collect();

    for mut bot in bots.iter_mut() {
        let owner = players
            .iter()
            .find(|(_, character_info, _, _, _)| {
                character_info.name == bot.buddy.assigned_player_name
            })
            .map(|(entity, _, position, client_entity, _)| {
                (entity, position.clone(), client_entity.is_some())
            });

        let Some((owner_entity, owner_position, owner_in_zone)) = owner else {
            // Player offline: the bot waits where it is, but is not left dead or out of
            // its zone.
            if bot.client_entity.is_none()
                || (bot.dead.is_some() && bot.command.is_dead_for(BUDDY_REVIVE_DELAY))
            {
                let position = bot.position.clone();
                place_buddy_bot(
                    &mut commands,
                    &mut client_entity_list,
                    &game_data,
                    &mut bot,
                    &position,
                );
            }
            continue;
        };

        if !owner_in_zone {
            // The player is between zones; the bot follows once they have joined.
            continue;
        }

        let owner_joined_zone = joined_players.contains(&owner_entity);
        if owner_joined_zone {
            // Every login / warp hands the bot back to the built-in follow behaviour.
            bot.buddy.is_following = true;
        }

        let dead = bot.dead.is_some();
        let far_away = bot.position.zone_id != owner_position.zone_id
            || bot
                .position
                .position
                .xy()
                .distance(owner_position.position.xy())
                > BUDDY_CATCH_UP_TELEPORT_CM;
        let needs_teleport = owner_joined_zone
            || bot.client_entity.is_none()
            || (dead && bot.command.is_dead_for(BUDDY_REVIVE_DELAY))
            || (!dead && bot.buddy.is_following && far_away);

        if needs_teleport {
            place_buddy_bot(
                &mut commands,
                &mut client_entity_list,
                &game_data,
                &mut bot,
                &owner_position,
            );
            log::info!(
                "Moved LLM buddy bot {} to player '{}' in zone {:?}",
                bot.buddy.id,
                bot.buddy.assigned_player_name,
                owner_position.zone_id
            );
        }

        if owner_joined_zone {
            if let Ok((_, _, _, _, mut owner_party)) = players.get_mut(owner_entity) {
                let bot_entity = bot.entity;
                ensure_buddy_party(
                    bot_entity,
                    &mut bot.party_membership,
                    owner_entity,
                    &mut owner_party,
                    &party_query,
                    &mut party_events,
                );
            }
        }
    }

    if joined_players.is_empty() {
        return;
    }

    // Spawn the stored bots of players who just joined a zone that are not in the world yet
    let live_bot_ids: Vec<Uuid> = bots.iter().map(|bot| bot.buddy.id).collect();
    for player_entity in joined_players {
        let Ok((_, character_info, player_position, Some(player_client_entity), _)) =
            players.get(player_entity)
        else {
            continue;
        };

        let saved_bots = match LlmBuddyBotStorage::load_for_player(&character_info.name) {
            Ok(bots) => bots,
            Err(e) => {
                log::error!(
                    "Failed to load saved bots for player '{}': {:?}",
                    character_info.name,
                    e
                );
                continue;
            }
        };

        for bot_storage in saved_bots {
            if live_bot_ids.contains(&bot_storage.bot_id) {
                continue;
            }

            spawn_bot_for_player(
                &mut commands,
                &mut client_entity_list,
                &bot_manager,
                &game_data,
                bot_storage,
                player_position,
                player_client_entity.id.0 as u32,
                player_entity,
                &mut party_events,
            );
        }
    }
}

/// Puts the bot next to `destination` (giving up its current zone slot first), clears its
/// orders and revives it if it is dead.
fn place_buddy_bot(
    commands: &mut Commands,
    client_entity_list: &mut ClientEntityList,
    game_data: &GameData,
    bot: &mut BuddyKeeperQueryItem,
    destination: &Position,
) {
    let previous_position = bot.position.clone();
    if let (Some(client_entity), Some(client_entity_sector)) =
        (bot.client_entity, bot.client_entity_sector)
    {
        client_entity_leave_zone(
            commands,
            client_entity_list,
            bot.entity,
            client_entity,
            client_entity_sector,
            &previous_position,
        );
    }

    let new_position = Position::new(
        buddy_arrival_position(game_data, destination, &previous_position),
        destination.zone_id,
    );
    *bot.position = new_position.clone();
    *bot.command = Command::with_stop();
    *bot.next_command = NextCommand::default();
    bot.buddy.follow_goal = None;

    if bot.dead.is_some() {
        // Same state reset as revive_event_system
        let max_health = bot.ability_values.get_max_health();
        let max_mana = bot.ability_values.get_max_mana();
        commands.entity(bot.entity).remove::<Dead>().insert((
            HealthPoints::new((3 * max_health) / 10),
            ManaPoints::new((3 * max_mana) / 10),
            StatusEffects::default(),
            MoveMode::Run,
            DamageSources::default_character(),
            PassiveRecoveryTime::default(),
        ));
    }

    // Joining with a new client entity id makes every client respawn the bot at its new
    // position (and alive), and makes party_member_update_info_system send the new id.
    match client_entity_join_zone(
        commands,
        client_entity_list,
        bot.entity,
        ClientEntityType::Character,
        &new_position,
    ) {
        Ok(_) => {
            commands
                .entity(bot.entity)
                .insert(ClientEntityVisibility::new());
        }
        Err(e) => {
            log::error!(
                "Failed to place LLM buddy bot {} in zone {:?}: {:?}",
                bot.buddy.id,
                destination.zone_id,
                e
            );
        }
    }
}

/// A walkable spot next to `destination`, on the side the bot was coming from.
fn buddy_arrival_position(
    game_data: &GameData,
    destination: &Position,
    previous_position: &Position,
) -> Vec3 {
    let target = destination.position;
    let preferred_direction = if previous_position.zone_id == destination.zone_id {
        (previous_position.position.xy() - target.xy()).try_normalize()
    } else {
        None
    }
    .unwrap_or(Vec2::NEG_Y);

    let nav_grid = game_data
        .zones
        .get_zone(destination.zone_id)
        .and_then(|zone| zone.nav_grid.as_ref());
    for step in 0..8 {
        let direction = Vec2::from_angle(step as f32 * FRAC_PI_4).rotate(preferred_direction);
        let candidate = target.xy() + direction * BUDDY_ARRIVAL_OFFSET_CM;
        if nav_grid.map_or(true, |nav_grid| {
            !nav_grid.is_blocked_world(candidate.x, candidate.y)
        }) {
            return Vec3::new(candidate.x, candidate.y, target.z);
        }
    }

    target
}

/// Puts the bot into its player's party and makes the player its leader.
fn ensure_buddy_party(
    bot_entity: Entity,
    bot_party: &mut Mut<PartyMembership>,
    owner_entity: Entity,
    owner_party: &mut Mut<PartyMembership>,
    party_query: &Query<&Party>,
    party_events: &mut MessageWriter<PartyEvent>,
) {
    if let Some(party_entity) = bot_party.party {
        match party_query.get(party_entity) {
            Ok(party)
                if party
                    .members
                    .iter()
                    .any(|member| member.get_entity() == Some(owner_entity)) =>
            {
                // game_server_join_system resets the PartyMembership of a player who joins a
                // zone while still an online party member (every warp), which breaks group
                // skills, shared xp and leaving the party. The party itself still lists them.
                if owner_party.party.is_none() {
                    **owner_party = PartyMembership::new(party_entity);
                }

                // On logout the party passes to the bot; give it back to the player.
                if party.owner == bot_entity {
                    party_events.write(PartyEvent::ChangeOwner {
                        owner_entity: bot_entity,
                        new_owner_entity: owner_entity,
                    });
                }
                return;
            }
            Ok(_) => {
                // In a party without its player: leave it, then take the player's invite
                // (party_system handles both in this order).
                party_events.write(PartyEvent::Leave {
                    leaver_entity: bot_entity,
                });
            }
            Err(_) => {
                // The party is gone; a stale membership would make the invite fail.
                **bot_party = PartyMembership::default();
            }
        }
    }

    log::info!(
        "Sending party invite from player entity {:?} to LLM buddy bot entity {:?}",
        owner_entity,
        bot_entity
    );
    party_events.write(PartyEvent::Invite {
        owner_entity,
        invited_entity: bot_entity,
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn_bot_for_player(
    commands: &mut Commands,
    client_entity_list: &mut ClientEntityList,
    bot_manager: &LlmBotManagerResource,
    game_data: &GameData,
    bot_storage: LlmBuddyBotStorage,
    player_position: &Position,
    player_client_entity_id: u32,
    player_entity: Entity,
    party_events: &mut MessageWriter<PartyEvent>,
) {
    log::info!(
        "Spawning saved bot '{}' ({}) for player '{}'",
        bot_storage.name,
        bot_storage.bot_id,
        bot_storage.assigned_player
    );

    let build_type = bot_storage.build_type.parse().unwrap_or_default();
    let bot_build = super::super::bots::get_bot_build(build_type);
    let bot_data = create_bot_character_data_from_storage(game_data, &bot_storage, &bot_build);

    let mut llm_buddy_bot = LlmBuddyBot::new(
        bot_storage.bot_id,
        player_client_entity_id,
        bot_storage.assigned_player.clone(),
    );
    llm_buddy_bot.set_follow_distance(bot_storage.follow_distance);
    // A bot restored on login always follows its player
    llm_buddy_bot.is_following = true;

    let ability_values = game_data.ability_value_calculator.calculate(
        &bot_data.info,
        &bot_data.level,
        &bot_data.equipment,
        &bot_data.basic_stats,
        &bot_data.skill_list,
        &StatusEffects::new(),
    );

    let move_speed = MoveSpeed {
        speed: ability_values.run_speed as f32,
    };

    use rose_data::EquipmentIndex;
    let weapon_motion_type = game_data
        .items
        .get_equipped_weapon_item_data(&bot_data.equipment, EquipmentIndex::Weapon)
        .map(|item_data| item_data.motion_type)
        .unwrap_or(0) as usize;

    let motion_data = MotionData::from_character(
        game_data.motions.as_ref(),
        weapon_motion_type,
        bot_data.info.gender,
    );

    let position = Position::new(
        buddy_arrival_position(game_data, player_position, player_position),
        player_position.zone_id,
    );

    let entity = commands
        .spawn((
            llm_buddy_bot,
            CharacterBundle {
                ability_values,
                basic_stats: bot_data.basic_stats,
                bank: Default::default(),
                cooldowns: Default::default(),
                command: Command::default(),
                damage_sources: DamageSources::default_character(),
                equipment: bot_data.equipment,
                experience_points: bot_data.experience_points,
                health_points: bot_data.health_points,
                hotbar: bot_data.hotbar,
                info: bot_data.info,
                inventory: bot_data.inventory,
                level: bot_data.level,
                mana_points: bot_data.mana_points,
                motion_data,
                move_mode: MoveMode::Run,
                move_speed,
                next_command: NextCommand::default(),
                party_membership: Default::default(),
                passive_recovery_time: Default::default(),
                position: position.clone(),
                quest_state: bot_data.quest_state,
                skill_list: bot_data.skill_list,
                skill_points: bot_data.skill_points,
                stamina: bot_data.stamina,
                stat_points: bot_data.stat_points,
                status_effects: StatusEffects::new(),
                status_effects_regen: Default::default(),
                team: Team::default_character(),
                union_membership: bot_data.union_membership,
                clan_membership: Default::default(),
            },
        ))
        .id();

    match client_entity_join_zone(
        commands,
        client_entity_list,
        entity,
        ClientEntityType::Character,
        &position,
    ) {
        Ok(_) => {
            commands
                .entity(entity)
                .insert(ClientEntityVisibility::new());
            log::info!(
                "Successfully spawned LLM buddy bot '{}' ({}) in zone {:?}",
                bot_storage.name,
                bot_storage.bot_id,
                position.zone_id
            );
        }
        Err(e) => {
            log::error!(
                "Failed to register spawned bot '{}' with ClientEntityList: {:?}",
                bot_storage.name,
                e
            );
        }
    }

    let bot_info = BotInfo::new(
        entity,
        bot_storage.name.clone(),
        Some(bot_storage.assigned_player.clone()),
        bot_storage.level,
        bot_storage.build_type.clone(),
    );
    bot_manager
        .bots_map
        .write()
        .insert(bot_storage.bot_id, bot_info);
    log::info!(
        "Registered spawned LLM buddy bot {} -> entity {:?}",
        bot_storage.bot_id,
        entity
    );

    // The bot accepts it in llm_buddy_bot_auto_accept_party_system
    log::info!(
        "Sending party invite from player entity {:?} to bot '{}' (entity {:?})",
        player_entity,
        bot_storage.name,
        entity
    );
    party_events.write(PartyEvent::Invite {
        owner_entity: player_entity,
        invited_entity: entity,
    });
}

fn create_bot_character_data_from_storage(
    game_data: &GameData,
    bot_storage: &LlmBuddyBotStorage,
    bot_build: &super::super::bots::BotBuild,
) -> crate::game::storage::character::CharacterStorage {
    use rand::seq::SliceRandom;

    const BOT_GENDERS: &[rose_game_common::components::CharacterGender] = &[
        rose_game_common::components::CharacterGender::Male,
        rose_game_common::components::CharacterGender::Female,
    ];
    const BOT_FACES: &[u8] = &[1, 8, 15, 22, 29, 36, 43];
    const BOT_HAIRS: &[u8] = &[0, 5, 10, 15, 20];

    let mut rng = rand::thread_rng();

    let mut bot_data = game_data
        .character_creator
        .create(
            bot_storage.name.clone(),
            *BOT_GENDERS.choose(&mut rng).unwrap(),
            1,
            *BOT_FACES.choose(&mut rng).unwrap(),
            *BOT_HAIRS.choose(&mut rng).unwrap(),
        )
        .expect("Failed to create bot character data");

    let target_level = bot_storage.level as u32;
    while bot_data.level.level < target_level {
        bot_data.level.level += 1;

        bot_data.skill_points.points += game_data
            .ability_value_calculator
            .calculate_levelup_reward_skill_points(bot_data.level.level);

        bot_data.stat_points.points += game_data
            .ability_value_calculator
            .calculate_levelup_reward_stat_points(bot_data.level.level);
    }

    if target_level >= 70 {
        bot_data.info.job = bot_build.job_id.get();
    } else if target_level >= 10 {
        bot_data.info.job = (bot_build.job_id.get() / 100) * 100 + 11;
    }

    super::super::bots::spend_stat_points(
        game_data,
        bot_build,
        &mut bot_data.stat_points,
        &mut bot_data.basic_stats,
    );

    let mut ability_values = game_data.ability_value_calculator.calculate(
        &bot_data.info,
        &bot_data.level,
        &bot_data.equipment,
        &bot_data.basic_stats,
        &bot_data.skill_list,
        &StatusEffects::new(),
    );

    super::super::bots::spend_skill_points(
        game_data,
        bot_build,
        &mut bot_data,
        &mut ability_values,
    );

    super::super::bots::choose_equipment_items(game_data, bot_build, &mut bot_data, target_level);

    let final_ability_values = game_data.ability_value_calculator.calculate(
        &bot_data.info,
        &bot_data.level,
        &bot_data.equipment,
        &bot_data.basic_stats,
        &bot_data.skill_list,
        &StatusEffects::new(),
    );

    // Bot files keep the HP / MP the bot was created with and are never updated (older
    // files even hold a 100 / 100 placeholder), so a restored bot starts at full health.
    bot_data.health_points.hp = final_ability_values.get_max_health();
    bot_data.mana_points.mp = final_ability_values.get_max_mana();
    bot_data.stamina.stamina = if bot_storage.stamina > 0 {
        bot_storage.stamina
    } else {
        rose_game_common::components::MAX_STAMINA
    };

    bot_data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_exists() {}
}
