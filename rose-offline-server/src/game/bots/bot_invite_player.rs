//! Hunting bots now and then ask a nearby real player of a similar level to
//! party up, the way players do in the fields. The invite goes through the
//! normal party system (the player gets the usual party request and can
//! accept or decline). Each bot asks a player at most once, a player is asked
//! by at most one bot every few minutes, and it can be switched off with
//! `GameConfig::bot_invite_players`.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use bevy::{
    math::Vec3Swizzles,
    prelude::{Entity, Query, Res, ResMut, Resource, With, Without},
};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    components::{
        CharacterInfo, ClientEntityType, Dead, GameClient, Level, LlmBuddyBot, Party,
        PartyMembership, Position,
    },
    resources::{ClientEntityList, GameConfig, GameData},
};

use super::{
    bot_chat::{fill_template, first_name, typing_delay, BotChatter, PendingLine},
    bot_personality::{BotLife, BotPersonality},
    is_town_zone, BotBuild, BotCombatTarget, BotRole, VendorBot,
};

/// Players within this distance can be asked.
const INVITE_RANGE: f32 = 1000.0;
/// Level gap within which bots ask players.
const INVITE_LEVEL_GAP: i32 = 8;
/// A player is asked by at most one bot within this time.
const PLAYER_INVITE_COOLDOWN: Duration = Duration::from_secs(10 * 60);

const INVITE_LINES: &[&str] = &[
    "Hey {name}, want to party up?",
    "{name}, wanna hunt together?",
    "Need a party {name}? Sending an invite.",
    "Party, {name}?",
    "Hi {name}, want to group up? Xp is better together.",
];

/// When each player was last asked by a bot.
#[derive(Resource, Default)]
pub struct BotPlayerInvites {
    last_invited: HashMap<Entity, Instant>,
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn bot_invite_player_system(
    mut query_bots: Query<
        (
            Entity,
            &Position,
            &Level,
            &PartyMembership,
            Option<&BotRole>,
            &BotPersonality,
            &mut BotLife,
            &mut BotChatter,
        ),
        (
            With<BotBuild>,
            Without<LlmBuddyBot>,
            Without<VendorBot>,
            Without<Dead>,
            Without<BotCombatTarget>,
        ),
    >,
    query_players: Query<
        (&CharacterInfo, &Level, &PartyMembership),
        (With<GameClient>, Without<Dead>),
    >,
    query_party: Query<&Party>,
    client_entity_list: Res<ClientEntityList>,
    game_data: Res<GameData>,
    game_config: Res<GameConfig>,
    mut invites: ResMut<BotPlayerInvites>,
) {
    if !game_config.bot_invite_players {
        return;
    }

    let mut rng = rand::thread_rng();
    let now = Instant::now();

    if invites.last_invited.len() > 256 {
        invites
            .last_invited
            .retain(|_, at| now.saturating_duration_since(*at) < PLAYER_INVITE_COOLDOWN);
    }

    for (bot_entity, position, level, party_membership, role, personality, mut life, mut chatter) in
        query_bots.iter_mut()
    {
        if now < life.next_invite_scan_at {
            continue;
        }
        life.next_invite_scan_at = now + Duration::from_secs(rng.gen_range(20..40));

        if !matches!(role, Some(BotRole::Fighter | BotRole::Healer))
            || chatter.pending.is_some()
            || !rng.gen_bool((0.1 + 0.3 * personality.sociability) as f64)
        {
            continue;
        }

        // Solo, or leading a party with room.
        if let Some(party) = party_membership
            .party
            .and_then(|party| query_party.get(party).ok())
        {
            if party.owner != bot_entity || party.members.len() >= 5 {
                continue;
            }
        }

        let Some(zone_data) = game_data.zones.get_zone(position.zone_id) else {
            continue;
        };
        if is_town_zone(zone_data) {
            continue;
        }
        let Some(zone_entities) = client_entity_list.get_zone(position.zone_id) else {
            continue;
        };

        let candidate = zone_entities
            .iter_entity_type_within_distance(
                position.position.xy(),
                INVITE_RANGE,
                &[ClientEntityType::Character],
            )
            .find_map(|(player, _)| {
                let (player_info, player_level, player_party) = query_players.get(player).ok()?;
                let eligible = player_party.party.is_none()
                    && !player_party.pending_invites.contains(&bot_entity)
                    && (player_level.level as i32 - level.level as i32).abs() <= INVITE_LEVEL_GAP
                    && !life.invited_players.contains(&player)
                    && invites.last_invited.get(&player).map_or(true, |at| {
                        now.saturating_duration_since(*at) >= PLAYER_INVITE_COOLDOWN
                    });
                eligible.then(|| (player, player_info.name.clone()))
            });

        let Some((player, player_name)) = candidate else {
            continue;
        };
        let Some(template) = INVITE_LINES.choose(&mut rng) else {
            continue;
        };

        let text = fill_template(template, &[("name", first_name(&player_name))]);
        let say_at = now + typing_delay(&mut rng, text.len());
        chatter.queue_line(PendingLine {
            text,
            say_at,
            depth: 0,
            shout: false,
            invite: Some(player),
        });
        invites.last_invited.insert(player, now);
    }
}
