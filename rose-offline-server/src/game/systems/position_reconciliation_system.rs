use bevy::{
    ecs::prelude::{Commands, Entity, Query, Res, ResMut, With, Without},
    time::Time,
};

use crate::game::{
    components::{ClientEntity, GameClient, Position},
    resources::ServerMessages,
};

/// Configuration for position reconciliation
#[derive(bevy::ecs::prelude::Resource)]
pub struct PositionReconciliationConfig {
    /// How often to send position snapshots (in seconds)
    pub snapshot_interval_secs: f32,
    /// Maximum allowed position divergence before forcing correction (in cm)
    pub max_divergence_cm: f32,
}

impl Default for PositionReconciliationConfig {
    fn default() -> Self {
        Self {
            snapshot_interval_secs: 0.5, // Send snapshots every 500ms
            max_divergence_cm: 100.0,    // 1 meter tolerance
        }
    }
}

/// Tracks time since last position snapshot for each entity
#[derive(bevy::ecs::prelude::Component)]
pub struct PositionSnapshotTimer {
    pub timer: f32,
}

impl Default for PositionSnapshotTimer {
    fn default() -> Self {
        Self { timer: 0.0 }
    }
}

/// Server-authoritative position reconciliation system.
///
/// This system periodically sends the server-authoritative position to clients
/// as a correction to handle any position divergence.
///
/// Reconciliation strategy:
/// 1. Periodically sample entity positions (every snapshot_interval_secs)
/// 2. Send the server-authoritative position to the client as an AdjustPosition correction
pub fn position_reconciliation_system(
    mut query: Query<(
        Entity,
        &ClientEntity,
        &Position,
        &mut PositionSnapshotTimer,
    )>,
    _server_messages: ResMut<ServerMessages>,
    time: Res<Time>,
    config: Res<PositionReconciliationConfig>,
) {
    let delta_secs = time.delta_secs();

    for (_entity, _client_entity, _position, mut snapshot_timer) in query.iter_mut() {
        // Update timer
        snapshot_timer.timer += delta_secs;

        // Skip if not time for a snapshot yet
        if snapshot_timer.timer < config.snapshot_interval_secs {
            continue;
        }

        // Reset timer
        snapshot_timer.timer = 0.0;

        // Intentionally do not emit periodic AdjustPosition packets here.
        // Unconditional corrections were being interpreted as gameplay movement
        // transitions on the client, causing visible stop/start movement jitter.
        // Immediate reconciliation is still handled through explicit validation
        // paths such as invalid MoveCollision reports.
        let _ = config.max_divergence_cm;
    }
}

/// System to add PositionSnapshotTimer to new GameClient entities
pub fn add_position_snapshot_timer_system(
    mut commands: Commands,
    query: Query<Entity, (With<GameClient>, Without<PositionSnapshotTimer>)>,
) {
    for entity in query.iter() {
        commands.entity(entity).insert(PositionSnapshotTimer::default());
    }
}
