use bevy::prelude::{Commands, Component, Has, Query, With};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};

use crate::game::components::{AbilityValues, Command, HealthPoints, ManaPoints, NextCommand};

use super::{bot_personality::BotPersonality, BotCombatTarget, BotQueryFilterAlive};

/// Bots without a personality sit below this health fraction...
const DEFAULT_REST_BELOW_HP: f32 = 0.4;
/// ...and get up again at this one.
const DEFAULT_REST_UNTIL_HP: f32 = 1.0;
/// Out of a fight, bots also sit down when their mana is this low...
const REST_BELOW_MP: f32 = 0.15;
/// ...and wait for this much before getting up.
const REST_UNTIL_MP: f32 = 0.7;

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldSitRecoverHp {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct SitRecoverHp;

pub fn score_should_sit_recover_hp(
    mut query: Query<(&ShouldSitRecoverHp, &Actor, &mut Score)>,
    query_entity: Query<
        (
            &Command,
            &AbilityValues,
            &HealthPoints,
            &ManaPoints,
            Option<&BotPersonality>,
            Has<BotCombatTarget>,
        ),
        BotQueryFilterAlive,
    >,
) {
    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok((command, ability_values, health_points, mana_points, personality, in_fight)) =
            query_entity.get(entity)
        else {
            continue;
        };

        let (rest_below_hp, rest_until_hp) = personality.map_or(
            (DEFAULT_REST_BELOW_HP, DEFAULT_REST_UNTIL_HP),
            |personality| (personality.rest_below_hp, personality.rest_until_hp),
        );
        let health_percent =
            health_points.hp as f32 / ability_values.get_max_health().max(1) as f32;
        let max_mana = ability_values.get_max_mana();
        let mana_percent = if max_mana > 0 {
            mana_points.mp as f32 / max_mana as f32
        } else {
            1.0
        };

        if command.is_sit() {
            // Sitting already: stay down until rested enough.
            if health_percent < rest_until_hp || (!in_fight && mana_percent < REST_UNTIL_MP) {
                score.set(scorer.score);
            }
        } else if health_percent < rest_below_hp || (!in_fight && mana_percent < REST_BELOW_MP) {
            score.set(scorer.score);
        }
    }
}

pub fn action_sit_recover_hp(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<SitRecoverHp>>,
) {
    for (&Actor(entity), mut state) in query.iter_mut() {
        match *state {
            ActionState::Requested => {
                commands.entity(entity).insert(NextCommand::with_sitting());
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                // This task will execute until it is cancelled
            }
            ActionState::Cancelled => {
                commands.entity(entity).insert(NextCommand::with_standing());
                *state = ActionState::Success;
            }
            _ => {}
        }
    }
}
