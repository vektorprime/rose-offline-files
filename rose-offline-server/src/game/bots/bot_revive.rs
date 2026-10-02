use std::time::Duration;

use bevy::prelude::{Commands, Component, MessageWriter, Query, Res, With};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};

use crate::game::{
    bots::IDLE_DURATION,
    components::{Command, CommandData, Dead, Npc},
    events::{ReviveEvent, RevivePosition},
    resources::GameData,
};

use super::{
    bot_chat::{BotChatEvent, BotChatter},
    bot_personality::BotPersonality,
    BotCombatTarget,
};

/// Revive delay for bots without a personality.
const DEAD_DURATION: Duration = Duration::from_secs(10);

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct IsDead {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct ReviveCurrentZone;

pub fn score_is_dead(
    mut query: Query<(&IsDead, &Actor, &mut Score)>,
    query_entity: Query<(&Command, Option<&ReviveCurrentZone>), With<Dead>>,
) {
    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok((command, is_reviving)) = query_entity.get(entity) else {
            continue;
        };

        if command.is_dead() || is_reviving.is_some() {
            score.set(scorer.score);
        }
    }
}

pub fn action_revive_current_zone(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<ReviveCurrentZone>>,
    mut query_entity: Query<(&Command, Option<&BotPersonality>, Option<&mut BotChatter>)>,
    query_npc: Query<&Npc>,
    game_data: Res<GameData>,
    mut revive_events: MessageWriter<ReviveEvent>,
) {
    let mut rng = rand::thread_rng();

    for (&Actor(entity), mut state) in query.iter_mut() {
        let Ok((command, personality, chatter)) = query_entity.get_mut(entity) else {
            continue;
        };

        match *state {
            ActionState::Requested => {
                if !command.is_dead() {
                    *state = ActionState::Failure;
                    continue;
                }

                // Players lie there for a while (reading the death message,
                // grumbling) before pressing revive.
                let delay =
                    personality.map_or(DEAD_DURATION, |personality| personality.revive_delay);
                if !command.is_dead_for(delay) {
                    continue;
                }

                if let Some(mut chatter) = chatter {
                    let killer = match &command.command {
                        CommandData::Die {
                            killer: Some(killer),
                            ..
                        } => query_npc
                            .get(*killer)
                            .ok()
                            .and_then(|npc| game_data.npcs.get_npc(npc.id))
                            .map(|npc_data| npc_data.name.clone())
                            .filter(|name| !name.is_empty()),
                        _ => None,
                    };
                    chatter.notify(&mut rng, BotChatEvent::Revived(killer));
                }

                commands.entity(entity).remove::<BotCombatTarget>();
                revive_events.write(ReviveEvent {
                    entity,
                    position: RevivePosition::CurrentZone,
                });
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                if command.is_stop_for(IDLE_DURATION) {
                    // Wait until we are idle
                    *state = ActionState::Success;
                    continue;
                }
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
