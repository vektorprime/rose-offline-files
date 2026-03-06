use std::path::PathBuf;

use directories::ProjectDirs;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref LOCAL_STORAGE_DIR: PathBuf = {
        let project = ProjectDirs::from("", "", "rose-offline").unwrap();
        PathBuf::from(project.data_local_dir())
    };
    pub static ref ACCOUNT_STORAGE_DIR: PathBuf = LOCAL_STORAGE_DIR.join("accounts");
    pub static ref BANK_STORAGE_DIR: PathBuf = LOCAL_STORAGE_DIR.join("bank");
    pub static ref CHARACTER_STORAGE_DIR: PathBuf = LOCAL_STORAGE_DIR.join("characters");
    pub static ref CLAN_STORAGE_DIR: PathBuf = LOCAL_STORAGE_DIR.join("clan");
    pub static ref LLM_BUDDY_BOT_STORAGE_DIR: PathBuf = LOCAL_STORAGE_DIR.join("llm_buddy_bots");
}

pub mod account;
pub mod bank;
pub mod character;
pub mod clan;
pub mod llm_buddy_bot;
