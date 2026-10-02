use anyhow::Context;
use serde::{de::IgnoredAny, Deserialize, Serialize};
use std::{io::Write, path::PathBuf};

use rose_game_common::components::CharacterGender;

use crate::game::{
    components::{
        BasicStats, CharacterDeleteTime, CharacterInfo, Equipment, ExperiencePoints, HealthPoints,
        Hotbar, Inventory, Level, ManaPoints, Position, QuestState, SkillList, SkillPoints,
        Stamina, StatPoints, UnionMembership,
    },
    storage::CHARACTER_STORAGE_DIR,
};

#[derive(Deserialize, Serialize)]
pub struct CharacterStorage {
    pub info: CharacterInfo,
    pub basic_stats: BasicStats,
    pub inventory: Inventory,
    pub equipment: Equipment,
    pub level: Level,
    pub experience_points: ExperiencePoints,
    pub position: Position,
    pub skill_list: SkillList,
    pub hotbar: Hotbar,
    pub delete_time: Option<CharacterDeleteTime>,
    pub health_points: HealthPoints,
    pub mana_points: ManaPoints,
    pub skill_points: SkillPoints,
    pub stat_points: StatPoints,
    pub quest_state: QuestState,
    pub union_membership: UnionMembership,
    pub stamina: Stamina,
}

/// Borrowed view of a character which serialises to exactly the same JSON as
/// [`CharacterStorage`], so a live character can be saved without cloning its components.
///
/// Field names, field order and serde attributes must stay identical to [`CharacterStorage`].
#[derive(Serialize)]
pub struct CharacterStorageRef<'a> {
    pub info: &'a CharacterInfo,
    pub basic_stats: &'a BasicStats,
    pub inventory: &'a Inventory,
    pub equipment: &'a Equipment,
    pub level: &'a Level,
    pub experience_points: &'a ExperiencePoints,
    pub position: &'a Position,
    pub skill_list: &'a SkillList,
    pub hotbar: &'a Hotbar,
    pub delete_time: Option<&'a CharacterDeleteTime>,
    pub health_points: &'a HealthPoints,
    pub mana_points: &'a ManaPoints,
    pub skill_points: &'a SkillPoints,
    pub stat_points: &'a StatPoints,
    pub quest_state: &'a QuestState,
    pub union_membership: &'a UnionMembership,
    pub stamina: &'a Stamina,
}

impl CharacterStorageRef<'_> {
    pub fn save(&self) -> Result<(), anyhow::Error> {
        save_character_json(self, &self.info.name, true)
    }
}

/// The parts of a [`CharacterStorage`] file needed to list a clan member.
///
/// Every other field which `CharacterStorage` requires must still be present, but its value is
/// only checked to be valid JSON instead of being fully deserialised.
#[allow(dead_code)]
#[derive(Deserialize)]
struct CharacterLevelAndJobStorage {
    info: CharacterInfo,
    basic_stats: IgnoredAny,
    inventory: IgnoredAny,
    equipment: IgnoredAny,
    level: Level,
    experience_points: IgnoredAny,
    position: IgnoredAny,
    skill_list: IgnoredAny,
    hotbar: IgnoredAny,
    health_points: IgnoredAny,
    mana_points: IgnoredAny,
    skill_points: IgnoredAny,
    stat_points: IgnoredAny,
    quest_state: IgnoredAny,
    union_membership: IgnoredAny,
    stamina: IgnoredAny,
}

fn get_character_path(name: &str) -> PathBuf {
    CHARACTER_STORAGE_DIR.join(format!("{}.json", name))
}

fn save_character_json<T: Serialize>(
    character: &T,
    character_name: &str,
    allow_overwrite: bool,
) -> Result<(), anyhow::Error> {
    let path = get_character_path(character_name);
    let storage_dir = path.parent().unwrap();

    std::fs::create_dir_all(storage_dir).with_context(|| {
        format!(
            "Failed to create character storage directory {}",
            storage_dir.to_string_lossy()
        )
    })?;

    let json = serde_json::to_string_pretty(character).with_context(|| {
        format!(
            "Failed to serialise CharacterStorage whilst saving character {}",
            character_name
        )
    })?;

    let mut file = tempfile::Builder::new()
        .tempfile_in(storage_dir)
        .with_context(|| {
            format!(
                "Failed to create temporary file whilst saving character {}",
                character_name
            )
        })?;
    file.write_all(json.as_bytes()).with_context(|| {
        format!(
            "Failed to write data to temporary file whilst saving character {}",
            character_name
        )
    })?;

    if allow_overwrite {
        file.persist(&path).with_context(|| {
            format!(
                "Failed to persist temporary character file to path {}",
                path.to_string_lossy()
            )
        })?;
    } else {
        file.persist_noclobber(&path).with_context(|| {
            format!(
                "Failed to persist_noclobber character file to path {}",
                path.to_string_lossy()
            )
        })?;
    }

    Ok(())
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
pub enum CharacterCreatorError {
    InvalidName,
    InvalidGender,
    InvalidBirthStone,
    InvalidFace,
    InvalidHair,
}

pub trait CharacterCreator {
    fn create(
        &self,
        name: String,
        gender: CharacterGender,
        birth_stone: u8,
        face: u8,
        hair: u8,
    ) -> Result<CharacterStorage, CharacterCreatorError>;

    fn get_basic_stats(&self, gender: CharacterGender)
        -> Result<BasicStats, CharacterCreatorError>;
}

impl CharacterStorage {
    pub fn try_create(&self, character_name: &str) -> Result<(), anyhow::Error> {
        save_character_json(self, character_name, false)
    }

    pub fn try_load(name: &str) -> Result<Self, anyhow::Error> {
        let path = get_character_path(name);
        let str = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read file {}", path.to_string_lossy()))?;
        let character: CharacterStorage = serde_json::from_str(&str).with_context(|| {
            format!(
                "Failed to deserialise CharacterStorage from file {}",
                path.to_string_lossy()
            )
        })?;
        Ok(character)
    }

    /// Load only the level and job of a character, without building its inventory, quests etc.
    pub fn try_load_level_and_job(name: &str) -> Result<(Level, u16), anyhow::Error> {
        let path = get_character_path(name);
        let str = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read file {}", path.to_string_lossy()))?;
        let character: CharacterLevelAndJobStorage =
            serde_json::from_str(&str).with_context(|| {
                format!(
                    "Failed to deserialise CharacterStorage from file {}",
                    path.to_string_lossy()
                )
            })?;
        Ok((character.level, character.info.job))
    }

    pub fn save(&self) -> Result<(), anyhow::Error> {
        save_character_json(self, &self.info.name, true)
    }

    pub fn exists(name: &str) -> bool {
        get_character_path(name).exists()
    }

    pub fn delete(name: &str) -> Result<(), anyhow::Error> {
        let path = get_character_path(name);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
