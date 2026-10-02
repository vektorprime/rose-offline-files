use bevy::{
    math::Vec2,
    prelude::{Entity, Local, Query, Res, ResMut, With},
    time::Time,
};

use rose_data::ZoneId;
use rose_game_common::messages::server::ServerMessage;

use crate::game::{
    components::{ClientEntity, GameClient, MoveMode, Position, ServerBoatState},
    resources::{ServerMessages, ServerWindSettings, ServerWindState},
};

/// Ocean zone where sailing is available.
pub const OCEAN_ZONE_ID: u16 = 200;

/// Anti-cheat bound for boat speed (m/s).
pub const SAIL_MAX_SPEED_MS: f32 = 12.0;
/// Position delta treated as a teleport and rejected outright (cm).
const SAIL_TELEPORT_THRESHOLD_CM: f32 = 500.0;
/// Multiplier applied to expected movement for network jitter tolerance.
const SAIL_SPEED_TOLERANCE: f32 = 1.5;
/// Fixed extra allowance (cm) for collision stops and report jitter.
const SAIL_REPORT_JITTER_CM: f32 = 200.0;
/// How often authoritative sail state is broadcast (seconds).
const SAIL_STATE_BROADCAST_INTERVAL: f32 = 0.1;
/// How often wind state is broadcast to the ocean zone (seconds).
const WIND_BROADCAST_INTERVAL: f32 = 1.0;

/// Server-side wind simulation. Mirrors the client `wind_update_system`
/// algorithm exactly so client prediction matches the authoritative wind.
pub fn server_wind_update_system(
    time: Res<Time>,
    settings: Res<ServerWindSettings>,
    mut wind: ResMut<ServerWindState>,
) {
    wind.time_accumulator += time.delta_secs();
    let t = wind.time_accumulator;

    let base_angle = (t * settings.direction_drift_speed).sin() * 0.5
        + (t * settings.direction_drift_speed * 0.37).sin() * 0.3
        + (t * settings.direction_drift_speed * 0.13).sin() * 0.2;

    let gust = ((t * settings.gust_frequency * std::f32::consts::TAU).sin() * 0.5 + 0.5).powf(3.0);
    let gust_angle_offset = gust * settings.gust_direction_variance * (t * 2.3).sin();

    wind.angle = base_angle + gust_angle_offset;
    wind.speed = settings.base_speed * (1.0 + gust * (settings.gust_max_multiplier - 1.0));
    wind.gust_factor = gust;
    wind.direction = Vec2::new(wind.angle.sin(), wind.angle.cos()) * wind.speed;
}

/// Broadcasts the authoritative wind to the ocean zone at 1 Hz.
pub fn wind_broadcast_system(
    time: Res<Time>,
    wind: Res<ServerWindState>,
    mut server_messages: ResMut<ServerMessages>,
    mut accumulator: Local<f32>,
) {
    *accumulator += time.delta_secs();
    if *accumulator < WIND_BROADCAST_INTERVAL {
        return;
    }
    *accumulator = 0.0;

    server_messages.send_zone_message(
        ZoneId::new(OCEAN_ZONE_ID).unwrap(),
        ServerMessage::WindStateUpdate {
            angle: wind.angle,
            speed: wind.speed,
            gust_factor: wind.gust_factor,
        },
    );
}

/// Broadcasts authoritative sail state for every sailing entity to its zone
/// at 10 Hz. The state is the last validated client report (position, heading,
/// speed, trim), so remote clients render the boat exactly as the sailing
/// player sees it.
pub fn sail_state_broadcast_system(
    time: Res<Time>,
    mut server_messages: ResMut<ServerMessages>,
    query: Query<(
        Entity,
        &ClientEntity,
        &Position,
        &ServerBoatState,
        &MoveMode,
    )>,
    mut accumulator: Local<f32>,
) {
    *accumulator += time.delta_secs();
    if *accumulator < SAIL_STATE_BROADCAST_INTERVAL {
        return;
    }
    *accumulator = 0.0;

    for (_entity, client_entity, position, boat, move_mode) in query.iter() {
        if *move_mode != MoveMode::Sail {
            continue;
        }

        server_messages.send_zone_message(
            position.zone_id,
            ServerMessage::SailState {
                entity_id: client_entity.id,
                position: position.position,
                heading: boat.heading,
                speed: boat.speed,
                sail_trim: boat.sail_trim,
            },
        );
    }
}

/// Maximum horizontal distance (cm) a sailing client may report per second.
pub fn sail_max_allowed_distance_cm(elapsed_secs: f32) -> f32 {
    SAIL_MAX_SPEED_MS * elapsed_secs.max(0.05).min(1.0) * SAIL_SPEED_TOLERANCE * 100.0
        + SAIL_REPORT_JITTER_CM
}

/// Teleport rejection threshold for sail reports (cm).
pub fn sail_teleport_threshold_cm() -> f32 {
    SAIL_TELEPORT_THRESHOLD_CM
}
