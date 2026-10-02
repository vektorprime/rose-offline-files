use std::{
    sync::Arc,
    thread::{self, ScopedJoinHandle},
};

use rose_data::{CharacterMotionDatabaseOptions, NpcDatabaseOptions};
use rose_data_irose::{
    get_ai_database, get_character_motion_database, get_data_decoder, get_item_database,
    get_job_class_database, get_npc_database, get_quest_database, get_skill_database,
    get_status_effect_database, get_string_database, get_warp_gate_database, get_zone_database,
};
use rose_file_readers::VirtualFilesystem;
use rose_game_irose::data::{get_ability_value_calculator, get_drop_table};

use crate::game::GameData;

mod character_creator;
use character_creator::get_character_creator;

/// Waits for a loader thread, re-raising its panic on this thread.
fn join<T>(handle: ScopedJoinHandle<'_, T>) -> T {
    handle
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

pub fn get_game_data(vfs: &VirtualFilesystem) -> GameData {
    let string_database = get_string_database(vfs, 1).expect("Failed to load string database");

    // These databases only read the VFS and the string database, so they load
    // concurrently. Results are unwrapped in the serial load's order with its
    // messages, so the first failure reported is the same.
    thread::scope(|scope| {
        let item_database = scope.spawn(|| get_item_database(vfs, string_database.clone()));
        let npc_database = scope.spawn(|| {
            get_npc_database(
                vfs,
                string_database.clone(),
                &NpcDatabaseOptions {
                    load_frame_data: true,
                },
            )
        });
        let job_class_database =
            scope.spawn(|| get_job_class_database(vfs, string_database.clone()));
        let skill_database = scope.spawn(|| get_skill_database(vfs, string_database.clone()));
        let zone_database = scope.spawn(|| get_zone_database(vfs, string_database.clone()));
        let ai_database = scope.spawn(|| get_ai_database(vfs));
        let motion_database = scope.spawn(|| {
            get_character_motion_database(
                vfs,
                &CharacterMotionDatabaseOptions {
                    load_frame_data: true,
                },
            )
        });
        let quest_database = scope.spawn(|| get_quest_database(vfs, string_database.clone()));
        let status_effect_database =
            scope.spawn(|| get_status_effect_database(vfs, string_database.clone()));
        let warp_gate_database = scope.spawn(|| get_warp_gate_database(vfs));

        let item_database = Arc::new(join(item_database).expect("Failed to load item database"));
        let npc_database = Arc::new(join(npc_database).expect("Failed to load npc database"));
        let job_class_database =
            Arc::new(join(job_class_database).expect("Failed to load job class database"));
        let skill_database = Arc::new(join(skill_database).expect("Failed to load skill database"));
        let zone_database = Arc::new(join(zone_database).expect("Failed to load zone database"));
        let drop_table = get_drop_table(vfs, item_database.clone(), npc_database.clone())
            .expect("Failed to load drop table");

        GameData {
            character_creator: get_character_creator(
                vfs,
                item_database.clone(),
                skill_database.clone(),
                &zone_database,
            )
            .expect("Failed to get character creator"),
            ability_value_calculator: get_ability_value_calculator(
                item_database.clone(),
                skill_database.clone(),
                npc_database.clone(),
            ),
            data_decoder: get_data_decoder(),
            drop_table,
            ai: Arc::new(join(ai_database).expect("Failed to load AI database")),
            items: item_database,
            job_class: job_class_database,
            motions: Arc::new(join(motion_database).expect("Failed to load motion database")),
            npcs: npc_database,
            quests: Arc::new(join(quest_database).expect("Failed to load quest database")),
            skills: skill_database,
            status_effects: Arc::new(
                join(status_effect_database).expect("Failed to load status effect database"),
            ),
            string_database: string_database.clone(),
            warp_gates: Arc::new(
                join(warp_gate_database).expect("Failed to load warp gate database"),
            ),
            zones: zone_database,
        }
    })
}
