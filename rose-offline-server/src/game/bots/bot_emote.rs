use std::time::Duration;

use bevy::{ecs::query::QueryData, prelude::{Commands, Component, Query, Res, With}};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::{seq::SliceRandom, Rng};
use rose_data::MotionId;

use crate::game::{
    bots::{is_idle_for_duration, is_town_zone, BotQueryFilterAlive, IDLE_DURATION},
    components::{Command, NextCommand, Position},
    resources::GameData,
};

/// Common iRose emote motions: wave, dance, cheer, bow, clap.
const BOT_EMOTE_MOTIONS: &[u16] = &[15, 16, 17, 18, 19];
const EMOTE_DURATION: Duration = Duration::from_secs(2);

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldEmote {
    pub score: f32,
    pub chance_per_tick: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct BotEmote;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    command: &'w Command,
    position: &'w Position,
}

pub fn score_should_emote(
    mut query: Query<(&ShouldEmote, &Actor, &mut Score)>,
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

pub fn action_bot_emote(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<BotEmote>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
) {
    let mut rng = rand::thread_rng();

    for (&Actor(entity), mut state) in query.iter_mut() {
        let Ok(bot) = query_bot.get(entity) else {
            *state = ActionState::Failure;
            continue;
        };

        match *state {
            ActionState::Requested => {
                let Some(&motion_id) = BOT_EMOTE_MOTIONS.choose(&mut rng) else {
                    *state = ActionState::Failure;
                    continue;
                };

                commands
                    .entity(entity)
                    .insert(NextCommand::with_emote(MotionId::new(motion_id), true));
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                if bot.command.is_stop_for(EMOTE_DURATION) {
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
