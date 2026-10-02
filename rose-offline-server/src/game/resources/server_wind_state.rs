use bevy::math::Vec2;
use bevy::prelude::Resource;

/// Server-authoritative wind state. The same layered sine + gust algorithm
/// used by the client's `wind_update_system` runs server-side so that all
/// clients converge on the same wind when it is broadcast.
#[derive(Resource)]
pub struct ServerWindState {
    /// Wind direction in Position-space XY plane, scaled by speed.
    pub direction: Vec2,
    /// Wind speed in m/s.
    pub speed: f32,
    /// Wind angle in radians. 0 = +Y in Position space.
    pub angle: f32,
    /// 0..1 gust intensity.
    pub gust_factor: f32,
    /// Simulation time accumulator.
    pub time_accumulator: f32,
}

impl Default for ServerWindState {
    fn default() -> Self {
        let speed: f32 = 5.0;
        Self {
            direction: Vec2::new(0.0, speed),
            speed,
            angle: 0.0,
            gust_factor: 0.0,
            time_accumulator: 0.0,
        }
    }
}

/// Wind simulation tuning values (mirrors the client `WindSettings` defaults
/// so both sides produce identical wind when no authoritative wind is used).
#[derive(Resource)]
pub struct ServerWindSettings {
    pub base_speed: f32,
    pub direction_drift_speed: f32,
    pub gust_frequency: f32,
    pub gust_max_multiplier: f32,
    pub gust_direction_variance: f32,
}

impl Default for ServerWindSettings {
    fn default() -> Self {
        Self {
            base_speed: 5.0,
            direction_drift_speed: 0.05,
            gust_frequency: 0.1,
            gust_max_multiplier: 1.5,
            gust_direction_variance: 0.3,
        }
    }
}
