//! Persistent storage for LLM Buddy Bots
//!
//! This module provides persistence for LLM buddy bots so they can be recovered
//! after game server restarts. Bot state is saved to JSON files and loaded on startup.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
};
use uuid::Uuid;

use super::LLM_BUDDY_BOT_STORAGE_DIR;

/// Stored bots as (listed name, bot), in storage directory listing order followed by bots first
/// written after the index was loaded.
type BotIndex = Vec<(String, LlmBuddyBotStorage)>;

/// In-memory copy of the bot storage directory, loaded on first use and kept in sync by every
/// write and delete made through this module, so player zone joins do not re-read every bot file
/// on the ECS thread. Bot files changed by anything else while the server runs are not seen.
static BOT_INDEX: Mutex<Option<BotIndex>> = Mutex::new(None);

fn lock_bot_index() -> MutexGuard<'static, Option<BotIndex>> {
    BOT_INDEX.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The name under which `list_all_bot_names` reports the file at `path`, if it reports it at all.
fn listed_bot_name(path: &Path) -> Option<&str> {
    if path.parent() != Some(LLM_BUDDY_BOT_STORAGE_DIR.as_path())
        || !path.extension().map_or(false, |ext| ext == "json")
    {
        return None;
    }
    path.file_stem().and_then(|stem| stem.to_str())
}

/// Persistent storage for an LLM Buddy Bot
///
/// This struct contains all the data needed to recreate an LLM buddy bot
/// after a server restart. It stores the bot's identity, configuration,
/// and current state.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmBuddyBotStorage {
    /// Unique bot ID (UUID)
    pub bot_id: Uuid,
    /// Bot character name
    pub name: String,
    /// Bot level
    pub level: u16,
    /// Build type (knight, mage, cleric, etc.)
    pub build_type: String,
    /// Name of the player this bot is assigned to assist
    pub assigned_player: String,
    /// Entity ID of the assigned player (may change between sessions)
    #[serde(default)]
    pub assigned_player_entity_id: u32,
    /// Current zone ID
    pub zone_id: u16,
    /// Current position
    pub position: PositionData,
    /// Follow distance
    #[serde(default = "default_follow_distance")]
    pub follow_distance: f32,
    /// Whether the bot is currently following
    #[serde(default)]
    pub is_following: bool,
    /// Current health points
    pub health: VitalPoints,
    /// Current mana points
    pub mana: VitalPoints,
    /// Current stamina
    #[serde(default = "default_stamina")]
    pub stamina: u32,
    /// LLM conversation memory/context (optional)
    #[serde(default)]
    pub memory: Option<String>,
    /// Timestamp of last save
    #[serde(default)]
    pub last_saved: Option<String>,
}

/// Position data for storage
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PositionData {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Default for PositionData {
    fn default() -> Self {
        Self {
            x: 520000.0,
            y: 520000.0,
            z: 0.0,
        }
    }
}

/// Vital points (current/max)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VitalPoints {
    pub current: i32,
    pub max: i32,
}

impl Default for VitalPoints {
    fn default() -> Self {
        Self { current: 100, max: 100 }
    }
}

fn default_follow_distance() -> f32 {
    50.0  // Reduced from 300.0 to 50.0 for closer following
}

fn default_stamina() -> u32 {
    10000
}

fn get_bot_storage_path(name: &str) -> PathBuf {
    LLM_BUDDY_BOT_STORAGE_DIR.join(format!("{}.json", name))
}

impl LlmBuddyBotStorage {
    /// Create a new bot storage with default values
    pub fn new(
        bot_id: Uuid,
        name: String,
        level: u16,
        build_type: String,
        assigned_player: String,
        zone_id: u16,
        position: PositionData,
    ) -> Self {
        Self {
            bot_id,
            name,
            level,
            build_type,
            assigned_player,
            assigned_player_entity_id: 0,
            zone_id,
            position,
            follow_distance: default_follow_distance(),
            is_following: false,
            health: VitalPoints::default(),
            mana: VitalPoints::default(),
            stamina: default_stamina(),
            memory: None,
            last_saved: None,
        }
    }

    /// Try to create a new bot storage file (fails if already exists)
    pub fn try_create(&self) -> Result<(), anyhow::Error> {
        self.save_impl(false)
    }

    /// Try to load a bot storage by name
    pub fn try_load(name: &str) -> Result<Self, anyhow::Error> {
        let path = get_bot_storage_path(name);
        let str = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read bot file {}", path.to_string_lossy()))?;
        let bot: LlmBuddyBotStorage = serde_json::from_str(&str).with_context(|| {
            format!(
                "Failed to deserialize LlmBuddyBotStorage from file {}",
                path.to_string_lossy()
            )
        })?;
        Ok(bot)
    }

    /// Try to load a bot storage by bot_id
    pub fn try_load_by_id(bot_id: &Uuid) -> Result<Self, anyhow::Error> {
        // List all bots and find by ID
        for bot_name in Self::list_all_bot_names()? {
            if let Ok(bot) = Self::try_load(&bot_name) {
                if bot.bot_id == *bot_id {
                    return Ok(bot);
                }
            }
        }
        anyhow::bail!("Bot with ID {} not found", bot_id)
    }

    /// Save the bot storage (overwrites if exists)
    pub fn save(&self) -> Result<(), anyhow::Error> {
        self.save_impl(true)
    }

    fn save_impl(&self, allow_overwrite: bool) -> Result<(), anyhow::Error> {
        let path = get_bot_storage_path(&self.name);
        let storage_dir = path.parent().unwrap();

        std::fs::create_dir_all(storage_dir).with_context(|| {
            format!(
                "Failed to create LLM buddy bot storage directory {}",
                storage_dir.to_string_lossy()
            )
        })?;

        // Update last_saved timestamp
        let mut bot_to_save = self.clone();
        bot_to_save.last_saved = Some(chrono::Utc::now().to_rfc3339());

        let json = serde_json::to_string_pretty(&bot_to_save).with_context(|| {
            format!(
                "Failed to serialize LlmBuddyBotStorage whilst saving bot {}",
                self.name
            )
        })?;

        let mut file = tempfile::Builder::new()
            .tempfile_in(storage_dir)
            .with_context(|| {
                format!(
                    "Failed to create temporary file whilst saving bot {}",
                    self.name
                )
            })?;
        file.write_all(json.as_bytes()).with_context(|| {
            format!(
                "Failed to write data to temporary file whilst saving bot {}",
                self.name
            )
        })?;

        // Held across the rename so the index and the directory always change together.
        let mut bot_index = lock_bot_index();

        if allow_overwrite {
            file.persist(&path).with_context(|| {
                format!(
                    "Failed to persist temporary bot file to path {}",
                    path.to_string_lossy()
                )
            })?;
        } else {
            file.persist_noclobber(&path).with_context(|| {
                format!(
                    "Failed to persist_noclobber bot file {}",
                    path.to_string_lossy()
                )
            })?;
        }

        // An index that is not loaded yet will read the new file when it is loaded.
        if let (Some(bots), Some(listed_name)) = (bot_index.as_mut(), listed_bot_name(&path)) {
            if let Some((_, indexed_bot)) = bots
                .iter_mut()
                .find(|(indexed_name, _)| indexed_name == listed_name)
            {
                *indexed_bot = bot_to_save;
            } else {
                bots.push((listed_name.to_string(), bot_to_save));
            }
        }
        drop(bot_index);

        log::info!("Saved LLM buddy bot '{}' ({})", self.name, self.bot_id);
        Ok(())
    }

    /// Check if a bot storage exists by name
    pub fn exists(name: &str) -> bool {
        get_bot_storage_path(name).exists()
    }

    /// Check if a bot storage exists by bot_id
    pub fn exists_by_id(bot_id: &Uuid) -> bool {
        Self::try_load_by_id(bot_id).is_ok()
    }

    /// Delete a bot storage by name
    pub fn delete(name: &str) -> Result<(), anyhow::Error> {
        let path = get_bot_storage_path(name);
        // Held across the removal so the index and the directory always change together.
        let mut bot_index = lock_bot_index();
        if path.exists() {
            std::fs::remove_file(&path)?;
            log::info!("Deleted LLM buddy bot storage for '{}'", name);
        }
        if let (Some(bots), Some(listed_name)) = (bot_index.as_mut(), listed_bot_name(&path)) {
            bots.retain(|(indexed_name, _)| indexed_name != listed_name);
        }
        Ok(())
    }

    /// List all bot names that have stored data
    pub fn list_all_bot_names() -> Result<Vec<String>, anyhow::Error> {
        let mut names = Vec::new();
        
        if !LLM_BUDDY_BOT_STORAGE_DIR.exists() {
            return Ok(names);
        }

        for entry in std::fs::read_dir(LLM_BUDDY_BOT_STORAGE_DIR.as_path())? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().map_or(false, |ext| ext == "json") {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(name.to_string());
                }
            }
        }

        Ok(names)
    }

    /// Read every bot in the storage directory, in directory listing order
    fn read_storage_dir() -> Result<BotIndex, anyhow::Error> {
        let mut bots = Vec::new();

        for name in Self::list_all_bot_names()? {
            match Self::try_load(&name) {
                Ok(bot) => bots.push((name, bot)),
                Err(e) => log::error!("Failed to load bot '{}': {:?}", name, e),
            }
        }

        Ok(bots)
    }

    /// Run `f` over the in-memory bot index, reading the storage directory on first use
    fn with_bot_index<R>(f: impl FnOnce(&[(String, Self)]) -> R) -> Result<R, anyhow::Error> {
        let mut bot_index = lock_bot_index();
        if bot_index.is_none() {
            // A failed read is not cached, so the next call retries it.
            *bot_index = Some(Self::read_storage_dir()?);
        }
        Ok(f(bot_index.as_deref().unwrap_or_default()))
    }

    /// Load all stored bots
    pub fn load_all() -> Result<Vec<Self>, anyhow::Error> {
        Self::with_bot_index(|bots| bots.iter().map(|(_, bot)| bot.clone()).collect())
    }

    /// Load all stored bots assigned to a specific player
    pub fn load_for_player(player_name: &str) -> Result<Vec<Self>, anyhow::Error> {
        Self::with_bot_index(|bots| {
            bots.iter()
                .filter(|(_, bot)| bot.assigned_player == player_name)
                .map(|(_, bot)| bot.clone())
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bot_storage_creation() {
        let bot_id = Uuid::new_v4();
        let storage = LlmBuddyBotStorage::new(
            bot_id,
            "TestBot".to_string(),
            10,
            "knight".to_string(),
            "TestPlayer".to_string(),
            1,
            PositionData::default(),
        );

        assert_eq!(storage.bot_id, bot_id);
        assert_eq!(storage.name, "TestBot");
        assert_eq!(storage.level, 10);
        assert_eq!(storage.build_type, "knight");
        assert_eq!(storage.assigned_player, "TestPlayer");
    }
}
