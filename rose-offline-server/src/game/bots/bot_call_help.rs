//! Call for help: when a bot gets attacked it shouts for help in zone chat,
//! and nearby idle bots drop what they are doing to attack the attacker.

use std::time::{Duration, Instant};

use bevy::{
    math::Vec3Swizzles,
    prelude::{
        Commands, Component, Entity, MessageReader, MessageWriter, Query, ResMut, Resource, With,
        Without,
    },
};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    components::{
        CharacterInfo, ChatType, ClientEntity, Command, Dead, LlmBuddyBot, NextCommand, Position,
    },
    events::{ChatMessageEvent, DamageEvent},
    messages::server::ServerMessage,
    resources::ServerMessages,
};

use super::{
    bot_personality::{BotActivity, BotLife, BotPersonality},
    BotBuild, BotCombatTarget, VendorBot,
};

/// How long an assist request stays active.
const ASSIST_DURATION: Duration = Duration::from_secs(8);
/// Maximum distance a bot will travel to help.
const ASSIST_RANGE: f32 = 800.0;
/// How long before a bot will shout for help again.
const CALL_COOLDOWN: Duration = Duration::from_secs(45);

const HELP_LINES: &[&str] = &[
    "Help! I'm under attack!",
    "Someone help me, please!",
    "Get it off me!",
    "A little help here!",
];

/// Tracks when a bot last shouted for help.
#[derive(Component, Default)]
pub struct BotCallForHelpState {
    last_call_at: Option<Instant>,
}

#[derive(Clone, Debug)]
pub(crate) struct BotAssistRequest {
    pub defender: Entity,
    pub attacker: Entity,
    pub expires_at: Instant,
}

/// Active "come help me" requests, written by `bot_call_for_help_system`.
#[derive(Default, Resource)]
pub struct BotAssistRequests {
    pub requests: Vec<BotAssistRequest>,
}

pub fn bot_call_for_help_system(
    mut commands: Commands,
    mut damage_events: MessageReader<DamageEvent>,
    mut query_bots: Query<
        (
            Entity,
            &ClientEntity,
            &CharacterInfo,
            &Position,
            &Command,
            Option<&mut BotCallForHelpState>,
        ),
        (With<BotBuild>, Without<LlmBuddyBot>, Without<VendorBot>, Without<Dead>),
    >,
    query_helper: Query<(Option<&BotPersonality>, Option<&BotLife>)>,
    mut assist_requests: ResMut<BotAssistRequests>,
    mut server_messages: ResMut<ServerMessages>,
    mut chat_message_events: MessageWriter<ChatMessageEvent>,
) {
    let now = Instant::now();
    let mut rng = rand::thread_rng();

    // Forget expired requests.
    assist_requests
        .requests
        .retain(|request| request.expires_at > now);

    let mut fresh: Vec<(Entity, Entity)> = Vec::new();
    let mut shouted_this_frame: std::collections::HashSet<Entity> = std::collections::HashSet::new();

    for event in damage_events.read() {
        let (attacker, defender) = match event {
            DamageEvent::Attack {
                attacker, defender, ..
            }
            | DamageEvent::Immediate {
                attacker, defender, ..
            }
            | DamageEvent::Skill {
                attacker, defender, ..
            }
            | DamageEvent::Tagged { attacker, defender } => (*attacker, *defender),
        };

        if attacker == defender {
            continue;
        }

        let Ok((
            bot_entity,
            client_entity,
            character_info,
            position,
            command,
            call_state,
        )) = query_bots.get_mut(defender)
        else {
            continue;
        };

        if command.is_dead() {
            continue;
        }

        // Only shout once per bot, and once per frame.
        let on_cooldown = call_state
            .as_ref()
            .and_then(|state| state.last_call_at)
            .map_or(false, |last| now.saturating_duration_since(last) < CALL_COOLDOWN);
        if on_cooldown || !shouted_this_frame.insert(defender) {
            continue;
        }

        let line = HELP_LINES.choose(&mut rng).unwrap_or(&"").to_string();
        let line = match query_helper.get(defender) {
            Ok((Some(personality), _)) => personality.stylize(&mut rng, &line),
            _ => line,
        };
        server_messages.send_entity_message(
            client_entity,
            ServerMessage::ShoutChat {
                name: character_info.name.clone(),
                text: line.clone(),
            },
        );
        chat_message_events.write(ChatMessageEvent {
            sender_entity: bot_entity,
            sender_name: character_info.name.clone(),
            zone_id: position.zone_id,
            message: line,
            chat_type: ChatType::Shout,
        });

        assist_requests.requests.push(BotAssistRequest {
            defender,
            attacker,
            expires_at: now + ASSIST_DURATION,
        });
        fresh.push((defender, attacker));

        let mut state = match call_state {
            Some(mut state) => std::mem::take(&mut *state),
            None => BotCallForHelpState::default(),
        };
        state.last_call_at = Some(now);
        commands.entity(bot_entity).insert(state);
    }

    // Dispatch idle nearby bots to attack the attacker.
    for (defender, attacker) in fresh {
        // Copy the defender position out so the query borrow ends before we
        // iterate all bots below.
        let (defender_zone_id, defender_pos) = {
            let Ok((_, _, _, defender_position, _, _)) = query_bots.get_mut(defender) else {
                continue;
            };
            (defender_position.zone_id, defender_position.position.xy())
        };

        for (bot_entity, _, _, position, command, _) in query_bots
            .iter()
            .filter(|(_, _, _, position, command, _)| {
                position.zone_id == defender_zone_id && command.is_stop()
            })
        {
            if bot_entity == defender || bot_entity == attacker {
                continue;
            }
            if position.position.xy().distance(defender_pos) > ASSIST_RANGE {
                continue;
            }

            // Not everyone drops what they are doing: helpful characters
            // usually come, AFK ones never do.
            let (personality, life) = query_helper.get(bot_entity).unwrap_or((None, None));
            if life.is_some_and(|life| life.activity == BotActivity::Afk) {
                continue;
            }
            let help_chance =
                personality.map_or(0.7, |personality| 0.3 + 0.6 * personality.sociability);
            if !rng.gen_bool(help_chance as f64) {
                continue;
            }

            commands
                .entity(bot_entity)
                .insert(BotCombatTarget { entity: attacker })
                .insert(NextCommand::with_attack(attacker));
        }
    }
}

