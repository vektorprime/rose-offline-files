use bevy::prelude::{Entity, Message};

/// Written when a client entity starts an emote motion, so nearby bots can
/// react to players emoting.
#[derive(Message, Debug)]
pub struct PlayerEmoteEvent {
    pub entity: Entity,
    pub motion_id: u16,
}
