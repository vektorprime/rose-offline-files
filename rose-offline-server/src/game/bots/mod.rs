mod bot_accept_party_invite;
mod bot_attack_party_target;
mod bot_attack_target;
mod bot_attack_threat;
mod bot_call_help;
mod bot_chat;
mod bot_chat_reply;
mod bot_cheer_kill;
mod bot_emotes;
mod bot_field_life;
mod bot_find_monster_spawn;
mod bot_find_nearby_target;
mod bot_follow_party_leader;
mod bot_greet;
mod bot_invite_player;
mod bot_join_zone;
mod bot_level_up;
mod bot_navigation;
mod bot_personality;
mod bot_pickup_item;
mod bot_react_emote;
mod bot_revive;
mod bot_send_party_invite;
mod bot_sit_recover_hp;
mod bot_snowball_fight;
mod bot_town_life;
mod bot_use_attack_skill;
mod bot_use_buff_skill;
mod bot_use_heal_skill;
mod bot_vendor;
mod bot_vendor_ad;

mod create_bot;
mod create_llm_buddy_bot;
pub mod names;

use crate::game::components::BasicStats;
pub use create_bot::{
    bot_build_artisan, bot_build_bourgeois, bot_build_champion, bot_build_cleric, bot_build_knight,
    bot_build_mage, bot_build_raider, bot_build_scout, bot_create_random_build,
    bot_create_with_build, spend_skill_points, spend_skill_points_with_bundle, spend_stat_points,
    BotBuild,
};
pub use create_llm_buddy_bot::{
    choose_equipment_items, create_llm_buddy_bot, get_bot_build, process_llm_bot_creations_system,
    LlmBuddyBotBuildType, LlmBuddyBotConfig,
};

use bot_accept_party_invite::{
    action_accept_party_invite, score_has_party_invite, AcceptPartyInvite, HasPartyInvite,
};
use bot_call_help::{bot_call_for_help_system, BotAssistRequests};
use bot_chat::{bot_chatter_system, bot_say, fill_template, first_name, BotChatZoneLimits};
use bot_chat_reply::bot_chat_hear_system;
use bot_cheer_kill::bot_cheer_kill_system;
use bot_emotes::{bot_emotes_init_system, BotEmotes};
use bot_field_life::{
    action_restock_trip, action_take_break, bot_potion_table_init_system, bot_upkeep_system,
    score_needs_restock, score_should_take_break, BotPotionTable, NeedsRestock, RestockTrip,
    ShouldTakeBreak, TakeBreak,
};
use bot_greet::{bot_greet_system, BotGreetLimiter};
use bot_invite_player::{bot_invite_player_system, BotPlayerInvites};
use bot_level_up::bot_level_up_system;
use bot_personality::{bot_assign_personality_system, BotPersonality};
use bot_react_emote::{bot_react_emote_system, player_emote_detect_system, LastEmoteState};
use bot_town_life::{action_town_life, score_in_town_zone, InTownZone, TownLife};
use bot_vendor_ad::bot_vendor_ad_system;
use bot_attack_party_target::{
    action_attack_party_target, score_should_attack_party_target, AttackPartyTarget,
    ShouldAttackPartyTarget,
};
use bot_attack_target::{
    action_attack_target, score_should_attack_target, ActionAttackTarget, ShouldAttackTarget,
};
use bot_attack_threat::{
    action_attack_threat, score_threat_is_not_target, AttackThreat, ThreatIsNotTarget,
};
use bot_find_monster_spawn::{action_find_monster_spawn, FindMonsterSpawns};
use bot_find_nearby_target::{
    action_attack_random_nearby_target, score_find_nearby_target, AttackRandomNearbyTarget,
    FindNearbyTarget,
};
use bot_follow_party_leader::{
    action_follow_party_leader, score_should_follow_party_leader, FollowPartyLeader,
    ShouldFollowPartyLeader,
};
use bot_join_zone::{action_join_zone, score_is_teleporting, IsTeleporting, JoinZone};
pub use bot_navigation::{
    bot_chase_steering_system, bot_navigation_system, bot_rescue_stranded_system,
    bot_separation_system, BotNavigation,
};
use bot_pickup_item::{
    action_pickup_nearest_item_drop, score_find_nearby_item_drop_system, FindNearbyItemDrop,
    PickupNearestItemDrop,
};
use bot_revive::{action_revive_current_zone, score_is_dead, IsDead, ReviveCurrentZone};
use bot_send_party_invite::{
    action_party_invite_nearby_bot, score_can_party_invite_nearby_bot, CanPartyInviteNearbyBot,
    PartyInviteNearbyBot,
};
use bot_sit_recover_hp::{
    action_sit_recover_hp, score_should_sit_recover_hp, ShouldSitRecoverHp, SitRecoverHp,
};
use bot_snowball_fight::{action_snowball_fight, SnowballFight};
use bot_use_attack_skill::{
    action_use_attack_skill, score_should_use_attack_skill, ShouldUseAttackSkill, UseAttackSkill,
};
use bot_use_buff_skill::{
    action_use_buff_skill, score_should_use_buff_skill, ShouldUseBuffSkill, UseBuffSkill,
};
use bot_use_heal_skill::{
    action_use_heal_skill, score_should_use_heal_skill, ShouldUseHealSkill, UseHealSkill,
};
pub use bot_vendor::{
    action_vendor_idle, bot_vendor_thinker, spawn_vendor_bot, VendorBot, VendorIdle,
    VENDOR_BOTS_PER_TOWN,
};

use bevy::{
    ecs::{change_detection::DetectChangesMut, entity::EntityHashSet, schedule::IntoScheduleConfigs},
    prelude::{
        Commands, Component, Entity, MessageWriter, Plugin, Query, Res, ResMut, Startup, Update,
        With, Without,
    },
};
use big_brain::{
    prelude::{ActionState, Actor, Highest},
    thinker::{Thinker, ThinkerBuilder},
    BigBrainPlugin, BigBrainSet,
};
use rand::{seq::SliceRandom, Rng};
use std::time::{Duration, Instant};

use crate::game::{
    bundles::SkillListBundle,
    components::{
        AbilityValues, CharacterInfo, ClientEntity, Command, CommandData, Dead, ExperiencePoints,
        HealthPoints, Inventory, Level, LlmBuddyBot, ManaPoints, NextCommand, PartyMembership,
        Position, SkillList, SkillPoints, Stamina, StatPoints, UnionMembership,
    },
    events::{ChatMessageEvent, PartyEvent, PlayerEmoteEvent},
    resources::{GameData, ServerMessages},
};

const IDLE_DURATION: Duration = Duration::from_millis(250);

/// Number of NPC spawns a zone needs to be considered a town.
pub const TOWN_NPC_THRESHOLD: usize = 12;

/// How long a bot must wait between chat replies (players talking to the
/// bot by name are always answered).
pub const BOT_CHAT_REPLY_COOLDOWN: Duration = Duration::from_secs(12);

/// Maximum number of recent replies a bot remembers.
pub const BOT_CHAT_MEMORY_SIZE: usize = 8;

type BotQueryFilterAlive = (With<ClientEntity>, Without<Dead>);
type BotQueryFilterAliveNoTarget = (With<ClientEntity>, Without<Dead>, Without<BotCombatTarget>);

/// The social role a bot plays in the world. Determines which thinker is used.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotRole {
    /// Stays in towns: strolls, shops, chats, sits, emotes, goes AFK.
    Town,
    /// Hunts monsters in the fields.
    Fighter,
    /// Hunts monsters but prioritises healing themselves and party members.
    Healer,
    /// Stands in towns selling items from a personal store.
    Vendor,
}

/// What a bot remembers about the chat it answered, so it does not answer
/// too often or repeat itself to the same person.
#[derive(Component, Default)]
pub struct BotChatMemory {
    pub last_reply_time: Option<Instant>,
    /// (speaker, topic, when) of recent replies.
    pub replied_to: std::collections::VecDeque<(Entity, bot_chat::BotChatTopic, Instant)>,
}

/// Tracks a pending party invite decision so bots accept with a natural delay.
#[derive(Component)]
pub struct BotPartyInviteResponse {
    pub owner_entity: Entity,
    pub accept_at: Instant,
}

#[derive(Component)]
pub struct BotCombatTarget {
    entity: Entity,
}

impl BotCombatTarget {
    pub fn entity(&self) -> Entity {
        self.entity
    }
}

#[derive(Clone, Debug)]
pub struct BotBehaviorConfig {
    pub threat_is_not_target_score: f32,
    pub use_attack_skill_score: f32,
    pub attack_target_min_score: f32,
    pub attack_target_max_score: f32,
    pub use_heal_skill_score: f32,
    pub use_heal_skill_min_health_percent: f32,
    pub party_heal_min_health_percent: f32,
    pub pickup_item_score: f32,
    /// Hunters take a break now and then (above resting, so a break that
    /// sits down is not pre-empted by the HP rest).
    pub take_break_score: f32,
    pub sit_recover_hp_score: f32,
    /// Walking to a merchant: after resting, before looking for monsters.
    pub restock_score: f32,
    pub party_invite_score: f32,
    pub use_buff_skill_score: f32,
    pub find_nearby_target_score: f32,
    /// Town routine for hunters standing in a town with nothing to fight.
    pub town_life_score: f32,
    pub follow_party_leader_score: f32,
    pub follow_party_leader_distance: f32,
    pub attack_party_target_score: f32,
    /// Minimum party invite accept delay, for natural behaviour.
    pub accept_invite_delay_min: Duration,
    pub accept_invite_delay_max: Duration,
}

impl Default for BotBehaviorConfig {
    fn default() -> Self {
        Self {
            threat_is_not_target_score: 0.9,
            use_attack_skill_score: 0.85,
            attack_target_min_score: 0.6,
            attack_target_max_score: 0.8,
            use_heal_skill_score: 0.7,
            use_heal_skill_min_health_percent: 0.45,
            party_heal_min_health_percent: 0.6,
            pickup_item_score: 0.5,
            take_break_score: 0.45,
            sit_recover_hp_score: 0.4,
            restock_score: 0.38,
            party_invite_score: 0.35,
            use_buff_skill_score: 0.3,
            find_nearby_target_score: 0.2,
            town_life_score: 0.15,
            follow_party_leader_score: 0.95,
            follow_party_leader_distance: 350.0,
            attack_party_target_score: 0.92,
            accept_invite_delay_min: Duration::from_secs(1),
            accept_invite_delay_max: Duration::from_secs(3),
        }
    }
}

pub struct BotPlugin;

impl Plugin for BotPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.init_resource::<BotAssistRequests>()
            .init_resource::<LastEmoteState>()
            .init_resource::<BotEmotes>()
            .init_resource::<BotPotionTable>()
            .init_resource::<BotChatZoneLimits>()
            .init_resource::<BotGreetLimiter>()
            .init_resource::<BotPlayerInvites>()
            .add_message::<PlayerEmoteEvent>()
            // Lookup tables built from the game data once it is loaded.
            .add_systems(Startup, (bot_emotes_init_system, bot_potion_table_init_system))
            // Same schedule as the bot scorers/actions below: big-brain only
            // chains Scorers -> Thinkers -> Actions inside its own schedule,
            // so thinkers pick from this frame's scores and actions run the
            // pick in the same frame.
            .add_plugins(BigBrainPlugin::new(Update))
            .add_systems(
                Update,
                (
                    (
                        (
                            action_accept_party_invite,
                            action_attack_party_target,
                            action_attack_random_nearby_target,
                            action_attack_target,
                            action_attack_threat,
                            action_find_monster_spawn,
                            action_follow_party_leader,
                            action_join_zone,
                            action_party_invite_nearby_bot,
                            action_pickup_nearest_item_drop,
                        )
                            .in_set(BigBrainSet::Actions),
                        (
                            action_restock_trip,
                            action_revive_current_zone,
                            action_sit_recover_hp,
                            action_snowball_fight,
                            action_take_break,
                            action_town_life,
                            action_use_attack_skill,
                            action_use_buff_skill,
                            action_use_heal_skill,
                            action_vendor_idle,
                        )
                            .in_set(BigBrainSet::Actions),
                    ),
                    (
                        (
                            score_can_party_invite_nearby_bot,
                            score_find_nearby_item_drop_system,
                            score_find_nearby_target,
                            score_has_party_invite,
                            score_in_town_zone,
                            score_is_dead,
                            score_is_teleporting,
                            score_needs_restock,
                            score_should_attack_party_target,
                        )
                            .in_set(BigBrainSet::Scorers),
                        (
                            score_should_attack_target,
                            score_should_follow_party_leader,
                            score_should_sit_recover_hp,
                            score_should_take_break,
                            score_should_use_attack_skill,
                            score_should_use_buff_skill,
                            score_should_use_heal_skill,
                            score_threat_is_not_target,
                        )
                            .in_set(BigBrainSet::Scorers),
                    ),
                    (
                        bot_assign_personality_system,
                        bot_auto_accept_party_invites_system,
                        bot_auto_progression_system,
                        bot_call_for_help_system,
                        bot_chat_hear_system,
                        bot_chatter_system,
                        bot_cheer_kill_system,
                        bot_greet_system,
                        bot_invite_player_system,
                        bot_level_up_system,
                        player_emote_detect_system,
                        bot_react_emote_system,
                        bot_upkeep_system,
                        bot_vendor_ad_system,
                    ),
                ),
            );
    }
}

/// Returns true if the zone has enough NPC spawns to be considered a town.
pub fn is_town_zone(zone_data: &rose_data::ZoneData) -> bool {
    zone_data.npcs.len() >= TOWN_NPC_THRESHOLD
}

/// Actors whose skill action `A` is still in progress, for the "keep the
/// score while casting" rule of the skill scorers. big-brain puts the action
/// component on the action entity (with an `Actor` back-reference), never on
/// the actor itself. An executing action only counts while its cast is the
/// live command, or is queued and was accepted by `command_system`
/// (`has_sent_server_message`); a cast it refused stays queued unsent, and
/// must not pin the thinker to the action.
fn actors_with_skill_in_progress<A: Component>(
    query_actions: &Query<(&Actor, &ActionState), With<A>>,
    query_command: &Query<(&Command, &NextCommand)>,
) -> EntityHashSet {
    query_actions
        .iter()
        .filter_map(|(&Actor(actor), state)| {
            let in_progress = match state {
                ActionState::Init | ActionState::Requested => true,
                ActionState::Executing => {
                    query_command
                        .get(actor)
                        .is_ok_and(|(command, next_command)| {
                            matches!(command.command, CommandData::CastSkill { .. })
                                || (next_command.has_sent_server_message
                                    && matches!(
                                        next_command.command,
                                        Some(CommandData::CastSkill { .. })
                                    ))
                        })
                }
                ActionState::Cancelled | ActionState::Success | ActionState::Failure => false,
            };
            in_progress.then_some(actor)
        })
        .collect()
}

/// True when the bot is idle (or sitting) for at least `duration`.
pub fn is_idle_for_duration(command: &Command, duration: Duration) -> bool {
    command.is_stop_for(duration) || (command.is_sit() && command.duration > duration)
}

pub fn bot_thinker() -> ThinkerBuilder {
    bot_thinker_with_role(&BotBehaviorConfig::default(), BotRole::Fighter)
}

pub fn bot_thinker_with_config(config: &BotBehaviorConfig) -> ThinkerBuilder {
    bot_thinker_with_role(config, BotRole::Fighter)
}

pub fn bot_thinker_with_role(config: &BotBehaviorConfig, role: BotRole) -> ThinkerBuilder {
    match role {
        BotRole::Town => bot_town_thinker(config),
        BotRole::Fighter | BotRole::Healer => bot_combat_thinker(config),
        BotRole::Vendor => bot_vendor_thinker(),
    }
}

/// Hunters. Chat is not a thinker choice (see `bot_chat`): bots talk while
/// doing whatever they are doing.
fn bot_combat_thinker(config: &BotBehaviorConfig) -> ThinkerBuilder {
    Thinker::build()
        .picker(Highest)
        .when(IsDead { score: 1.0 }, ReviveCurrentZone)
        .when(IsTeleporting { score: 1.0 }, JoinZone)
        .when(
            ShouldFollowPartyLeader {
                score: config.follow_party_leader_score,
                follow_distance: config.follow_party_leader_distance,
            },
            FollowPartyLeader,
        )
        .when(
            ShouldAttackPartyTarget {
                score: config.attack_party_target_score,
            },
            AttackPartyTarget,
        )
        .when(
            ThreatIsNotTarget {
                score: config.threat_is_not_target_score,
            },
            AttackThreat,
        )
        .when(
            ShouldUseHealSkill {
                score: config.use_heal_skill_score,
                min_health_percent: config.use_heal_skill_min_health_percent,
                party_min_health_percent: config.party_heal_min_health_percent,
            },
            UseHealSkill {
                min_health_percent: config.use_heal_skill_min_health_percent,
                party_min_health_percent: config.party_heal_min_health_percent,
            },
        )
        .when(
            ShouldUseAttackSkill {
                score: config.use_attack_skill_score,
            },
            UseAttackSkill,
        )
        .when(
            ShouldAttackTarget {
                min_score: config.attack_target_min_score,
                max_score: config.attack_target_max_score,
            },
            ActionAttackTarget,
        )
        .when(
            FindNearbyItemDrop {
                score: config.pickup_item_score,
            },
            PickupNearestItemDrop,
        )
        .when(
            ShouldTakeBreak {
                score: config.take_break_score,
            },
            TakeBreak,
        )
        .when(
            ShouldSitRecoverHp {
                score: config.sit_recover_hp_score,
            },
            SitRecoverHp,
        )
        .when(
            NeedsRestock {
                score: config.restock_score,
            },
            RestockTrip::default(),
        )
        .when(
            CanPartyInviteNearbyBot {
                score: config.party_invite_score,
            },
            PartyInviteNearbyBot,
        )
        .when(
            ShouldUseBuffSkill {
                score: config.use_buff_skill_score,
            },
            UseBuffSkill,
        )
        .when(
            FindNearbyTarget {
                score: config.find_nearby_target_score,
            },
            AttackRandomNearbyTarget,
        )
        .when(
            InTownZone {
                score: config.town_life_score,
            },
            TownLife::default(),
        )
        .otherwise(FindMonsterSpawns)
}

/// Town folk: the `TownLife` routine, interrupted only by revives, zone
/// joins, following their party and fighting back.
fn bot_town_thinker(config: &BotBehaviorConfig) -> ThinkerBuilder {
    Thinker::build()
        .picker(Highest)
        .when(IsDead { score: 1.0 }, ReviveCurrentZone)
        .when(IsTeleporting { score: 1.0 }, JoinZone)
        .when(
            ShouldFollowPartyLeader {
                score: config.follow_party_leader_score,
                follow_distance: config.follow_party_leader_distance,
            },
            FollowPartyLeader,
        )
        .when(
            ThreatIsNotTarget {
                score: config.threat_is_not_target_score,
            },
            AttackThreat,
        )
        .when(
            ShouldAttackTarget {
                min_score: config.attack_target_min_score,
                max_score: config.attack_target_max_score,
            },
            ActionAttackTarget,
        )
        .otherwise(TownLife::default())
}

pub fn bot_snowball_fight() -> ThinkerBuilder {
    Thinker::build()
        .picker(Highest)
        .otherwise(SnowballFight::default())
}

const PARTY_ACCEPT_LINES: &[&str] = &[
    "Thanks for the invite!",
    "Sure, let's go!",
    "Thanks {name}!",
    "Ty for the party.",
    "Joined! Let's hunt.",
    "Hi all!",
];

/// Delayed party invite acceptance so bots react naturally, and reply in chat
/// when a player invites them.
#[allow(clippy::type_complexity)]
pub fn bot_auto_accept_party_invites_system(
    mut commands: Commands,
    query: Query<
        (
            Entity,
            &PartyMembership,
            Option<&BotPartyInviteResponse>,
        ),
        (With<BotBuild>, With<ClientEntity>, Without<LlmBuddyBot>, Without<VendorBot>, Without<Dead>),
    >,
    query_bot: Query<(&ClientEntity, &CharacterInfo, &Position, Option<&BotPersonality>)>,
    query_inviter_is_bot: Query<(), (With<BotBuild>, Without<LlmBuddyBot>)>,
    mut party_events: MessageWriter<PartyEvent>,
    mut server_messages: ResMut<ServerMessages>,
    mut chat_message_events: MessageWriter<ChatMessageEvent>,
) {
    let mut rng = rand::thread_rng();

    for (bot_entity, party_membership, invite_response) in query.iter() {
        if party_membership.party.is_some() {
            if invite_response.is_some() {
                commands.entity(bot_entity).remove::<BotPartyInviteResponse>();
            }
            continue;
        }

        let Some(&owner_entity) = party_membership.pending_invites.first() else {
            continue;
        };

        let accept_at = match invite_response {
            Some(response) if response.owner_entity == owner_entity => response.accept_at,
            _ => {
                let config = BotBehaviorConfig::default();
                let delay = rng.gen_range(
                    config.accept_invite_delay_min..=config.accept_invite_delay_max,
                );
                commands.entity(bot_entity).insert(BotPartyInviteResponse {
                    owner_entity,
                    accept_at: Instant::now() + delay,
                });
                continue;
            }
        };

        if Instant::now() < accept_at {
            continue;
        }

        party_events.write(PartyEvent::AcceptInvite {
            owner_entity,
            invited_entity: bot_entity,
        });
        commands.entity(bot_entity).remove::<BotPartyInviteResponse>();

        // Reply in chat when invited by a player so it feels alive.
        if query_inviter_is_bot.get(owner_entity).is_err() {
            let Ok((client_entity, character_info, position, personality)) =
                query_bot.get(bot_entity)
            else {
                continue;
            };
            let inviter_name = query_bot
                .get(owner_entity)
                .map_or("", |(_, inviter_info, _, _)| inviter_info.name.as_str());

            let template = PARTY_ACCEPT_LINES.choose(&mut rng).copied().unwrap_or("");
            let line = fill_template(template, &[("name", first_name(inviter_name))]);
            let message = match personality {
                Some(personality) => personality.stylize(&mut rng, &line),
                None => line,
            };

            bot_say(
                &mut server_messages,
                &mut chat_message_events,
                bot_entity,
                client_entity,
                &character_info.name,
                position.zone_id,
                message,
                false,
            );
        }
    }
}

/// Spends unspent stat / skill points. Points the build cannot use yet stay
/// unspent, so the spend runs without change detection and the components
/// are only marked changed when something was actually spent (otherwise every
/// such bot forced a full ability recalculation and a party info update every
/// tick).
pub fn bot_auto_progression_system(
    game_data: Res<GameData>,
    mut query: Query<(
        &BotBuild,
        &CharacterInfo,
        &Level,
        &mut StatPoints,
        &mut BasicStats,
        &mut SkillList,
        &mut SkillPoints,
        &AbilityValues,
        &ExperiencePoints,
        &Inventory,
        &Stamina,
        &UnionMembership,
        &HealthPoints,
        &ManaPoints,
    )>,
) {
    for (
        bot_build,
        character_info,
        level,
        mut stat_points,
        mut basic_stats,
        mut skill_list,
        mut skill_points,
        ability_values,
        experience_points,
        inventory,
        stamina,
        union_membership,
        health_points,
        mana_points,
    ) in query.iter_mut()
    {
        if stat_points.points > 0
            && spend_stat_points(
                &game_data,
                bot_build,
                stat_points.bypass_change_detection(),
                basic_stats.bypass_change_detection(),
            )
        {
            stat_points.set_changed();
            basic_stats.set_changed();
        }

        if skill_points.points > 0 {
            let mut skill_list_bundle = SkillListBundle {
                skill_list: skill_list.bypass_change_detection(),
                skill_points: Some(skill_points.bypass_change_detection()),
                game_client: None,
                ability_values,
                level,
                move_speed: None,
                team: None,
                character_info: Some(character_info),
                experience_points: Some(experience_points),
                inventory: Some(inventory),
                stamina: Some(stamina),
                stat_points: Some(&*stat_points),
                union_membership: Some(union_membership),
                health_points: Some(health_points),
                mana_points: Some(mana_points),
            };

            if spend_skill_points_with_bundle(&game_data, bot_build, &mut skill_list_bundle) {
                skill_list.set_changed();
                skill_points.set_changed();
            }
        }
    }
}
