use bevy::ecs::prelude::Component;

/// Server-side sailing state attached to entities while they are sailing
/// (`MoveMode::Sail`). The server validates client sail reports and relays the
/// accepted state to other clients via `SailState` broadcasts.
#[derive(Component)]
pub struct ServerBoatState {
    /// Water surface height in cm used to keep the boat at water level.
    pub water_height_cm: f32,
    /// Last accepted sailing position (cm).
    pub last_position: bevy::math::Vec3,
    /// Last reported heading (radians).
    pub heading: f32,
    /// Last reported speed (m/s).
    pub speed: f32,
    /// Last reported sail trim (radians, 0..PI).
    pub sail_trim: f32,
    /// Wall-clock time of the last accepted sail report.
    pub last_report_time: f32,
}

impl ServerBoatState {
    pub fn new(position: bevy::math::Vec3) -> Self {
        Self {
            water_height_cm: position.z,
            last_position: position,
            heading: 0.0,
            speed: 0.0,
            sail_trim: std::f32::consts::FRAC_PI_4,
            last_report_time: 0.0,
        }
    }
}
