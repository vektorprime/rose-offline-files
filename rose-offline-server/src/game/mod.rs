mod bots;
mod bundles;
mod events;
mod game_world;
mod resources;
mod systems;

pub mod api;
pub mod components;
pub mod messages;
pub mod storage;

#[cfg(feature = "llm-feedback")]
pub mod llm;

pub use game_world::GameWorld;
pub use resources::{GameConfig, GameData};
