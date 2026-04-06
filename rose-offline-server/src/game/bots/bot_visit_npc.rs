use bevy::{
    ecs::query::QueryData,
    prelude::{Commands, Component, Query, With},
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    bots::IDLE_DURATION,
    components::{Command, NextCommand, Npc, Position},
};

use super::BotQueryFilterAlive;

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldVisitNpc {
    pub score: f32,
    pub chance_per_tick: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct VisitNpc;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    command: &'w Command,
    position: &'w Position,
}

pub fn score_should_visit_npc(
    mut query: Query<(&ShouldVisitNpc, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
) {
    let mut rng = rand::thread_rng();

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(bot) = query_bot.get(entity) else {
            continue;
        };

        if !bot.command.is_stop_for(IDLE_DURATION) {
            continue;
        }

        if rng.gen_bool(scorer.chance_per_tick as f64) {
            score.set(scorer.score);
        }
    }
}

pub fn action_visit_npc(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<VisitNpc>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    query_npc: Query<&Position, With<Npc>>,
) {
    let mut rng = rand::thread_rng();

    for (&Actor(entity), mut state) in query.iter_mut() {
        match *state {
            ActionState::Requested => {
                let Ok(bot) = query_bot.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let mut npc_positions = Vec::new();
                for npc_position in query_npc.iter() {
                    if npc_position.zone_id == bot.position.zone_id {
                        npc_positions.push(npc_position.position);
                    }
                }

                let Some(npc_position) = npc_positions.choose(&mut rng) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let destination = *npc_position
                    + bevy::math::Vec3::new(
                        rng.gen_range(-350.0..350.0),
                        rng.gen_range(-350.0..350.0),
                        0.0,
                    );

                commands
                    .entity(entity)
                    .insert(NextCommand::with_move(destination, None, None));
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                let Ok(bot) = query_bot.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                if bot.command.is_stop_for(IDLE_DURATION) {
                    *state = ActionState::Success;
                }
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
