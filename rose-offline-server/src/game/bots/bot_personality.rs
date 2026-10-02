//! Persistent per-bot personality and day-to-day life state.
//!
//! Every server-driven bot gets a `BotPersonality` (who it is: archetype,
//! chat style, how social / bold / patient it is) and a `BotLife` (what it is
//! doing with its day: breaks, potion stock, restock trips, level tracking).
//! Behaviour systems scale their timers and thresholds from these so a crowd
//! of bots does not move, talk and rest in lockstep.

use std::time::{Duration, Instant};

use bevy::prelude::{Commands, Component, Entity, Query, With, Without};
use rand::{seq::SliceRandom, Rng};

use crate::game::components::{Level, LlmBuddyBot};

use super::{bot_chat::BotChatter, BotBuild, BotRole};

/// Broad play style. Drives activity weights, timers and chat topics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotArchetype {
    /// Lives to level: long hunting sessions, quick reactions, few words.
    Grinder,
    /// Talks to everyone, greets, emotes, joins groups.
    Socialite,
    /// Takes it easy: long breaks, sits a lot, goes AFK.
    Casual,
    /// Roams a lot and comments on places.
    Explorer,
    /// Hangs around shops and stalls, talks trade.
    Trader,
}

/// How a bot types. Applied to every line it says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatStyle {
    /// Capitals and punctuation as written.
    Proper,
    /// all lowercase, no full stops.
    Lowercase,
    /// lowercase with chat abbreviations (u, ty, pls, brb).
    Shorthand,
    /// Exclamation marks and smileys.
    Cheerful,
}

/// What a bot is currently doing in its daily routine, as far as other
/// systems (mostly chat) need to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BotActivity {
    #[default]
    Idle,
    Shopping,
    Socializing,
    Sitting,
    Afk,
    OnBreak,
    Restocking,
}

#[derive(Component, Clone, Debug)]
pub struct BotPersonality {
    pub archetype: BotArchetype,
    pub chat_style: ChatStyle,
    /// 0..1: how often the bot talks.
    pub chattiness: f32,
    /// 0..1: how readily it greets, reacts, helps and approaches others.
    pub sociability: f32,
    /// 0..1: how far above its own level it is willing to fight.
    pub boldness: f32,
    /// Health fraction below which the bot sits down to recover.
    pub rest_below_hp: f32,
    /// Health fraction at which a resting bot gets back up.
    pub rest_until_hp: f32,
    /// Health fraction below which the bot drinks a potion in a fight.
    pub potion_below_hp: f32,
    /// Delay before picking the next monster after a fight.
    pub reaction: Duration,
    /// Pause before walking off to a new hunting spot.
    pub hunt_pause: Duration,
    /// Runs around towns instead of walking.
    pub runs_in_town: bool,
    /// How long the bot lies dead before reviving.
    pub revive_delay: Duration,
    /// Range of minutes between longer breaks.
    pub break_interval_mins: (f32, f32),
    /// Range of seconds a break lasts.
    pub break_length_secs: (f32, f32),
}

fn range(rng: &mut impl Rng, min: f32, max: f32) -> f32 {
    rng.gen_range(min..=max)
}

fn secs(rng: &mut impl Rng, min: f32, max: f32) -> Duration {
    Duration::from_secs_f32(range(rng, min, max))
}

impl BotPersonality {
    pub fn generate(rng: &mut impl Rng, role: BotRole) -> Self {
        let archetype = {
            let weights: &[(BotArchetype, u32)] = match role {
                BotRole::Town => &[
                    (BotArchetype::Socialite, 35),
                    (BotArchetype::Casual, 30),
                    (BotArchetype::Explorer, 15),
                    (BotArchetype::Trader, 20),
                ],
                BotRole::Fighter => &[
                    (BotArchetype::Grinder, 45),
                    (BotArchetype::Socialite, 20),
                    (BotArchetype::Casual, 20),
                    (BotArchetype::Explorer, 15),
                ],
                BotRole::Healer => &[
                    (BotArchetype::Grinder, 25),
                    (BotArchetype::Socialite, 35),
                    (BotArchetype::Casual, 25),
                    (BotArchetype::Explorer, 15),
                ],
                BotRole::Vendor => &[(BotArchetype::Trader, 1)],
            };
            weights
                .choose_weighted(rng, |(_, weight)| *weight)
                .map(|(archetype, _)| *archetype)
                .unwrap_or(BotArchetype::Casual)
        };

        let chat_style = {
            let weights: &[(ChatStyle, u32)] = match archetype {
                BotArchetype::Grinder => &[
                    (ChatStyle::Proper, 15),
                    (ChatStyle::Lowercase, 35),
                    (ChatStyle::Shorthand, 45),
                    (ChatStyle::Cheerful, 5),
                ],
                BotArchetype::Socialite => &[
                    (ChatStyle::Proper, 25),
                    (ChatStyle::Lowercase, 20),
                    (ChatStyle::Shorthand, 15),
                    (ChatStyle::Cheerful, 40),
                ],
                _ => &[
                    (ChatStyle::Proper, 35),
                    (ChatStyle::Lowercase, 30),
                    (ChatStyle::Shorthand, 20),
                    (ChatStyle::Cheerful, 15),
                ],
            };
            weights
                .choose_weighted(rng, |(_, weight)| *weight)
                .map(|(style, _)| *style)
                .unwrap_or(ChatStyle::Proper)
        };

        match archetype {
            BotArchetype::Grinder => Self {
                archetype,
                chat_style,
                chattiness: range(rng, 0.1, 0.4),
                sociability: range(rng, 0.2, 0.5),
                boldness: range(rng, 0.5, 1.0),
                rest_below_hp: range(rng, 0.3, 0.4),
                rest_until_hp: range(rng, 0.75, 0.9),
                potion_below_hp: range(rng, 0.3, 0.45),
                reaction: secs(rng, 0.25, 0.7),
                hunt_pause: secs(rng, 0.8, 2.5),
                runs_in_town: rng.gen_bool(0.7),
                revive_delay: secs(rng, 6.0, 15.0),
                break_interval_mins: (30.0, 60.0),
                break_length_secs: (25.0, 70.0),
            },
            BotArchetype::Socialite => Self {
                archetype,
                chat_style,
                chattiness: range(rng, 0.6, 1.0),
                sociability: range(rng, 0.7, 1.0),
                boldness: range(rng, 0.2, 0.6),
                rest_below_hp: range(rng, 0.35, 0.5),
                rest_until_hp: range(rng, 0.9, 1.0),
                potion_below_hp: range(rng, 0.25, 0.35),
                reaction: secs(rng, 0.5, 1.2),
                hunt_pause: secs(rng, 2.0, 6.0),
                runs_in_town: rng.gen_bool(0.3),
                revive_delay: secs(rng, 10.0, 30.0),
                break_interval_mins: (15.0, 35.0),
                break_length_secs: (40.0, 120.0),
            },
            BotArchetype::Casual => Self {
                archetype,
                chat_style,
                chattiness: range(rng, 0.4, 0.8),
                sociability: range(rng, 0.4, 0.8),
                boldness: range(rng, 0.0, 0.4),
                rest_below_hp: range(rng, 0.4, 0.55),
                rest_until_hp: range(rng, 0.95, 1.0),
                potion_below_hp: range(rng, 0.25, 0.3),
                reaction: secs(rng, 0.7, 1.6),
                hunt_pause: secs(rng, 3.0, 9.0),
                runs_in_town: rng.gen_bool(0.2),
                revive_delay: secs(rng, 15.0, 45.0),
                break_interval_mins: (10.0, 25.0),
                break_length_secs: (60.0, 180.0),
            },
            BotArchetype::Explorer => Self {
                archetype,
                chat_style,
                chattiness: range(rng, 0.3, 0.7),
                sociability: range(rng, 0.3, 0.7),
                boldness: range(rng, 0.4, 0.8),
                rest_below_hp: range(rng, 0.35, 0.45),
                rest_until_hp: range(rng, 0.85, 1.0),
                potion_below_hp: range(rng, 0.3, 0.4),
                reaction: secs(rng, 0.4, 1.0),
                hunt_pause: secs(rng, 1.0, 4.0),
                runs_in_town: rng.gen_bool(0.6),
                revive_delay: secs(rng, 8.0, 25.0),
                break_interval_mins: (20.0, 45.0),
                break_length_secs: (30.0, 90.0),
            },
            BotArchetype::Trader => Self {
                archetype,
                chat_style,
                chattiness: range(rng, 0.5, 0.9),
                sociability: range(rng, 0.5, 0.9),
                boldness: range(rng, 0.1, 0.5),
                rest_below_hp: range(rng, 0.4, 0.5),
                rest_until_hp: range(rng, 0.9, 1.0),
                potion_below_hp: range(rng, 0.3, 0.4),
                reaction: secs(rng, 0.6, 1.3),
                hunt_pause: secs(rng, 2.0, 6.0),
                runs_in_town: rng.gen_bool(0.3),
                revive_delay: secs(rng, 12.0, 30.0),
                break_interval_mins: (15.0, 30.0),
                break_length_secs: (40.0, 120.0),
            },
        }
    }

    /// Highest monster level above its own this bot will pick a fight with.
    pub fn max_levels_above(&self) -> i32 {
        2 + (self.boldness * 6.0).round() as i32
    }

    /// Time until the next longer break.
    pub fn next_break_in(&self, rng: &mut impl Rng) -> Duration {
        let (min, max) = self.break_interval_mins;
        Duration::from_secs_f32(range(rng, min, max) * 60.0)
    }

    /// Length of one break.
    pub fn break_length(&self, rng: &mut impl Rng) -> Duration {
        let (min, max) = self.break_length_secs;
        secs(rng, min, max)
    }

    /// Rewrites a line the way this bot types.
    pub fn stylize(&self, rng: &mut impl Rng, text: &str) -> String {
        match self.chat_style {
            ChatStyle::Proper => text.to_string(),
            ChatStyle::Lowercase => {
                let mut line = text.to_lowercase();
                if line.ends_with('.') && !line.ends_with("..") {
                    line.pop();
                }
                line
            }
            ChatStyle::Shorthand => shorthand(&text.to_lowercase()),
            ChatStyle::Cheerful => {
                let mut line = text.to_string();
                if line.ends_with('.') && !line.ends_with("..") {
                    line.pop();
                    line.push('!');
                }
                if rng.gen_bool(0.35) {
                    if let Some(smiley) = [" :)", " ^^", " :D", " hehe"].choose(rng) {
                        line.push_str(smiley);
                    }
                }
                line
            }
        }
    }
}

/// Phrase and word abbreviations used by the shorthand chat style. Phrases
/// are matched on the lowercase line before single words.
const SHORTHAND_PHRASES: &[(&str, &str)] = &[
    ("be right back", "brb"),
    ("to be honest", "tbh"),
    ("right now", "rn"),
    ("thank you", "ty"),
    ("going to", "gonna"),
    ("want to", "wanna"),
    ("congratulations", "gz"),
    ("congrats", "gz"),
    ("by the way", "btw"),
    ("i don't know", "idk"),
];

const SHORTHAND_WORDS: &[(&str, &str)] = &[
    ("you", "u"),
    ("your", "ur"),
    ("you're", "ur"),
    ("thanks", "thx"),
    ("please", "pls"),
    ("okay", "ok"),
    ("people", "ppl"),
    ("because", "cuz"),
    ("i'm", "im"),
    ("don't", "dont"),
    ("can't", "cant"),
    ("that's", "thats"),
    ("level", "lvl"),
    ("really", "rly"),
];

fn shorthand(lower: &str) -> String {
    let mut line = lower.to_string();
    for (phrase, short) in SHORTHAND_PHRASES {
        if line.contains(phrase) {
            line = line.replace(phrase, short);
        }
    }

    let words: Vec<String> = line
        .split_whitespace()
        .map(|word| {
            // Keep trailing punctuation ("you?" -> "u?").
            let core_len = word
                .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '\'')
                .len();
            let (core, tail) = word.split_at(core_len);
            match SHORTHAND_WORDS.iter().find(|(long, _)| *long == core) {
                Some((_, short)) => format!("{}{}", short, tail),
                None => word.to_string(),
            }
        })
        .collect();

    let mut line = words.join(" ");
    while line.ends_with('.') || line.ends_with('!') {
        line.pop();
    }
    line
}

/// Day-to-day state of a bot's routine.
#[derive(Component)]
pub struct BotLife {
    pub activity: BotActivity,
    /// Level last seen by the level-up system.
    pub last_level: u32,
    pub next_break_at: Instant,
    /// Set while a break is running (survives an interrupted break).
    pub break_until: Option<Instant>,
    /// Set when the bot wants to walk to a merchant (no potions, bags full,
    /// or just time for a trip).
    pub restock_due: bool,
    pub next_restock_at: Instant,
    pub hp_potions: u32,
    pub mp_potions: u32,
    /// Restock trips also buy mana potions (casters).
    pub mp_potion_user: bool,
    pub next_potion_at: Instant,
    /// Staggered timer for the potion / inventory upkeep check.
    pub next_upkeep_at: Instant,
    /// Staggered timer for looking for real players to party with.
    pub next_invite_scan_at: Instant,
    /// Players this bot already invited (never asks twice).
    pub invited_players: Vec<Entity>,
}

impl BotLife {
    pub fn new(
        rng: &mut impl Rng,
        personality: &BotPersonality,
        role: BotRole,
        level: u32,
        uses_mana: bool,
    ) -> Self {
        let now = Instant::now();
        let fighter = matches!(role, BotRole::Fighter | BotRole::Healer);
        Self {
            activity: BotActivity::Idle,
            last_level: level,
            // First break comes anywhere within one interval so bots spawned
            // together do not all sit down at the same moment.
            next_break_at: now
                + Duration::from_secs_f32(
                    personality.next_break_in(rng).as_secs_f32() * rng.gen_range(0.2..1.0),
                ),
            break_until: None,
            restock_due: false,
            next_restock_at: now + Duration::from_secs(rng.gen_range(25 * 60..80 * 60)),
            hp_potions: if fighter {
                rng.gen_range(10..=30)
            } else {
                rng.gen_range(3..=8)
            },
            mp_potions: if fighter && uses_mana {
                rng.gen_range(5..=15)
            } else {
                0
            },
            mp_potion_user: fighter && uses_mana,
            next_potion_at: now,
            next_upkeep_at: now + Duration::from_millis(rng.gen_range(0..1000)),
            next_invite_scan_at: now + Duration::from_secs(rng.gen_range(20..90)),
            invited_players: Vec::new(),
        }
    }
}

/// Throttled results of the per-bot proximity scans used by scorers, so a
/// scorer does not run a spatial query for every bot on every tick.
#[derive(Component, Default)]
pub struct BotScanCache {
    pub next_target_scan: Option<Instant>,
    pub target_found: bool,
    pub next_item_scan: Option<Instant>,
    pub item_found: bool,
    pub next_party_scan: Option<Instant>,
    pub party_candidate: bool,
}

/// Gives every new server-driven bot a personality and life state. New bots
/// are the only ones matching the query, so this is free once all have one.
#[allow(clippy::type_complexity)]
pub fn bot_assign_personality_system(
    mut commands: Commands,
    query: Query<
        (Entity, &BotBuild, &Level, Option<&BotRole>),
        (
            With<BotBuild>,
            Without<BotPersonality>,
            Without<LlmBuddyBot>,
        ),
    >,
) {
    let mut rng = rand::thread_rng();

    for (entity, bot_build, level, role) in query.iter() {
        let role = role.copied().unwrap_or(BotRole::Fighter);
        let personality = BotPersonality::generate(&mut rng, role);
        // Muse builds (mage / cleric) live on their mana.
        let uses_mana = bot_build.job_id.get() / 100 == 2;
        let life = BotLife::new(&mut rng, &personality, role, level.level, uses_mana);
        let chatter = BotChatter::new(&mut rng, &personality);

        commands
            .entity(entity)
            .insert((personality, life, chatter, BotScanCache::default()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_abbreviates_words_and_phrases() {
        assert_eq!(shorthand("thank you, see you later."), "ty, see u later");
        assert_eq!(
            shorthand("are you going to the shop?"),
            "are u gonna the shop?"
        );
        assert_eq!(shorthand("nice level!"), "nice lvl");
    }

    #[test]
    fn lowercase_style_drops_full_stop_only() {
        let mut rng = rand::thread_rng();
        let mut personality = BotPersonality::generate(&mut rng, BotRole::Town);
        personality.chat_style = ChatStyle::Lowercase;
        assert_eq!(personality.stylize(&mut rng, "Hello There."), "hello there");
        assert_eq!(personality.stylize(&mut rng, "Hmm..."), "hmm...");
    }
}
