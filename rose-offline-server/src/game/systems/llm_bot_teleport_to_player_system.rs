//! System to teleport LLM buddy bots to their assigned player when the player logs in
//!
//! When the server starts, bots are restored before players log in. This system
//! detects when a player joins a zone and teleports their assigned bots to them.

use bevy::ecs::prelude::*;
use bevy::math::Vec3;

use crate::game::{
    bundles::client_entity_teleport_zone,
    components::{
        CharacterInfo, ClientEntity, ClientEntitySector, ClientEntityVisibility, Command,
        CommandData, LlmBuddyBot, NextCommand, Position,
    },
    resources::ClientEntityList,
    systems::LlmBotManagerResource,
};

use rose_game_common::components::MoveMode;

/// System that teleports LLM buddy bots to their assigned player when the player logs in
///
/// This system detects when a player joins a zone (ClientEntityVisibility is added)
/// and teleports all bots assigned to that player to the player's current position.
///
/// IMPORTANT: This system properly updates the ClientEntityList sector registration
/// to ensure bots remain visible to clients after teleportation.
pub fn llm_bot_teleport_to_player_on_login_system(
    mut commands: Commands,
    // Query for players who just became visible (logged in/joined zone)
    players_joining: Query<
        (&Position, &CharacterInfo, &ClientEntity),
        (Added<ClientEntityVisibility>, Without<LlmBuddyBot>),
    >,
    // Query for all bots - includes ClientEntity and ClientEntitySector for proper teleport
    mut bot_query: Query<
        (
            &LlmBuddyBot,
            &mut Position,
            &mut NextCommand,
            &mut Command,
            &ClientEntity,
            &ClientEntitySector,
            Entity,
        ),
        Without<CharacterInfo>,
    >,
    bot_manager: Res<LlmBotManagerResource>,
    mut client_entity_list: ResMut<ClientEntityList>,
) {
    // Skip if no bots are registered
    let bots_map = bot_manager.bots_map.read();
    if bots_map.is_empty() {
        return;
    }

    // For each player that just joined
    for (player_position, character_info, _client_entity) in players_joining.iter() {
        let player_name = &character_info.name;
        let player_pos = player_position.position;
        let player_zone = player_position.zone_id;

        log::info!(
            "Player '{}' joined zone {:?}, checking for assigned bots to teleport",
            player_name,
            player_zone
        );

        // Find all bots assigned to this player
        for (_bot_id, bot_info) in bots_map.iter() {
            // Check if this bot is assigned to the joining player
            if let Some(ref assigned_player) = bot_info.assigned_player {
                if assigned_player == player_name {
                    let entity = bot_info.entity;

                    // Get the bot's components
                    if let Ok((
                        _buddy_bot,
                        mut bot_position,
                        mut next_command,
                        mut command,
                        bot_client_entity,
                        bot_client_entity_sector,
                        _bot_entity,
                    )) = bot_query.get_mut(entity)
                    {
                        // Store the previous position for teleport function
                        let previous_position = bot_position.clone();

                        // Update the bot's assigned player ID
                        // Note: We can't mutate buddy_bot here, but the follow system will
                        // use the player name to find the position

                        // Create new position for the bot
                        let new_position = Position::new(player_pos, player_zone);

                        // Use client_entity_teleport_zone to properly update ClientEntityList
                        // This ensures the bot's sector registration is updated correctly
                        client_entity_teleport_zone(
                            &mut commands,
                            &mut client_entity_list,
                            entity,
                            bot_client_entity,
                            bot_client_entity_sector,
                            &previous_position,
                            new_position,
                            None, // No GameClient for bots
                        );

                        // Also update the Position component directly for the bot's local state
                        bot_position.position = player_pos;
                        bot_position.zone_id = player_zone;

                        // Set a move command to ensure proper synchronization
                        // Use a position slightly offset from the player to avoid collision
                        let offset = Vec3::new(50.0, 0.0, 50.0);
                        let teleport_dest = player_pos + offset;

                        // Clear current command and set a stop command
                        *command = Command::with_stop();
                        next_command.command = Some(CommandData::Move {
                            destination: teleport_dest,
                            target: None,
                            move_mode: Some(MoveMode::Run),
                        });
                        next_command.has_sent_server_message = false;

                        log::info!(
                            "Teleported LLM buddy bot '{}' to player '{}' at position {:?} in zone {:?} (with ClientEntityList update)",
                            bot_info.name,
                            player_name,
                            player_pos,
                            player_zone
                        );
                    } else {
                        log::warn!(
                            "Could not find bot entity {:?} for bot '{}' assigned to player '{}'",
                            entity,
                            bot_info.name,
                            player_name
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_exists() {
        // This test just verifies the system compiles
        // The actual functionality is tested via integration tests
    }
}
