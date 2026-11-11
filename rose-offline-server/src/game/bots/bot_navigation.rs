//! Server-side bot navigation: route long straight-line moves around walls.
//!
//! Big-brain actions (`bot_find_monster_spawn`, `bot_visit_npc`,
//! `bot_wander_town`, ...) pick a destination and issue a single
//! `NextCommand::with_move`. The server's `update_position_system` then walks
//! the entity straight at the goal with no collision knowledge, so bots clip
//! through buildings and walls on every client's screen.
//!
//! This module rewrites far `target=None` travel orders into waypoint legs
//! from the zone's baked [`rose_data::NavGrid`] (terrain slopes, walls,
//! ledges, object floors, deep water and warp gates; see `rose_data::nav_bake`
//! and `rose-data-irose/src/zone_nav.rs`). Short moves go straight when the
//! straight line is walkable. Target-tracking moves (attack chase, pickup,
//! follow) are steered by `bot_chase_steering_system`: they are close-range
//! and `command_system` recomputes their destination every tick.
//!
//! Server movement can never physically jam (no colliders), so legs always
//! arrive. An unreachable goal is moved to the nearest spot the bot can
//! reach (`NavGrid::reachable_point`); with none, the bot stops instead of
//! walking through a wall, off a cliff or into deep water. A bot that still
//! ends up stranded (on blocked ground, or in a pocket no designer placed
//! point leads to) is put back by `bot_rescue_stranded_system`.

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use bevy::{
    math::{Vec2, Vec3, Vec3Swizzles},
    prelude::{Commands, Component, Entity, Local, Query, Res, ResMut, Without},
    time::Time,
};
use rand::Rng;
use rose_data::ZoneId;

use crate::game::{
    bundles::{client_entity_join_zone, client_entity_leave_zone},
    components::{
        ClientEntity, ClientEntitySector, ClientEntityType, ClientEntityVisibility, Command,
        CommandData, Dead, GameClient, LlmBuddyBot, MoveMode, NextCommand, Position,
    },
    messages::server::ServerMessage,
    resources::{ClientEntityList, GameConfig, GameData, ServerMessages},
};

use super::BotRole;

/// Waypoint legs below this travel distance go direct (no routing).
const NAV_ROUTE_MIN_DIST_CM: f32 = 600.0;
/// Waypoint counts as reached within this radius; also the final-arrival band.
const NAV_WAYPOINT_REACH_CM: f32 = 150.0;
/// Clamp for a single leg: `MoveEntity.distance` is a u16 cm field.
const NAV_MAX_LEG_CM: f32 = 50000.0;
/// How far an unreachable goal may move to the nearest reachable spot.
const NAV_RETARGET_CM: f32 = 2000.0;
/// `NextCommand` destinations within this distance of the fed leg (or the
/// goal) are ours; anything else is a new order and gets a new route.
const NAV_GOAL_EPS_CM: f32 = 1.0;

#[derive(Component, Default)]
pub struct BotNavigation {
    /// The bot's real destination; `NextCommand` holds only the current leg.
    goal: Vec3,
    /// Remaining smoothed waypoints (front = waypoint currently walked to).
    waypoints: VecDeque<Vec3>,
    /// The destination currently fed for the front waypoint (`split_leg`).
    leg: Option<Vec3>,
    /// The waypoint `leg` was split from.
    leg_waypoint: Option<Vec3>,
}

/// Last route query that produced no usable route (unreachable goal, or
/// start and goal snapping to the same cell). `NavGrid::find_path` is a pure
/// function of the snapped cells on the immutable zone grid, so A* only
/// reruns once the snapped start or goal cell changes.
#[derive(Component)]
pub struct BotNavNoRoute {
    zone_id: ZoneId,
    key: (u32, u32),
}

/// The move the chase steering last broadcast for this entity. Cleared
/// whenever something else (re)announces the entity's movement, so the next
/// steering rewrite is broadcast again.
#[derive(Component, Default)]
pub struct BotSteerBroadcast {
    sent: Option<SteerSent>,
}

#[derive(Clone, Copy, PartialEq)]
struct SteerSent {
    destination: Vec3,
    target: Option<Entity>,
    move_mode: Option<MoveMode>,
}

/// Probe distance (cm) for wall checks ahead of a chaser.
const STEER_PROBE_CM: f32 = 300.0;
/// Goal staleness (cm) before a chase route replans to the live target.
const CHASE_REPLAN_CM: f32 = 400.0;
/// Deflection angles tried when the straight step is blocked, in order.
const STEER_ANGLES_DEG: [f32; 8] = [30.0, -30.0, 60.0, -60.0, 90.0, -90.0, 135.0, -135.0];
/// A steered destination is re-broadcast once it drifts this far from the
/// last broadcast one (wall-slide points move with the chaser every tick).
const STEER_REBROADCAST_CM: f32 = 100.0;

/// Outcome of a wall-slide probe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Steer {
    /// The straight step is walkable (or there is no distance to cover).
    Clear,
    /// Deflected destination at the same remaining distance.
    Deflect(f32, f32),
    /// Every probed direction is blocked: needs a full route.
    Surrounded,
}

/// Wall-slide steering: if the straight step from `from` toward `to` enters a
/// blocked cell, return a deflected destination at the same distance.
/// Pure over the grid (unit-tested).
pub fn steer_around_blocker(
    grid: &rose_data::NavGrid,
    from_x: f32,
    from_y: f32,
    to_x: f32,
    to_y: f32,
) -> Steer {
    let dx = to_x - from_x;
    let dy = to_y - from_y;
    let dist = dx.hypot(dy);
    if dist < 1.0 {
        return Steer::Clear;
    }
    let probe = dist.min(STEER_PROBE_CM);
    let step_x = dx / dist * probe;
    let step_y = dy / dist * probe;
    if !grid.is_blocked_world(from_x + step_x, from_y + step_y) {
        return Steer::Clear;
    }
    let base_ang = dy.atan2(dx);
    for deflect in STEER_ANGLES_DEG {
        let ang = base_ang + deflect.to_radians();
        let px = from_x + ang.cos() * probe;
        let py = from_y + ang.sin() * probe;
        if !grid.is_blocked_world(px, py) {
            // Keep full remaining distance so arrival/speed logic is unchanged.
            return Steer::Deflect(from_x + ang.cos() * dist, from_y + ang.sin() * dist);
        }
    }
    Steer::Surrounded
}

fn split_leg(from: Vec3, to: Vec3) -> Vec3 {
    let delta = to.xy() - from.xy();
    let dist = delta.length();
    if dist > NAV_MAX_LEG_CM {
        let clamped = from.xy() + delta * (NAV_MAX_LEG_CM / dist);
        Vec3::new(clamped.x, clamped.y, to.z)
    } else {
        to
    }
}

/// Route query with the negative-result cache. Returns the waypoints after
/// the start point (front = first leg), or `None` when there is no usable
/// route (unreachable, or start and goal in one cell).
#[allow(clippy::too_many_arguments)]
fn find_route(
    commands: &mut Commands,
    entity: Entity,
    nav_grid: &rose_data::NavGrid,
    zone_id: ZoneId,
    no_route: Option<&mut BotNavNoRoute>,
    from: Vec3,
    to: Vec3,
) -> Option<VecDeque<Vec3>> {
    // No key means `find_path` cannot snap and returns None as well.
    let key = nav_grid.route_key(from.x, from.y, to.x, to.y)?;
    if no_route
        .as_ref()
        .is_some_and(|cached| cached.zone_id == zone_id && cached.key == key)
    {
        return None;
    }

    match nav_grid.find_path_vec3(from, to) {
        Some(path) if path.len() > 1 => Some(path.into_iter().skip(1).collect()),
        _ => {
            match no_route {
                Some(cached) => {
                    cached.zone_id = zone_id;
                    cached.key = key;
                }
                None => {
                    commands.entity(entity).insert(BotNavNoRoute { zone_id, key });
                }
            }
            None
        }
    }
}

/// Route long travel orders through the nav grid. Runs in `PreUpdate` before
/// `command_system` so rewritten legs are what gets broadcast to clients.
#[allow(clippy::type_complexity)]
pub fn bot_navigation_system(
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &Position,
            &Command,
            &mut NextCommand,
            Option<&mut BotNavigation>,
            Option<&mut BotNavNoRoute>,
            Option<&mut BotSteerBroadcast>,
        ),
        Without<GameClient>,
    >,
    game_data: Res<GameData>,
    game_config: Res<GameConfig>,
) {
    if !game_config.enable_bot_navigation {
        return;
    }

    for (entity, position, command, mut next_command, nav, mut no_route, steer_broadcast) in
        query.iter_mut()
    {
        // command_system (next in the chain) broadcasts NextCommand this tick
        // when the flag is clear, replacing whatever the chase steering last
        // told clients about this entity.
        if !next_command.has_sent_server_message && next_command.command.is_some() {
            if let Some(mut steer_broadcast) = steer_broadcast {
                if steer_broadcast.sent.is_some() {
                    steer_broadcast.sent = None;
                }
            }
        }

        let destination = match next_command.command.as_ref() {
            Some(CommandData::Move {
                destination,
                target: None,
                ..
            }) => *destination,
            _ => {
                // Not a travel order: drop stale routes (e.g. bot switched to
                // Attack/Pickup, whose destinations track live targets).
                if nav.is_some() && !matches!(command.command, CommandData::Move { .. }) {
                    commands.entity(entity).remove::<BotNavigation>();
                }
                continue;
            }
        };

        let Some(nav_grid) = game_data
            .zones
            .get_zone(position.zone_id)
            .and_then(|zone| zone.nav_grid.as_ref())
        else {
            if nav.is_some() {
                commands.entity(entity).remove::<BotNavigation>();
            }
            continue;
        };

        // NextCommand holds either the leg this pass fed, the goal itself
        // (same order re-issued), or a new order that needs a new route.
        let following_route = nav.as_ref().is_some_and(|nav| {
            nav.leg
                .is_some_and(|leg| (leg - destination).length() <= NAV_GOAL_EPS_CM)
                || (nav.goal - destination).length() <= NAV_GOAL_EPS_CM
        });

        if !following_route {
            // Short hops go straight unless that crosses blocked ground
            // (a bank, a ledge, a wall right next to the bot).
            if position.position.xy().distance(destination.xy()) < NAV_ROUTE_MIN_DIST_CM
                && nav_grid.segment_walkable(
                    position.position.x,
                    position.position.y,
                    destination.x,
                    destination.y,
                )
            {
                if nav.is_some() {
                    commands.entity(entity).remove::<BotNavigation>();
                }
                continue;
            }

            let mut goal = destination;
            let mut route = find_route(
                &mut commands,
                entity,
                nav_grid,
                position.zone_id,
                no_route.as_deref_mut(),
                position.position,
                destination,
            );
            if route.is_none() {
                // Unreachable as ordered (across a moat, up a cliff, onto a
                // ledge): head for the nearest spot of it the bot can reach.
                if let Some(reachable) = nav_grid
                    .reachable_point(position.position, destination, NAV_RETARGET_CM)
                    .filter(|point| point.xy().distance(destination.xy()) > NAV_GOAL_EPS_CM)
                {
                    goal = reachable;
                    route = find_route(
                        &mut commands,
                        entity,
                        nav_grid,
                        position.zone_id,
                        no_route.as_deref_mut(),
                        position.position,
                        reachable,
                    );
                }
            }

            match route {
                Some(waypoints) => {
                    let first = waypoints[0];
                    let fed = split_leg(position.position, first);
                    let route = BotNavigation {
                        goal,
                        waypoints,
                        leg: Some(fed),
                        leg_waypoint: Some(first),
                    };
                    match nav {
                        Some(mut nav) => *nav = route,
                        None => {
                            commands.entity(entity).insert(route);
                        }
                    }
                    if let Some(CommandData::Move {
                        destination: dest, ..
                    }) = next_command.command.as_mut()
                    {
                        *dest = fed;
                    }
                    next_command.has_sent_server_message = false;
                }
                None => {
                    if nav.is_some() {
                        commands.entity(entity).remove::<BotNavigation>();
                    }
                    if nav_grid.segment_walkable(
                        position.position.x,
                        position.position.y,
                        goal.x,
                        goal.y,
                    ) {
                        // Close enough to walk straight on free ground.
                        if goal != destination {
                            if let Some(CommandData::Move {
                                destination: dest, ..
                            }) = next_command.command.as_mut()
                            {
                                *dest = goal;
                            }
                            next_command.has_sent_server_message = false;
                        }
                    } else {
                        // Nowhere reachable to go: stand still rather than
                        // walk through a wall, off a cliff or into deep water
                        // (a stranded bot is then put back by
                        // `bot_rescue_stranded_system`).
                        *next_command = NextCommand::with_stop(true);
                    }
                }
            }
            continue;
        }

        let Some(mut nav) = nav else { continue };

        // Arrived at the real goal: hand back the exact destination so the
        // arrival check (`distance < 0.1` in `command_system`) can stop.
        if position.position.xy().distance(nav.goal.xy()) < NAV_WAYPOINT_REACH_CM {
            if destination != nav.goal {
                if let Some(CommandData::Move {
                    destination: dest, ..
                }) = next_command.command.as_mut()
                {
                    *dest = nav.goal;
                }
                next_command.has_sent_server_message = false;
            }
            commands.entity(entity).remove::<BotNavigation>();
            continue;
        }

        while let Some(front) = nav.waypoints.front() {
            if position.position.xy().distance(front.xy()) < NAV_WAYPOINT_REACH_CM {
                nav.waypoints.pop_front();
            } else {
                break;
            }
        }

        // Route consumed short of the goal (snapped goal cell): keep walking
        // the last fed leg.
        let Some(&front) = nav.waypoints.front() else {
            continue;
        };

        // Keep feeding the current leg until it is reached; a clamped long
        // leg is re-split from the bot's position once its point is reached.
        let fed = match (nav.leg, nav.leg_waypoint) {
            (Some(leg), Some(leg_waypoint))
                if leg_waypoint == front
                    && position.position.xy().distance(leg.xy()) >= NAV_WAYPOINT_REACH_CM =>
            {
                leg
            }
            _ => split_leg(position.position, front),
        };
        if nav.leg != Some(fed) || nav.leg_waypoint != Some(front) {
            nav.leg = Some(fed);
            nav.leg_waypoint = Some(front);
        }

        // Only a changed leg is rewritten (and so re-broadcast by command_system).
        if destination != fed {
            if let Some(CommandData::Move {
                destination: dest, ..
            }) = next_command.command.as_mut()
            {
                *dest = fed;
            }
            next_command.has_sent_server_message = false;
        }
    }
}

/// Soft separation so travelling bots and monsters do not stack on one
/// position. Mirrors the client's `monster_separation_system` visually, but
/// runs server-side so positions stay authoritative. Only mobile entities
/// (active Move/Attack) are pushed; each push is capped per tick and written
/// back to the sector index like `update_position_system` does.
pub fn bot_separation_system(
    mut query: Query<
        (
            Entity,
            Option<&ClientEntity>,
            Option<&mut ClientEntitySector>,
            &mut Position,
            &Command,
        ),
        Without<GameClient>,
    >,
    mut client_entity_list: ResMut<ClientEntityList>,
    game_config: Res<GameConfig>,
    game_data: Res<GameData>,
    time: Res<Time>,
) {
    if !game_config.enable_bot_navigation {
        return;
    }

    const SEPARATION_RADIUS_CM: f32 = 100.0;
    const SEPARATION_MAX_CM_PER_SEC: f32 = 200.0;

    fn is_mobile(command: &Command) -> bool {
        matches!(
            command.command,
            CommandData::Move { .. } | CommandData::Attack { .. }
        )
    }

    // Snapshot of mobile entities, bucketed by zone in query order so every
    // entity sums its neighbours in the same order as a flat scan would.
    let mut snapshot: HashMap<ZoneId, Vec<(Entity, Vec3)>> = HashMap::new();
    let mut snapshot_len = 0usize;
    for (entity, _, _, position, command) in query.iter() {
        if is_mobile(command) {
            snapshot
                .entry(position.zone_id)
                .or_default()
                .push((entity, position.position));
            snapshot_len += 1;
        }
    }
    if snapshot_len < 2 {
        return;
    }

    let mut rng = rand::thread_rng();
    let max_step = SEPARATION_MAX_CM_PER_SEC * time.delta_secs();
    let jitter_rot = Vec2::from_angle(rng.gen_range(-0.035..0.035));

    for (entity, client_entity, client_entity_sector, mut position, command) in query.iter_mut() {
        // Commands are not changed here, so this is exactly snapshot membership.
        if !is_mobile(command) {
            continue;
        }
        let Some(zone_movers) = snapshot.get(&position.zone_id) else {
            continue;
        };
        let mut push = Vec3::ZERO;
        let mut count = 0u32;
        for (other_entity, other_pos) in zone_movers.iter() {
            if *other_entity == entity {
                continue;
            }
            let delta = position.position - *other_pos;
            let dist = delta.xy().length();
            if dist < SEPARATION_RADIUS_CM && dist > 0.001 {
                // Tiny deterministic rotation breaks perfect symmetry stacks.
                let dir = (delta.xy() / dist).rotate(jitter_rot);
                push += Vec3::new(dir.x, dir.y, 0.0) * (SEPARATION_RADIUS_CM - dist);
                count += 1;
            }
        }
        if count > 0 {
            let step = (push / count as f32).xy().clamp_length_max(max_step);
            let pushed = position.position.xy() + step;
            // Never shove anyone off walkable ground (into a moat, up a bank).
            let nav_grid = game_data
                .zones
                .get_zone(position.zone_id)
                .and_then(|zone| zone.nav_grid.as_ref());
            if nav_grid.is_some_and(|nav_grid| {
                nav_grid.is_blocked_world(pushed.x, pushed.y)
                    && !nav_grid.is_blocked_world(position.position.x, position.position.y)
            }) {
                continue;
            }
            position.position.x = pushed.x;
            position.position.y = pushed.y;

            if let (Some(client_entity), Some(mut sector)) =
                (client_entity, client_entity_sector)
            {
                if let Some(zone) = client_entity_list.get_zone_mut(position.zone_id) {
                    zone.update_position(entity, client_entity, &mut sector, position.position);
                }
            }
        }
    }
}

/// Wall-aware follower for live `Command::Move` orders: attack chase, pickup
/// and follow legs. Runs in `PreUpdate` AFTER `command_system`, which is what
/// sets these destinations every tick straight at the (possibly moving)
/// target — bypassing the `NextCommand` routing above entirely.
///
/// Division of labour with `bot_navigation_system` (which owns orders whose
/// `NextCommand` is an untargeted travel move): this pass skips those, and
/// handles everything else with an active `Command::Move`:
/// - straight step clear: keep command_system's destination;
/// - blocked: deflect one step (wall slide; the chaser recomputes from the
///   live target next tick, so sliding emerges naturally);
/// - surrounded: escalate to a full A* route stored in `BotNavigation`,
///   replanned when the target drifts from the stored goal.
/// Rewritten destinations are broadcast as `MoveEntity` (mirroring the
/// promotion broadcast in `command_system`) only when they differ from what
/// clients were last told, and once more when steering hands the move back
/// (as `AttackEntity` for an attack chase, see `steer_hand_back`).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn bot_chase_steering_system(
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &Position,
            &mut Command,
            &NextCommand,
            Option<&ClientEntity>,
            Option<&mut BotNavigation>,
            Option<&mut BotNavNoRoute>,
            Option<&mut BotSteerBroadcast>,
        ),
        Without<GameClient>,
    >,
    target_clients: Query<&ClientEntity>,
    attack_targets: Query<(&ClientEntity, &Position)>,
    game_data: Res<GameData>,
    game_config: Res<GameConfig>,
    mut server_messages: ResMut<ServerMessages>,
) {
    if !game_config.enable_bot_navigation {
        return;
    }

    for (
        entity,
        position,
        mut command,
        next_command,
        client_entity,
        mut nav,
        mut no_route,
        mut steer_broadcast,
    ) in query.iter_mut()
    {
        let (destination, target, move_mode) = match &command.command {
            CommandData::Move {
                destination,
                target,
                move_mode,
            } => (*destination, *target, *move_mode),
            _ => {
                // No live move: drop chase routes left over from one. The new
                // command was announced by whoever set it.
                if nav.is_some() {
                    commands.entity(entity).remove::<BotNavigation>();
                }
                if let Some(steer_broadcast) = steer_broadcast.as_mut() {
                    if steer_broadcast.sent.is_some() {
                        steer_broadcast.sent = None;
                    }
                }
                continue;
            }
        };

        // Travel orders with an untargeted NextCommand belong to the pre-pass.
        if matches!(
            next_command.command,
            Some(CommandData::Move {
                target: None,
                ..
            })
        ) {
            continue;
        }

        let Some(nav_grid) = game_data
            .zones
            .get_zone(position.zone_id)
            .and_then(|zone| zone.nav_grid.as_ref())
        else {
            continue;
        };

        let steer_move = SteerMove {
            client_entity,
            target,
            move_mode,
            from: position.position,
        };
        // The queued attack this move chases for, re-announced on hand-back.
        let queued_attack = match &next_command.command {
            Some(CommandData::Attack { target }) => Some(*target),
            _ => None,
        };

        // Advance an escalated chase route first.
        let mut drop_route = false;
        if let Some(nav) = nav.as_deref_mut() {
            if (nav.goal - destination).length() > CHASE_REPLAN_CM {
                // Target drifted from the routed goal: steer fresh below.
                drop_route = true;
            } else if position.position.xy().distance(nav.goal.xy()) < NAV_WAYPOINT_REACH_CM {
                // Reached the routed goal: resume live target tracking.
                commands.entity(entity).remove::<BotNavigation>();
                steer_hand_back(
                    steer_broadcast.as_deref_mut(),
                    steer_move,
                    destination,
                    queued_attack,
                    &target_clients,
                    &attack_targets,
                    &mut server_messages,
                );
                continue;
            } else {
                while let Some(front) = nav.waypoints.front() {
                    if position.position.xy().distance(front.xy()) < NAV_WAYPOINT_REACH_CM {
                        nav.waypoints.pop_front();
                    } else {
                        break;
                    }
                }
                if let Some(leg) = nav.waypoints.front().copied() {
                    steer_rewrite(
                        &mut commands,
                        entity,
                        &mut command,
                        steer_broadcast.as_deref_mut(),
                        steer_move,
                        leg,
                        &target_clients,
                        &mut server_messages,
                    );
                    continue;
                }
                // Route consumed short of the goal (snap offset): steer fresh
                // toward the live destination below.
                drop_route = true;
            }
        }
        if drop_route {
            commands.entity(entity).remove::<BotNavigation>();
        }

        match steer_around_blocker(
            nav_grid,
            position.position.x,
            position.position.y,
            destination.x,
            destination.y,
        ) {
            Steer::Clear => {
                steer_hand_back(
                    steer_broadcast.as_deref_mut(),
                    steer_move,
                    destination,
                    queued_attack,
                    &target_clients,
                    &attack_targets,
                    &mut server_messages,
                );
            }
            Steer::Deflect(sx, sy) => {
                steer_rewrite(
                    &mut commands,
                    entity,
                    &mut command,
                    steer_broadcast.as_deref_mut(),
                    steer_move,
                    Vec3::new(sx, sy, destination.z),
                    &target_clients,
                    &mut server_messages,
                );
            }
            Steer::Surrounded => {
                // Escalate to a full A* route toward the live destination.
                match find_route(
                    &mut commands,
                    entity,
                    nav_grid,
                    position.zone_id,
                    no_route.as_deref_mut(),
                    position.position,
                    destination,
                ) {
                    Some(waypoints) => {
                        let first = waypoints[0];
                        let fed = split_leg(position.position, first);
                        // Inserted after the removal above (commands apply in
                        // order), so a dropped route is replaced.
                        commands.entity(entity).insert(BotNavigation {
                            goal: destination,
                            waypoints,
                            leg: Some(fed),
                            leg_waypoint: Some(first),
                        });
                        steer_rewrite(
                            &mut commands,
                            entity,
                            &mut command,
                            steer_broadcast.as_deref_mut(),
                            steer_move,
                            fed,
                            &target_clients,
                            &mut server_messages,
                        );
                    }
                    None => {
                        steer_hand_back(
                            steer_broadcast.as_deref_mut(),
                            steer_move,
                            destination,
                            queued_attack,
                            &target_clients,
                            &attack_targets,
                            &mut server_messages,
                        );
                    }
                }
            }
        }
    }
}

/// What a chase-steering broadcast says about one entity's move.
#[derive(Clone, Copy)]
struct SteerMove<'a> {
    client_entity: Option<&'a ClientEntity>,
    target: Option<Entity>,
    move_mode: Option<MoveMode>,
    from: Vec3,
}

/// Rewrite the live `Command::Move` destination; broadcast it only when it
/// differs from what clients were last told for this entity.
#[allow(clippy::too_many_arguments)]
fn steer_rewrite(
    commands: &mut Commands,
    entity: Entity,
    command: &mut Command,
    steer_broadcast: Option<&mut BotSteerBroadcast>,
    steer_move: SteerMove,
    destination: Vec3,
    target_clients: &Query<&ClientEntity>,
    server_messages: &mut ServerMessages,
) {
    *command = Command::with_move(destination, steer_move.target, steer_move.move_mode);

    let sent = SteerSent {
        destination,
        target: steer_move.target,
        move_mode: steer_move.move_mode,
    };
    match steer_broadcast {
        Some(steer_broadcast) => {
            let already_sent = steer_broadcast.sent.is_some_and(|previous| {
                previous.target == sent.target
                    && previous.move_mode == sent.move_mode
                    && previous.destination.xy().distance(destination.xy())
                        <= STEER_REBROADCAST_CM
            });
            if !already_sent {
                broadcast_move(steer_move, destination, target_clients, server_messages);
                steer_broadcast.sent = Some(sent);
            }
        }
        None => {
            broadcast_move(steer_move, destination, target_clients, server_messages);
            commands
                .entity(entity)
                .insert(BotSteerBroadcast { sent: Some(sent) });
        }
    }
}

/// Steering leaves command_system's destination in place. If clients were
/// last told a steered destination, re-announce the live chase once.
///
/// An attack chase (queued `Attack`) is re-announced as `AttackEntity`, like
/// `command_system` announces it: clients then chase and attack locally until
/// the in-range `AttackEntity`. A targeted `MoveEntity` would instead make
/// them walk straight at the target until that message arrives.
#[allow(clippy::too_many_arguments)]
fn steer_hand_back(
    steer_broadcast: Option<&mut BotSteerBroadcast>,
    steer_move: SteerMove,
    destination: Vec3,
    queued_attack: Option<Entity>,
    target_clients: &Query<&ClientEntity>,
    attack_targets: &Query<(&ClientEntity, &Position)>,
    server_messages: &mut ServerMessages,
) {
    let Some(steer_broadcast) = steer_broadcast else {
        return;
    };
    if steer_broadcast.sent.take().is_none() {
        return;
    }

    let attack_target = queued_attack.and_then(|target| attack_targets.get(target).ok());
    match attack_target {
        Some((target_client, target_position)) => {
            broadcast_attack(steer_move, target_client, target_position, server_messages);
        }
        None => broadcast_move(steer_move, destination, target_clients, server_messages),
    }
}

/// `AttackEntity` exactly like the attack broadcast in `command_system`.
fn broadcast_attack(
    steer_move: SteerMove,
    target_client: &ClientEntity,
    target_position: &Position,
    server_messages: &mut ServerMessages,
) {
    let Some(client_entity) = steer_move.client_entity else {
        return;
    };
    let distance = steer_move.from.xy().distance(target_position.position.xy());
    server_messages.send_entity_message(
        client_entity,
        ServerMessage::AttackEntity {
            entity_id: client_entity.id,
            target_entity_id: target_client.id,
            distance: distance as u16,
            x: target_position.position.x,
            y: target_position.position.y,
            z: target_position.position.z as u16,
        },
    );
}

/// `MoveEntity` exactly like the promotion broadcast in `command_system`, so
/// the rendered path matches the server path.
fn broadcast_move(
    steer_move: SteerMove,
    destination: Vec3,
    target_clients: &Query<&ClientEntity>,
    server_messages: &mut ServerMessages,
) {
    let Some(client_entity) = steer_move.client_entity else {
        return;
    };
    let target_entity_id = steer_move
        .target
        .and_then(|target| target_clients.get(target).ok())
        .map(|target_client| target_client.id);
    let distance = steer_move.from.xy().distance(destination.xy());
    server_messages.send_entity_message(
        client_entity,
        ServerMessage::MoveEntity {
            entity_id: client_entity.id,
            target_entity_id,
            distance: distance as u16,
            x: destination.x,
            y: destination.y,
            z: destination.z as u16,
            move_mode: steer_move.move_mode,
        },
    );
}

/// How often bots are checked for being stranded.
const RESCUE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// How long a bot may stay stranded before it is put back.
const RESCUE_AFTER: Duration = Duration::from_secs(4);
/// The same while fighting: archers and mages may hit something from a bank.
const RESCUE_AFTER_IN_COMBAT: Duration = Duration::from_secs(12);
/// How far from a stranded bot walkable ground is searched.
const RESCUE_SEARCH_CM: f32 = 6000.0;

/// When the bot was first seen stranded (see `NavGrid::is_stranded`).
#[derive(Component)]
pub struct BotStranded {
    since: Instant,
}

/// Puts back bots that ended up where no one can walk: on blocked ground (in
/// a moat, on a cliff face, inside a wall) or in a free pocket that no
/// designer placed point leads to (a moat bottom, a ledge top). Every route
/// out of such a spot is refused, so the bot would otherwise stand there
/// forever. It is moved to the nearest walkable ground of an anchored area,
/// or the zone's start position, and rejoins the zone so every client
/// respawns it there.
#[allow(clippy::type_complexity)]
pub fn bot_rescue_stranded_system(
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &BotRole,
            &ClientEntity,
            &ClientEntitySector,
            &mut Position,
            &mut Command,
            &mut NextCommand,
            Option<&BotStranded>,
        ),
        (Without<GameClient>, Without<Dead>, Without<LlmBuddyBot>),
    >,
    mut client_entity_list: ResMut<ClientEntityList>,
    game_data: Res<GameData>,
    game_config: Res<GameConfig>,
    mut last_check: Local<Option<Instant>>,
) {
    if !game_config.enable_bot_navigation {
        return;
    }
    let now = Instant::now();
    if last_check.is_some_and(|last| now.duration_since(last) < RESCUE_CHECK_INTERVAL) {
        return;
    }
    *last_check = Some(now);

    for (
        entity,
        role,
        client_entity,
        client_entity_sector,
        mut position,
        mut command,
        mut next_command,
        stranded,
    ) in query.iter_mut()
    {
        // Vendors keep their stall wherever it was set up.
        if *role == BotRole::Vendor {
            continue;
        }
        let Some(zone_data) = game_data.zones.get_zone(position.zone_id) else {
            continue;
        };
        let Some(nav_grid) = zone_data.nav_grid.as_ref() else {
            continue;
        };
        if !nav_grid.is_stranded(position.position.x, position.position.y) {
            if stranded.is_some() {
                commands.entity(entity).remove::<BotStranded>();
            }
            continue;
        }
        let Some(stranded) = stranded else {
            commands.entity(entity).insert(BotStranded { since: now });
            continue;
        };
        let patience = if matches!(command.command, CommandData::Attack { .. }) {
            RESCUE_AFTER_IN_COMBAT
        } else {
            RESCUE_AFTER
        };
        if now.duration_since(stranded.since) < patience {
            continue;
        }

        let destination = nav_grid
            .rescue_point(position.position, RESCUE_SEARCH_CM)
            .or_else(|| nav_grid.rescue_point(zone_data.start_position, RESCUE_SEARCH_CM))
            .unwrap_or(zone_data.start_position);
        log::debug!(
            "Rescuing stranded bot {:?} in zone {:?}: ({:.0}, {:.0}) -> ({:.0}, {:.0})",
            entity,
            position.zone_id,
            position.position.x,
            position.position.y,
            destination.x,
            destination.y,
        );

        // Same placement as LLM buddy bot teleports: leave, move, rejoin
        // with a new client entity id so clients respawn the bot there.
        let previous_position = position.clone();
        client_entity_leave_zone(
            &mut commands,
            &mut client_entity_list,
            entity,
            client_entity,
            client_entity_sector,
            &previous_position,
        );
        position.position = destination;
        *command = Command::with_stop();
        *next_command = NextCommand::default();
        commands
            .entity(entity)
            .remove::<(BotStranded, BotNavigation, BotNavNoRoute)>();
        match client_entity_join_zone(
            &mut commands,
            &mut client_entity_list,
            entity,
            ClientEntityType::Character,
            &position,
        ) {
            Ok(_) => {
                commands
                    .entity(entity)
                    .insert(ClientEntityVisibility::new());
            }
            Err(error) => {
                log::error!(
                    "Failed to put stranded bot {:?} back into zone {:?}: {:?}",
                    entity,
                    position.zone_id,
                    error
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall_grid() -> rose_data::NavGrid {
        // 20x20 @100cm; vertical wall at col 10 except a gap at rows 9..=11.
        let mut grid = rose_data::NavGrid::new(0.0, 0.0, 100.0, 20, 20);
        for cy in 0..20 {
            if !(9..=11).contains(&cy) {
                grid.set_blocked_cell(10, cy);
            }
        }
        grid
    }

    #[test]
    fn open_step_needs_no_steering() {
        let grid = rose_data::NavGrid::new(0.0, 0.0, 100.0, 20, 20);
        assert_eq!(
            steer_around_blocker(&grid, 100.0, 100.0, 500.0, 100.0),
            Steer::Clear
        );
    }

    #[test]
    fn blocked_step_deflects_off_wall() {
        let grid = wall_grid();
        // 300cm probe from (750,550) lands inside the wall column.
        let Steer::Deflect(dx, dy) = steer_around_blocker(&grid, 750.0, 550.0, 1900.0, 550.0)
        else {
            panic!("expected a deflection");
        };
        // Same remaining distance, pushed off the wall row.
        let dist = (dx - 750.0).hypot(dy - 550.0);
        assert!((dist - 1150.0).abs() < 1.0);
        assert!((dy - 550.0).abs() > 50.0);
        // The probe step toward the deflected point is clear ground.
        let px = 750.0 + (dx - 750.0) / dist * STEER_PROBE_CM;
        let py = 550.0 + (dy - 550.0) / dist * STEER_PROBE_CM;
        assert!(!grid.is_blocked_world(px, py));
    }

    #[test]
    fn gap_step_needs_no_steering() {
        let grid = wall_grid();
        assert_eq!(
            steer_around_blocker(&grid, 100.0, 1050.0, 1900.0, 1050.0),
            Steer::Clear
        );
    }

    #[test]
    fn boxed_in_reports_surrounded() {
        // Free centre cell ringed by blocked cells: every probe hits a wall.
        let mut grid = rose_data::NavGrid::new(0.0, 0.0, 100.0, 20, 20);
        for cy in 0..20 {
            for cx in 0..20 {
                if !(cx == 10 && cy == 10) {
                    grid.set_blocked_cell(cx, cy);
                }
            }
        }
        assert_eq!(
            steer_around_blocker(&grid, 1050.0, 1050.0, 1900.0, 1050.0),
            Steer::Surrounded
        );
    }

    #[test]
    fn long_leg_is_clamped() {
        let from = Vec3::new(0.0, 0.0, 0.0);
        let to = Vec3::new(NAV_MAX_LEG_CM * 2.0, 0.0, 5.0);
        let leg = split_leg(from, to);
        assert!((leg.x - NAV_MAX_LEG_CM).abs() < 1.0);
        assert_eq!(leg.z, 5.0);
        assert_eq!(split_leg(from, Vec3::new(100.0, 0.0, 0.0)), Vec3::new(100.0, 0.0, 0.0));
    }
}
