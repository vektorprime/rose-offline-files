//! Town life: what bots do with their time in a town.
//!
//! The old town thinker always scored "wander" highest whenever a bot stood
//! still, so town bots walked to a random spot, waited three seconds and
//! walked again, forever; sitting, emotes, chat and NPC visits never won.
//! `TownLife` is a single long-running action with its own little routine,
//! picked from weights that depend on the bot's personality:
//!
//! - stroll to the plaza, an NPC or somewhere nearby (sometimes with a detour
//!   so paths are not ruler-straight), then look around;
//! - visit an NPC shop, or browse a player vendor stall;
//! - join a group of people standing around, or walk up to someone and start
//!   a conversation (the chat systems carry it on);
//! - sit down for a while, go AFK, or play an emote.
//!
//! Commands are only issued when the bot is free (stopped, sitting, or on its
//! own untargeted walk); anything else (fighting, greeting emotes, following
//! the party) is waited out. Party members that are not the leader only do
//! things on the spot so they stay with their group.

use std::time::{Duration, Instant};

use bevy::{
    ecs::query::QueryData,
    math::{Vec2, Vec3, Vec3Swizzles},
    prelude::{Commands, Component, Entity, Has, Query, Res, ResMut, Without},
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    components::{
        CharacterInfo, ClientEntity, ClientEntityType, Command, CommandData, Dead, MoveMode,
        NextCommand, Party, PartyMembership, Position,
    },
    resources::{ClientEntityList, GameData, ServerMessages},
};

use super::{
    bot_chat::{BotChatEvent, BotChatter},
    bot_emotes::{bot_play_emote_kind, BotEmoteKind, BotEmotes},
    bot_personality::{BotActivity, BotArchetype, BotLife, BotPersonality},
    is_town_zone, BotQueryFilterAlive, BotQueryFilterAliveNoTarget, VendorBot,
};

/// A walk leg counts as arrived within this distance.
const ARRIVE_RADIUS: f32 = 120.0;
/// Walks longer than this sometimes take a detour point.
const DETOUR_MIN_DISTANCE: f32 = 1500.0;
/// How far around the bot it looks for vendors.
const VENDOR_SEARCH_RANGE: f32 = 6000.0;
/// How far around the bot it looks for people to stand with.
const CROWD_SEARCH_RANGE: f32 = 2500.0;
/// How far around the bot it looks for someone to talk to.
const SOCIALIZE_SEARCH_RANGE: f32 = 1500.0;
/// A stopped bot that is not where it was sent re-issues its walk after this.
const REORDER_DELAY: Duration = Duration::from_millis(1200);
/// A party member stops what it is doing once its leader is this far away,
/// so the follow behaviour can take over.
const LEADER_LEFT_DISTANCE: f32 = 350.0;

/// Scores while a combat bot stands in a town with nothing to fight, so
/// fighters and healers between hunts live the town routine instead of
/// standing still (towns have no monster spawns to walk to).
#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct InTownZone {
    pub score: f32,
}

pub fn score_in_town_zone(
    mut query: Query<(&InTownZone, &Actor, &mut Score)>,
    query_bot: Query<&Position, BotQueryFilterAliveNoTarget>,
    game_data: Res<GameData>,
) {
    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(position) = query_bot.get(entity) else {
            continue;
        };

        if game_data
            .zones
            .get_zone(position.zone_id)
            .is_some_and(is_town_zone)
        {
            score.set(scorer.score);
        }
    }
}

#[derive(Debug, Clone, Default, Component, ActionBuilder)]
pub struct TownLife {
    step: TownStep,
}

#[derive(Debug, Clone, Default)]
enum TownStep {
    #[default]
    Plan,
    Walk(TownWalk),
    Dwell(TownDwelling),
}

/// What to do once the bot is where it wanted to be.
#[derive(Debug, Clone)]
struct Dwell {
    activity: BotActivity,
    length: Duration,
    sit: bool,
    on_start: Option<BotChatEvent>,
    on_end: Option<BotChatEvent>,
}

#[derive(Debug, Clone)]
struct TownWalk {
    destination: Vec3,
    /// Final destination when `destination` is a detour point.
    then_to: Option<Vec3>,
    move_mode: MoveMode,
    ordered_at: Instant,
    deadline: Instant,
    arrival: Dwell,
}

#[derive(Debug, Clone)]
struct TownDwelling {
    dwell: Dwell,
    until: Instant,
    resat: bool,
}

#[derive(Clone, Copy, Debug)]
enum Activity {
    Stroll,
    VisitNpc,
    BrowseVendor,
    JoinCrowd,
    Socialize,
    Sit,
    Afk,
    LookAround,
    Emote,
}

fn activity_weights(archetype: BotArchetype) -> [(Activity, u32); 9] {
    use Activity::*;
    match archetype {
        BotArchetype::Socialite => [
            (Stroll, 15),
            (VisitNpc, 8),
            (BrowseVendor, 10),
            (JoinCrowd, 20),
            (Socialize, 20),
            (Sit, 8),
            (Afk, 3),
            (LookAround, 8),
            (Emote, 8),
        ],
        BotArchetype::Casual => [
            (Stroll, 14),
            (VisitNpc, 8),
            (BrowseVendor, 8),
            (JoinCrowd, 10),
            (Socialize, 8),
            (Sit, 20),
            (Afk, 12),
            (LookAround, 14),
            (Emote, 4),
        ],
        BotArchetype::Explorer => [
            (Stroll, 40),
            (VisitNpc, 14),
            (BrowseVendor, 8),
            (JoinCrowd, 5),
            (Socialize, 5),
            (Sit, 5),
            (Afk, 2),
            (LookAround, 15),
            (Emote, 4),
        ],
        BotArchetype::Trader => [
            (Stroll, 10),
            (VisitNpc, 18),
            (BrowseVendor, 30),
            (JoinCrowd, 10),
            (Socialize, 10),
            (Sit, 5),
            (Afk, 4),
            (LookAround, 8),
            (Emote, 2),
        ],
        BotArchetype::Grinder => [
            (Stroll, 18),
            (VisitNpc, 30),
            (BrowseVendor, 15),
            (JoinCrowd, 5),
            (Socialize, 5),
            (Sit, 10),
            (Afk, 3),
            (LookAround, 10),
            (Emote, 2),
        ],
    }
}

/// Party members that are not the leader stay on the spot.
const FOLLOWER_WEIGHTS: [(Activity, u32); 3] = [
    (Activity::LookAround, 6),
    (Activity::Sit, 2),
    (Activity::Emote, 1),
];

fn emote_kind_for(rng: &mut impl Rng, archetype: BotArchetype) -> BotEmoteKind {
    let kinds: &[BotEmoteKind] = match archetype {
        BotArchetype::Socialite => &[
            BotEmoteKind::Dance,
            BotEmoteKind::Laugh,
            BotEmoteKind::Greet,
            BotEmoteKind::Cheer,
        ],
        BotArchetype::Casual => &[BotEmoteKind::Sad, BotEmoteKind::Laugh, BotEmoteKind::Any],
        _ => &[BotEmoteKind::Any],
    };
    kinds.choose(rng).copied().unwrap_or(BotEmoteKind::Any)
}

/// True while the bot is doing something TownLife must not interrupt.
fn is_busy(command: &Command) -> bool {
    !matches!(
        command.command,
        CommandData::Stop { .. } | CommandData::Sit | CommandData::Move { target: None, .. }
    )
}

fn secs(rng: &mut impl Rng, min: f32, max: f32) -> Duration {
    Duration::from_secs_f32(rng.gen_range(min..=max))
}

fn random_offset(rng: &mut impl Rng, min: f32, max: f32) -> Vec3 {
    let angle = rng.gen_range(0.0..std::f32::consts::TAU);
    let distance = rng.gen_range(min..=max);
    Vec3::new(angle.cos() * distance, angle.sin() * distance, 0.0)
}

/// A walkable point at or near `point` (towns have walls and houses).
fn walkable(zone_data: &rose_data::ZoneData, point: Vec3) -> Option<Vec3> {
    let Some(nav_grid) = zone_data.nav_grid.as_ref() else {
        return Some(point);
    };
    if !nav_grid.is_blocked_world(point.x, point.y) {
        return Some(point);
    }
    nav_grid
        .nearest_unblocked(point.x, point.y, 6)
        .map(|(x, y)| Vec3::new(x, y, point.z))
}

/// A point `distance` away from `anchor` on the side facing `from`.
fn in_front_of(anchor: Vec3, from: Vec3, distance: f32) -> Vec3 {
    let direction = (from.xy() - anchor.xy()).try_normalize().unwrap_or(Vec2::X);
    anchor + Vec3::new(direction.x * distance, direction.y * distance, 0.0)
}

fn stroll_target(rng: &mut impl Rng, zone_data: &rose_data::ZoneData, from: Vec3) -> Option<Vec3> {
    for _ in 0..5 {
        let roll = rng.gen_range(0..100);
        let candidate = if roll < 45 {
            // The plaza / town centre draws a crowd.
            zone_data.start_position + random_offset(rng, 0.0, 900.0)
        } else if roll < 75 {
            zone_data
                .npcs
                .choose(rng)
                .map(|npc| npc.position)
                .unwrap_or(zone_data.start_position)
                + random_offset(rng, 250.0, 650.0)
        } else {
            from + random_offset(rng, 500.0, 1500.0)
        };

        if candidate.xy().distance(from.xy()) < 300.0 {
            continue;
        }
        if let Some(point) = walkable(zone_data, candidate) {
            return Some(point);
        }
    }
    None
}

/// A nearby NPC: the closest of a few random picks, so bots favour nearby
/// shops without all going to the same one.
fn npc_target(rng: &mut impl Rng, zone_data: &rose_data::ZoneData, from: Vec3) -> Option<Vec3> {
    let npc = (0..6)
        .filter_map(|_| zone_data.npcs.choose(rng))
        .min_by(|a, b| {
            a.position
                .xy()
                .distance_squared(from.xy())
                .total_cmp(&b.position.xy().distance_squared(from.xy()))
        })?;
    let spot = in_front_of(npc.position, from, rng.gen_range(180.0..280.0))
        + random_offset(rng, 0.0, 60.0);
    walkable(zone_data, spot)
}

#[derive(QueryData)]
#[query_data(mutable)]
pub struct TownBotQuery<'w> {
    client_entity: &'w ClientEntity,
    position: &'w Position,
    command: &'w Command,
    party_membership: &'w PartyMembership,
    personality: Option<&'w BotPersonality>,
    life: Option<&'w mut BotLife>,
    chatter: Option<&'w mut BotChatter>,
}

fn set_activity(life: &mut Option<bevy::prelude::Mut<BotLife>>, activity: BotActivity) {
    if let Some(life) = life.as_mut() {
        if life.activity != activity {
            life.activity = activity;
        }
    }
}

fn notify(
    rng: &mut impl Rng,
    chatter: &mut Option<bevy::prelude::Mut<BotChatter>>,
    event: &Option<BotChatEvent>,
) {
    if let (Some(chatter), Some(event)) = (chatter.as_mut(), event.as_ref()) {
        chatter.notify(rng, event.clone());
    }
}

fn order_move(commands: &mut Commands, entity: Entity, destination: Vec3, move_mode: MoveMode) {
    commands
        .entity(entity)
        .insert(NextCommand::with_move(destination, None, Some(move_mode)));
}

/// Starts walking to `destination` (maybe via a detour) and dwelling there.
#[allow(clippy::too_many_arguments)]
fn start_walk(
    commands: &mut Commands,
    rng: &mut impl Rng,
    zone_data: &rose_data::ZoneData,
    entity: Entity,
    from: Vec3,
    destination: Vec3,
    personality: Option<&BotPersonality>,
    now: Instant,
    arrival: Dwell,
) -> TownStep {
    let distance = from.xy().distance(destination.xy());
    let run = personality.is_some_and(|personality| personality.runs_in_town)
        || (distance > 3500.0 && rng.gen_bool(0.4));
    let move_mode = if run { MoveMode::Run } else { MoveMode::Walk };

    // Long walks sometimes bend through a point off the straight line.
    let detour = (distance > DETOUR_MIN_DISTANCE && rng.gen_bool(0.5))
        .then(|| {
            let along = from
                .xy()
                .lerp(destination.xy(), rng.gen_range(0.35..0.65f32));
            let sign = if rng.gen_bool(0.5) { 1.0f32 } else { -1.0f32 };
            let side = (destination.xy() - from.xy()).perp().normalize_or_zero()
                * (distance * rng.gen_range(0.1..0.25f32) * sign);
            let point = along + side;
            walkable(zone_data, Vec3::new(point.x, point.y, destination.z))
        })
        .flatten();

    let (first, then_to) = match detour {
        Some(detour) => (detour, Some(destination)),
        None => (destination, None),
    };
    order_move(commands, entity, first, move_mode);

    // Generous: walking speed is ~1-2 m/s and a route can wind.
    let deadline = now + Duration::from_secs_f32((distance / 100.0).max(15.0) + 15.0);
    TownStep::Walk(TownWalk {
        destination: first,
        then_to,
        move_mode,
        ordered_at: now,
        deadline,
        arrival,
    })
}

#[allow(clippy::too_many_arguments)]
fn start_dwell(
    commands: &mut Commands,
    rng: &mut impl Rng,
    entity: Entity,
    command: &Command,
    life: &mut Option<bevy::prelude::Mut<BotLife>>,
    chatter: &mut Option<bevy::prelude::Mut<BotChatter>>,
    now: Instant,
    dwell: Dwell,
) -> TownStep {
    if dwell.sit && !command.is_sit() {
        commands.entity(entity).insert(NextCommand::with_sitting());
    }
    set_activity(life, dwell.activity);
    notify(rng, chatter, &dwell.on_start);
    TownStep::Dwell(TownDwelling {
        until: now + dwell.length,
        dwell,
        resat: false,
    })
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn action_town_life(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState, &mut TownLife)>,
    mut query_bot: Query<TownBotQuery, BotQueryFilterAlive>,
    query_others: Query<(&Command, Option<&CharacterInfo>, Has<VendorBot>), Without<Dead>>,
    query_party: Query<&Party>,
    query_positions: Query<&Position>,
    client_entity_list: Res<ClientEntityList>,
    game_data: Res<GameData>,
    emotes: Res<BotEmotes>,
    mut server_messages: ResMut<ServerMessages>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    for (&Actor(entity), mut state, mut town_life) in query.iter_mut() {
        let Ok(mut bot) = query_bot.get_mut(entity) else {
            *state = ActionState::Failure;
            continue;
        };

        match *state {
            ActionState::Requested => {
                town_life.step = TownStep::Plan;
                *state = ActionState::Executing;
            }
            ActionState::Executing => {}
            ActionState::Cancelled => {
                set_activity(&mut bot.life, BotActivity::Idle);
                *state = ActionState::Failure;
                continue;
            }
            _ => continue,
        }

        let Some(zone_data) = game_data.zones.get_zone(bot.position.zone_id) else {
            *state = ActionState::Failure;
            continue;
        };
        let from = bot.position.position;

        // A party member whose leader walked off drops what it is doing and
        // waits for the follow behaviour instead of starting something new.
        let leader_left = bot
            .party_membership
            .party
            .and_then(|party| query_party.get(party).ok())
            .filter(|party| party.owner != entity)
            .and_then(|party| query_positions.get(party.owner).ok())
            .is_some_and(|leader_position| {
                leader_position.zone_id == bot.position.zone_id
                    && leader_position.position.xy().distance(from.xy()) > LEADER_LEFT_DISTANCE
            });

        match &mut town_life.step {
            TownStep::Walk(walk) => {
                if is_busy(bot.command) {
                    continue;
                }
                if now >= walk.deadline {
                    set_activity(&mut bot.life, BotActivity::Idle);
                    town_life.step = TownStep::Plan;
                    continue;
                }

                if from.xy().distance(walk.destination.xy()) <= ARRIVE_RADIUS {
                    if let Some(next) = walk.then_to.take() {
                        walk.destination = next;
                        walk.ordered_at = now;
                        order_move(&mut commands, entity, next, walk.move_mode);
                    } else {
                        let arrival = walk.arrival.clone();
                        town_life.step = start_dwell(
                            &mut commands,
                            &mut rng,
                            entity,
                            bot.command,
                            &mut bot.life,
                            &mut bot.chatter,
                            now,
                            arrival,
                        );
                    }
                } else if bot.command.is_stop_for(REORDER_DELAY)
                    && now.saturating_duration_since(walk.ordered_at) > REORDER_DELAY
                {
                    // Stopped short (an emote or greeting interrupted the
                    // walk): carry on.
                    walk.ordered_at = now;
                    order_move(&mut commands, entity, walk.destination, walk.move_mode);
                }
            }
            TownStep::Dwell(dwelling) => {
                if now >= dwelling.until || leader_left {
                    if bot.command.is_sit() {
                        commands.entity(entity).insert(NextCommand::with_standing());
                    }
                    let on_end = dwelling.dwell.on_end.clone();
                    notify(&mut rng, &mut bot.chatter, &on_end);
                    set_activity(&mut bot.life, BotActivity::Idle);
                    town_life.step = TownStep::Plan;
                    continue;
                }
                if is_busy(bot.command) {
                    continue;
                }
                if dwelling.dwell.sit
                    && !dwelling.resat
                    && !bot.command.is_sit()
                    && bot.command.is_stop_for(Duration::from_millis(1500))
                {
                    // Stood up to wave at someone: sit back down.
                    commands.entity(entity).insert(NextCommand::with_sitting());
                    dwelling.resat = true;
                }
            }
            TownStep::Plan => {
                if is_busy(bot.command) || leader_left {
                    continue;
                }

                let follower = bot
                    .party_membership
                    .party
                    .and_then(|party| query_party.get(party).ok())
                    .is_some_and(|party| party.owner != entity);
                let archetype = bot
                    .personality
                    .map_or(BotArchetype::Casual, |personality| personality.archetype);
                let archetype_weights = activity_weights(archetype);
                let weights: &[(Activity, u32)] = if follower {
                    &FOLLOWER_WEIGHTS
                } else {
                    &archetype_weights
                };
                let activity = weights
                    .choose_weighted(&mut rng, |(_, weight)| *weight)
                    .map(|(activity, _)| *activity)
                    .unwrap_or(Activity::LookAround);

                let look_around = Dwell {
                    activity: BotActivity::Idle,
                    length: secs(&mut rng, 3.0, 10.0),
                    sit: false,
                    on_start: None,
                    on_end: None,
                };

                let zone_entities = client_entity_list.get_zone(bot.position.zone_id);
                // Characters around the bot that match `accept`.
                let nearby = |range: f32, accept: &dyn Fn(&Command, bool) -> bool| {
                    let mut found: Vec<(Entity, Vec3)> = Vec::new();
                    if let Some(zone_entities) = zone_entities {
                        for (other, other_position) in zone_entities
                            .iter_entity_type_within_distance(
                                from.xy(),
                                range,
                                &[ClientEntityType::Character],
                            )
                        {
                            if other == entity {
                                continue;
                            }
                            if let Ok((other_command, _, is_vendor)) = query_others.get(other) {
                                if accept(other_command, is_vendor) {
                                    found.push((other, other_position));
                                }
                            }
                        }
                    }
                    found
                };
                let idle_person = |command: &Command, is_vendor: bool| {
                    !is_vendor && (command.is_sit() || command.is_stop_for(Duration::from_secs(1)))
                };

                let next_step = match activity {
                    Activity::Stroll => stroll_target(&mut rng, zone_data, from).map(|target| {
                        let arrival = Dwell {
                            length: if archetype == BotArchetype::Explorer {
                                secs(&mut rng, 1.0, 5.0)
                            } else {
                                secs(&mut rng, 2.0, 10.0)
                            },
                            ..look_around.clone()
                        };
                        start_walk(
                            &mut commands,
                            &mut rng,
                            zone_data,
                            entity,
                            from,
                            target,
                            bot.personality,
                            now,
                            arrival,
                        )
                    }),
                    Activity::VisitNpc => npc_target(&mut rng, zone_data, from).map(|target| {
                        let arrival = Dwell {
                            activity: BotActivity::Shopping,
                            length: secs(&mut rng, 6.0, 20.0),
                            sit: false,
                            on_start: None,
                            on_end: None,
                        };
                        start_walk(
                            &mut commands,
                            &mut rng,
                            zone_data,
                            entity,
                            from,
                            target,
                            bot.personality,
                            now,
                            arrival,
                        )
                    }),
                    Activity::BrowseVendor => {
                        let vendors =
                            nearby(VENDOR_SEARCH_RANGE, &|_: &Command, is_vendor: bool| {
                                is_vendor
                            });
                        vendors
                            .choose(&mut rng)
                            .and_then(|(_, vendor_position)| {
                                walkable(
                                    zone_data,
                                    in_front_of(
                                        *vendor_position,
                                        from,
                                        rng.gen_range(150.0..220.0),
                                    ),
                                )
                            })
                            .map(|target| {
                                let arrival = Dwell {
                                    activity: BotActivity::Shopping,
                                    length: secs(&mut rng, 10.0, 30.0),
                                    sit: false,
                                    on_start: Some(BotChatEvent::Browsing),
                                    on_end: None,
                                };
                                start_walk(
                                    &mut commands,
                                    &mut rng,
                                    zone_data,
                                    entity,
                                    from,
                                    target,
                                    bot.personality,
                                    now,
                                    arrival,
                                )
                            })
                    }
                    Activity::JoinCrowd => {
                        let people = nearby(CROWD_SEARCH_RANGE, &idle_person);
                        people
                            .choose(&mut rng)
                            .and_then(|(_, person_position)| {
                                walkable(
                                    zone_data,
                                    *person_position + random_offset(&mut rng, 140.0, 260.0),
                                )
                            })
                            .map(|target| {
                                let arrival = Dwell {
                                    activity: BotActivity::Socializing,
                                    length: secs(&mut rng, 20.0, 70.0),
                                    sit: rng.gen_bool(0.2),
                                    on_start: None,
                                    on_end: None,
                                };
                                start_walk(
                                    &mut commands,
                                    &mut rng,
                                    zone_data,
                                    entity,
                                    from,
                                    target,
                                    bot.personality,
                                    now,
                                    arrival,
                                )
                            })
                    }
                    Activity::Socialize => {
                        let people = nearby(SOCIALIZE_SEARCH_RANGE, &idle_person);
                        people
                            .choose(&mut rng)
                            .and_then(|&(person, person_position)| {
                                let name = query_others
                                    .get(person)
                                    .ok()
                                    .and_then(|(_, info, _)| info)
                                    .map(|info| info.name.clone())?;
                                let arrival = Dwell {
                                    activity: BotActivity::Socializing,
                                    length: secs(&mut rng, 15.0, 45.0),
                                    sit: false,
                                    on_start: Some(BotChatEvent::Opener(name)),
                                    on_end: None,
                                };
                                if from.xy().distance(person_position.xy()) < 260.0 {
                                    // Already next to them.
                                    return Some(start_dwell(
                                        &mut commands,
                                        &mut rng,
                                        entity,
                                        bot.command,
                                        &mut bot.life,
                                        &mut bot.chatter,
                                        now,
                                        arrival,
                                    ));
                                }
                                let target = walkable(
                                    zone_data,
                                    in_front_of(person_position, from, rng.gen_range(130.0..190.0)),
                                )?;
                                Some(start_walk(
                                    &mut commands,
                                    &mut rng,
                                    zone_data,
                                    entity,
                                    from,
                                    target,
                                    bot.personality,
                                    now,
                                    arrival,
                                ))
                            })
                    }
                    Activity::Sit => {
                        let length = if archetype == BotArchetype::Casual {
                            secs(&mut rng, 30.0, 135.0)
                        } else {
                            secs(&mut rng, 20.0, 90.0)
                        };
                        let arrival = Dwell {
                            activity: BotActivity::Sitting,
                            length,
                            sit: true,
                            on_start: None,
                            on_end: None,
                        };
                        // Half the time find a nicer spot first.
                        let spot = if !follower && rng.gen_bool(0.5) {
                            stroll_target(&mut rng, zone_data, from)
                        } else {
                            None
                        };
                        Some(match spot {
                            Some(target) => start_walk(
                                &mut commands,
                                &mut rng,
                                zone_data,
                                entity,
                                from,
                                target,
                                bot.personality,
                                now,
                                arrival,
                            ),
                            None => start_dwell(
                                &mut commands,
                                &mut rng,
                                entity,
                                bot.command,
                                &mut bot.life,
                                &mut bot.chatter,
                                now,
                                arrival,
                            ),
                        })
                    }
                    Activity::Afk => {
                        let dwell = Dwell {
                            activity: BotActivity::Afk,
                            length: secs(&mut rng, 60.0, 240.0),
                            sit: rng.gen_bool(0.5),
                            on_start: Some(BotChatEvent::GoingAfk),
                            on_end: Some(BotChatEvent::BackFromAfk),
                        };
                        Some(start_dwell(
                            &mut commands,
                            &mut rng,
                            entity,
                            bot.command,
                            &mut bot.life,
                            &mut bot.chatter,
                            now,
                            dwell,
                        ))
                    }
                    Activity::LookAround => {
                        if !bot.command.is_sit() && rng.gen_bool(0.4) {
                            // A small step turns the character, like a player
                            // adjusting the camera and shuffling about.
                            let step = from + random_offset(&mut rng, 60.0, 120.0);
                            walkable(zone_data, step).map(|target| {
                                start_walk(
                                    &mut commands,
                                    &mut rng,
                                    zone_data,
                                    entity,
                                    from,
                                    target,
                                    bot.personality,
                                    now,
                                    look_around.clone(),
                                )
                            })
                        } else {
                            Some(start_dwell(
                                &mut commands,
                                &mut rng,
                                entity,
                                bot.command,
                                &mut bot.life,
                                &mut bot.chatter,
                                now,
                                look_around.clone(),
                            ))
                        }
                    }
                    Activity::Emote => {
                        let kind = emote_kind_for(&mut rng, archetype);
                        bot_play_emote_kind(
                            &mut commands,
                            &mut server_messages,
                            &emotes,
                            &mut rng,
                            entity,
                            bot.client_entity,
                            kind,
                        )
                        .then(|| {
                            let dwell = Dwell {
                                length: secs(&mut rng, 2.0, 5.0),
                                ..look_around.clone()
                            };
                            start_dwell(
                                &mut commands,
                                &mut rng,
                                entity,
                                bot.command,
                                &mut bot.life,
                                &mut bot.chatter,
                                now,
                                dwell,
                            )
                        })
                    }
                };

                town_life.step = match next_step {
                    Some(step) => step,
                    // Nothing suitable around: just look around for a bit.
                    None => start_dwell(
                        &mut commands,
                        &mut rng,
                        entity,
                        bot.command,
                        &mut bot.life,
                        &mut bot.chatter,
                        now,
                        look_around,
                    ),
                };
            }
        }
    }
}
