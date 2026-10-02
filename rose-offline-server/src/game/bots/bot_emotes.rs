//! Emotes for bots.
//!
//! Emote motions are rows of the character motion table that the game's
//! emote skills (`SkillType::Emote`) play. The old hard-coded ids 15-19 were
//! jump / pick-up rows (see `rose-data-irose` `get_action_map`), so they are
//! read from the skill table instead and grouped by the skill name.
//!
//! `command_system` never announces an emote `NextCommand` to clients (there
//! is no `UseEmote` broadcast for it), so bot emotes were invisible. Bots send
//! the `UseEmote` message themselves and queue the command with the "already
//! announced" flag set, so a future broadcast in `command_system` (which only
//! announces unsent commands) cannot double it.

use bevy::prelude::{Commands, Entity, Res, ResMut, Resource};
use rand::{seq::SliceRandom, Rng};
use rose_data::{MotionId, SkillType};

use crate::game::{
    components::{ClientEntity, CommandData, NextCommand},
    messages::server::ServerMessage,
    resources::{GameData, ServerMessages},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotEmoteKind {
    Greet,
    Cheer,
    Bow,
    Laugh,
    Dance,
    Sad,
    Any,
}

const GREET_WORDS: &[&str] = &["greet", "hello", "wave", "salute", "welcome"];
const CHEER_WORDS: &[&str] = &[
    "cheer", "clap", "applau", "hurray", "hooray", "victory", "yay", "joy", "congrat", "fight",
];
const BOW_WORDS: &[&str] = &["bow", "thank", "polite", "respect", "courtesy"];
const LAUGH_WORDS: &[&str] = &["laugh", "smile", "happy", "lol", "funny"];
const DANCE_WORDS: &[&str] = &["dance"];
const SAD_WORDS: &[&str] = &[
    "sad", "cry", "sorry", "sigh", "tired", "depress", "frustrat",
];

/// Emote motions grouped by mood, built from the skill table at startup.
#[derive(Resource, Default)]
pub struct BotEmotes {
    greet: Vec<MotionId>,
    cheer: Vec<MotionId>,
    bow: Vec<MotionId>,
    laugh: Vec<MotionId>,
    dance: Vec<MotionId>,
    sad: Vec<MotionId>,
    all: Vec<MotionId>,
}

impl BotEmotes {
    fn list(&self, kind: BotEmoteKind) -> &[MotionId] {
        match kind {
            BotEmoteKind::Greet => &self.greet,
            BotEmoteKind::Cheer => &self.cheer,
            BotEmoteKind::Bow => &self.bow,
            BotEmoteKind::Laugh => &self.laugh,
            BotEmoteKind::Dance => &self.dance,
            BotEmoteKind::Sad => &self.sad,
            BotEmoteKind::Any => &self.all,
        }
    }

    /// A motion for `kind`, or any emote when the data has none of that kind.
    pub fn pick(&self, rng: &mut impl Rng, kind: BotEmoteKind) -> Option<MotionId> {
        self.list(kind)
            .choose(rng)
            .or_else(|| self.all.choose(rng))
            .copied()
    }

    /// The mood of an emote someone else played.
    pub fn kind_of(&self, motion_id: MotionId) -> BotEmoteKind {
        for kind in [
            BotEmoteKind::Greet,
            BotEmoteKind::Cheer,
            BotEmoteKind::Bow,
            BotEmoteKind::Laugh,
            BotEmoteKind::Dance,
            BotEmoteKind::Sad,
        ] {
            if self.list(kind).contains(&motion_id) {
                return kind;
            }
        }
        BotEmoteKind::Any
    }
}

fn name_matches(name: &str, words: &[&str]) -> bool {
    words.iter().any(|word| name.contains(word))
}

/// Fills `BotEmotes` from the emote skills once game data is loaded.
pub fn bot_emotes_init_system(mut emotes: ResMut<BotEmotes>, game_data: Res<GameData>) {
    for skill_data in game_data.skills.iter() {
        if !matches!(skill_data.skill_type, SkillType::Emote) {
            continue;
        }
        let Some(motion_id) = skill_data.action_motion_id else {
            continue;
        };
        if emotes.all.contains(&motion_id) {
            continue;
        }

        let name = skill_data.name.to_lowercase();
        emotes.all.push(motion_id);
        if name_matches(&name, GREET_WORDS) {
            emotes.greet.push(motion_id);
        }
        if name_matches(&name, CHEER_WORDS) {
            emotes.cheer.push(motion_id);
        }
        if name_matches(&name, BOW_WORDS) {
            emotes.bow.push(motion_id);
        }
        if name_matches(&name, LAUGH_WORDS) {
            emotes.laugh.push(motion_id);
        }
        if name_matches(&name, DANCE_WORDS) {
            emotes.dance.push(motion_id);
        }
        if name_matches(&name, SAD_WORDS) {
            emotes.sad.push(motion_id);
        }
    }

    log::info!(
        "Bot emotes: {} emote motions (greet {}, cheer {}, bow {}, laugh {}, dance {}, sad {})",
        emotes.all.len(),
        emotes.greet.len(),
        emotes.cheer.len(),
        emotes.bow.len(),
        emotes.laugh.len(),
        emotes.dance.len(),
        emotes.sad.len(),
    );
}

/// Plays an emote on a bot and tells nearby clients about it.
pub fn bot_play_emote(
    commands: &mut Commands,
    server_messages: &mut ServerMessages,
    entity: Entity,
    client_entity: &ClientEntity,
    motion_id: MotionId,
) {
    commands
        .entity(entity)
        .insert(NextCommand::with_command_skip_server_message(
            CommandData::Emote {
                motion_id,
                is_stop: true,
            },
        ));
    server_messages.send_entity_message(
        client_entity,
        ServerMessage::UseEmote {
            entity_id: client_entity.id,
            motion_id,
            is_stop: true,
        },
    );
}

/// Picks and plays an emote of `kind`. Returns false when the data has no
/// emotes at all.
pub fn bot_play_emote_kind(
    commands: &mut Commands,
    server_messages: &mut ServerMessages,
    emotes: &BotEmotes,
    rng: &mut impl Rng,
    entity: Entity,
    client_entity: &ClientEntity,
    kind: BotEmoteKind,
) -> bool {
    match emotes.pick(rng, kind) {
        Some(motion_id) => {
            bot_play_emote(commands, server_messages, entity, client_entity, motion_id);
            true
        }
        None => false,
    }
}
