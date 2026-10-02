use std::time::Duration;

use bevy::{ecs::query::QueryData, prelude::{Commands, Component, Query, Res, With}};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::Rng;

use crate::game::{
    bots::{is_idle_for_duration, is_town_zone, BotQueryFilterAlive, IDLE_DURATION},
    components::{Command, NextCommand, Position},
    resources::GameData,
};

const SIT_DURATION: Duration = Duration::from_secs(8);

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldSitInTown {
    pub score: f32,
    pub chance_per_tick: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct SitInTown;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    command: &'w Command,
    position: &'w Position,
}

pub fn score_should_sit_in_town(
    mut query: Query<(&ShouldSitInTown, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    game_data: Res<GameData>,
) {
    let mut rng = rand::thread_rng();

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(bot) = query_bot.get(entity) else {
            continue;
        };

        if !is_idle_for_duration(bot.command, IDLE_DURATION) {
            continue;
        }

        let Some(zone_data) = game_data.zones.get_zone(bot.position.zone_id) else {
            continue;
        };

        if !is_town_zone(zone_data) {
            continue;
        }

        if rng.gen_bool(scorer.chance_per_tick as f64) {
            score.set(scorer.score);
        }
    }
}

pub fn action_sit_in_town(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<SitInTown>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
) {
    for (&Actor(entity), mut state) in query.iter_mut() {
        let Ok(bot) = query_bot.get(entity) else {
            *state = ActionState::Failure;
            continue;
        };

        match *state {
            ActionState::Requested => {
                commands.entity(entity).insert(NextCommand::with_sitting());
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                if bot.command.is_sit() && bot.command.duration > SIT_DURATION {
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
