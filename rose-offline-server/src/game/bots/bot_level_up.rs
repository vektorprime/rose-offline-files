//! Level ups: bots celebrate, advance their job and upgrade their gear.
//!
//! Bots gain XP like players (shared kills and party XP go through the normal
//! reward systems), and `bot_auto_progression_system` spends the new points.
//! This system covers what a watching player notices: a cheer emote and a
//! "ding" line (nearby bots answer with congratulations through the normal
//! chat replies), the first/second job advancement a player would do at the
//! same levels as `bot_create_with_build`, and new armour/weapons once the bot
//! can wear better ones, announced to nearby clients like any equip change.

use bevy::prelude::{Changed, Commands, Entity, Query, Res, ResMut, With, Without};

use crate::game::{
    components::{
        CharacterInfo, ClientEntity, Command, CommandData, Dead, Equipment, Level, LlmBuddyBot,
    },
    messages::server::ServerMessage,
    resources::{GameData, ServerMessages},
};

use super::{
    bot_chat::{BotChatEvent, BotChatter},
    bot_emotes::{bot_play_emote_kind, BotEmoteKind, BotEmotes},
    bot_personality::BotLife,
    create_bot::bot_equipment_upgrades,
    BotBuild, VendorBot,
};

#[allow(clippy::type_complexity)]
pub fn bot_level_up_system(
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &ClientEntity,
            &Level,
            &Command,
            &BotBuild,
            &mut CharacterInfo,
            &mut Equipment,
            &mut BotLife,
            &mut BotChatter,
        ),
        (
            Changed<Level>,
            With<BotBuild>,
            Without<LlmBuddyBot>,
            Without<VendorBot>,
            Without<Dead>,
        ),
    >,
    game_data: Res<GameData>,
    emotes: Res<BotEmotes>,
    mut server_messages: ResMut<ServerMessages>,
) {
    let mut rng = rand::thread_rng();

    for (
        entity,
        client_entity,
        level,
        command,
        bot_build,
        mut character_info,
        mut equipment,
        mut life,
        mut chatter,
    ) in query.iter_mut()
    {
        let new_level = level.level;
        if new_level <= life.last_level {
            // Spawn, or a level set down by a GM command.
            if new_level != life.last_level {
                life.last_level = new_level;
            }
            continue;
        }
        life.last_level = new_level;

        // Job advancement at the same levels new bots get their job.
        let build_job = bot_build.job_id.get();
        let first_job = (build_job / 100) * 100 + 11;
        let job = character_info.job;
        if new_level >= 70 && job == first_job && build_job != first_job {
            character_info.job = build_job;
        } else if new_level >= 10 && job == 0 {
            character_info.job = first_job;
        }

        for (equipment_index, item) in
            bot_equipment_upgrades(&game_data, bot_build, &equipment, new_level)
        {
            equipment.equipped_items[equipment_index] = Some(item.clone());
            server_messages.send_entity_message(
                client_entity,
                ServerMessage::UpdateEquipment {
                    entity_id: client_entity.id,
                    equipment_index,
                    item: Some(item),
                },
            );
        }

        // Celebrate unless in the middle of something.
        if matches!(
            command.command,
            CommandData::Stop { .. } | CommandData::Move { target: None, .. }
        ) {
            bot_play_emote_kind(
                &mut commands,
                &mut server_messages,
                &emotes,
                &mut rng,
                entity,
                client_entity,
                BotEmoteKind::Cheer,
            );
        }
        chatter.notify(&mut rng, BotChatEvent::LevelUp(new_level));
    }
}
