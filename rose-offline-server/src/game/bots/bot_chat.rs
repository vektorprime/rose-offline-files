//! Bot chatter: what bots say on their own, and when.
//!
//! Chat is an overlay on top of the thinkers rather than a thinker action:
//! real players talk while they walk, fight and sit, and a chance-per-tick
//! chat choice either never won against the movement choices or cancelled the
//! running activity every time it fired. `bot_chatter_system` owns all
//! speech: queued replies (from `bot_chat_reply`), one-off event lines
//! (level up, loot, death, AFK, ...) and ambient lines picked from the bot's
//! situation (town or field, zone, time of day, fight, health, party, XP).
//! Every line goes through the bot's personality chat style.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use bevy::{
    ecs::query::QueryData,
    prelude::{Component, Entity, MessageWriter, Query, Res, ResMut, Resource, With, Without},
};
use rand::{seq::SliceRandom, Rng};
use rose_data::ZoneId;

use crate::game::{
    components::{
        AbilityValues, CharacterInfo, ChatType, ClientEntity, Command, Dead, ExperiencePoints,
        HealthPoints, Inventory, Level, LlmBuddyBot, Npc, Party, PartyMembership, Position,
    },
    events::{ChatMessageEvent, PartyEvent},
    messages::server::ServerMessage,
    resources::{GameData, ServerMessages, WorldTime},
};

use super::{
    bot_personality::{BotActivity, BotArchetype, BotLife, BotPersonality},
    is_town_zone, BotBuild, BotCombatTarget, VendorBot,
};

/// Topics used to tag heard chat so bots can reply appropriately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotChatTopic {
    Greeting,
    Farewell,
    Thanks,
    /// Someone announcing their own level up ("ding").
    LevelUp,
    /// Someone congratulating someone else ("gz").
    Congrats,
    Question,
    Party,
    Trade,
    Help,
    Battle,
    Death,
    Laugh,
    Random,
}

/// True when `word` occurs in `text` as a whole word (not inside another word).
pub fn contains_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let after = text[start + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn contains_any_word(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| contains_word(text, word))
}

/// Classifies a chat message into a topic using keyword matching. Most
/// specific topics are checked first.
pub fn classify_chat_topic(message: &str) -> BotChatTopic {
    let lower = message.to_lowercase();

    if contains_any_word(&lower, &["gz", "grats", "gratz", "congrats", "grz"])
        || lower.contains("congratulation")
    {
        BotChatTopic::Congrats
    } else if contains_any_word(&lower, &["ding"])
        || lower.contains("level up")
        || lower.contains("lvl up")
        || lower.contains("leveled")
        || lower.contains("levelled")
    {
        BotChatTopic::LevelUp
    } else if contains_any_word(&lower, &["ty", "thx", "thanks", "tyvm"]) || lower.contains("thank")
    {
        BotChatTopic::Thanks
    } else if contains_any_word(&lower, &["bye", "cya", "gn", "goodnight", "later", "brb"])
        || lower.contains("see you")
        || lower.contains("logging off")
    {
        BotChatTopic::Farewell
    } else if contains_any_word(&lower, &["rip", "died", "dead", "dying", "oof"]) {
        BotChatTopic::Death
    } else if lower.contains("party")
        || lower.contains("group")
        || contains_any_word(
            &lower,
            &["lfp", "lfg", "lfm", "inv", "invite", "join", "pt"],
        )
    {
        BotChatTopic::Party
    } else if contains_any_word(
        &lower,
        &[
            "wts", "wtb", "wtt", "sell", "selling", "buy", "buying", "trade", "price", "shop",
        ],
    ) || lower.contains("potion")
        || lower.contains("zuly")
    {
        BotChatTopic::Trade
    } else if lower.contains("help") || lower.contains("need a hand") {
        BotChatTopic::Help
    } else if contains_any_word(&lower, &["hi", "hey", "hello", "yo", "hiya", "sup", "heya"])
        || lower.contains("greetings")
        || lower.contains("good morning")
        || lower.contains("good evening")
    {
        BotChatTopic::Greeting
    } else if contains_any_word(&lower, &["lol", "haha", "hehe", "lmao", "xd", "rofl"]) {
        BotChatTopic::Laugh
    } else if lower.contains('?') {
        BotChatTopic::Question
    } else if lower.contains("fight")
        || lower.contains("attack")
        || lower.contains("kill")
        || lower.contains("hunt")
        || lower.contains("monster")
        || lower.contains("mob")
        || lower.contains("xp")
        || lower.contains("exp")
    {
        BotChatTopic::Battle
    } else {
        BotChatTopic::Random
    }
}

/// Replaces `{key}` placeholders.
pub fn fill_template(template: &str, vars: &[(&str, &str)]) -> String {
    let mut line = template.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{}}}", key);
        if line.contains(&placeholder) {
            line = line.replace(&placeholder, value);
        }
    }
    line
}

/// First word of a character name, which is how players address each other.
pub fn first_name(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

/// iRose job names by job id.
pub fn job_name(job: u16) -> &'static str {
    match job {
        111 => "soldier",
        121 => "knight",
        122 => "champion",
        211 => "muse",
        221 => "mage",
        222 => "cleric",
        311 => "hawker",
        321 => "raider",
        322 => "scout",
        411 => "dealer",
        421 => "bourgeois",
        422 => "artisan",
        _ => "visitor",
    }
}

// ---------------------------------------------------------------------------
// Line pools. Placeholders: {zone} {level} {next} {xp} {job} {monster}
// {name} {item} {killer}.
// ---------------------------------------------------------------------------

const TOWN_LINES: &[&str] = &[
    "So many people in {zone} today.",
    "Need to repair my gear before heading out.",
    "Anyone up for a hunt later?",
    "Finally back in town.",
    "I spent way too much on potions again.",
    "Does anyone know a good spot for level {level}?",
    "Taking it easy for a bit.",
    "Love the music in this town.",
    "Who else is grinding today?",
    "Just finished a quest, need a break.",
    "My bag is so full.",
    "Waiting for my friends to log in.",
    "What's everyone up to?",
    "Hmm, what should I do next...",
];

const TOWN_MORNING_LINES: &[&str] = &[
    "Good morning everyone!",
    "Morning! Coffee and grinding.",
    "Early start today.",
];

const TOWN_EVENING_LINES: &[&str] = &[
    "Getting dark already.",
    "Evening all.",
    "One more hunt before it gets dark?",
];

const TOWN_NIGHT_LINES: &[&str] = &[
    "Late night grind anyone?",
    "It's so quiet at night here.",
    "Should probably sleep soon... one more level.",
];

const TRADER_LINES: &[&str] = &[
    "Prices at the stalls are crazy today.",
    "Anyone selling potions cheap?",
    "Looking for a good deal on armor.",
    "I should open a shop too.",
    "That stall by the plaza has nice stuff.",
];

const SOCIAL_LINES: &[&str] = &[
    "Hi everyone!",
    "Anyone want to hang out?",
    "This server is so friendly.",
    "Who wants to party later?",
    "Nice outfit, whoever is wearing that.",
];

const EXPLORER_LINES: &[&str] = &[
    "Has anyone been past the next zone yet?",
    "I want to explore the whole map.",
    "{zone} looks great this time of day.",
    "I wonder what's past the hills.",
];

const GRINDER_TOWN_LINES: &[&str] = &[
    "Quick restock and back out.",
    "{xp}% to {next}, need to get back out there.",
    "No time to chill, gotta level.",
];

const CASUAL_LINES: &[&str] = &[
    "Just chilling.",
    "So relaxing here.",
    "Not in a hurry today.",
    "I could stand here all day.",
];

const SHOUT_LFG_LINES: &[&str] = &[
    "LF party, level {level} {job}!",
    "Level {level} {job} looking for a group!",
    "Anyone need a {job}? Level {level}.",
    "Looking for people to hunt with, level {level}.",
];

const SHOUT_WTS_LINES: &[&str] = &[
    "WTS {item}, cheap!",
    "Selling {item}, anyone interested?",
    "WTS {item}, come find me in town.",
];

const SHOUT_WTB_LINES: &[&str] = &[
    "WTB potions, paying well!",
    "WTB a better weapon for level {level}.",
    "Buying armor for a level {level} {job}.",
];

const FIELD_FIGHT_LINES: &[&str] = &[
    "These {monster}s hit harder than they look.",
    "Come on, drop something good.",
    "{monster}s everywhere here.",
    "Another one down.",
    "This spot is decent xp.",
    "Respawn is fast here.",
];

const FIELD_LOW_HP_LINES: &[&str] = &[
    "Ouch.",
    "That hurt.",
    "Need to rest soon.",
    "Low hp, careful...",
];

const FIELD_REST_LINES: &[&str] = &[
    "Resting a bit.",
    "Need a breather.",
    "Waiting for my hp...",
    "Sitting for a sec.",
];

const FIELD_PARTY_LINES: &[&str] = &[
    "Nice teamwork.",
    "Pull more!",
    "We should move to another spot soon.",
    "Good party, this is fast xp.",
    "Stay close everyone.",
];

const FIELD_PROGRESS_LINES: &[&str] = &[
    "{xp}% to {next}.",
    "Slowly getting there, {xp}%.",
    "Level {level} takes forever.",
];

const FIELD_ALMOST_LINES: &[&str] = &[
    "Almost {next}!",
    "So close to level {next}.",
    "{xp}%... come on.",
];

const FIELD_GENERAL_LINES: &[&str] = &[
    "{zone} is busy today.",
    "Anyone else hunting here?",
    "Nice weather for a hunt.",
    "Good spot, this one.",
    "Hmm, where did everyone go?",
];

const LEVEL_UP_LINES: &[&str] = &[
    "Ding! {level}!",
    "Level {level}, finally!",
    "Yes! Level {level}!",
    "Woo, {level}!",
    "{level} :D",
];

const LOOT_LINES: &[&str] = &[
    "Nice, got a {item}!",
    "Finally, a {item}.",
    "Ooh, {item} dropped.",
    "Lucky drop, {item}!",
];

const REVIVED_KILLER_LINES: &[&str] = &[
    "That {killer} got me...",
    "Ugh, killed by a {killer}.",
    "Note to self: don't fight {killer}s alone.",
];

const REVIVED_LINES: &[&str] = &[
    "Ugh, died again.",
    "That hurt.",
    "So much xp lost...",
    "Rip me.",
    "Okay, that was dumb.",
];

const AFK_LINES: &[&str] = &["brb", "afk a sec", "be right back", "afk"];
const BACK_LINES: &[&str] = &["back", "Back!", "ok back", "re"];
const BREAK_START_LINES: &[&str] = &[
    "Taking a short break.",
    "brb, getting a drink",
    "Need a break.",
    "Gonna sit for a bit.",
];
const BREAK_END_LINES: &[&str] = &["Okay, back to it.", "Let's go again.", "Back to hunting."];
const RESTOCK_LINES: &[&str] = &[
    "Heading back to restock.",
    "Bag's full, need to sell.",
    "Need more potions, brb.",
];
const OUT_OF_POTIONS_LINES: &[&str] = &[
    "Out of potions...",
    "No more pots, uh oh.",
    "Last potion gone.",
];
const BROWSING_LINES: &[&str] = &[
    "Hmm, pricey.",
    "Nice stuff here.",
    "Can't afford any of this lol.",
    "Ooh, that looks good.",
];
const OPENER_LINES: &[&str] = &[
    "Hey {name}, how's it going?",
    "{name}! What are you up to?",
    "Hi {name}, nice to see you.",
    "{name}, what level are you now?",
    "Yo {name}.",
    "Hey {name}, want to hunt later?",
];

// ---------------------------------------------------------------------------

/// Things worth mentioning soon, turned into a line by the chatter system.
#[derive(Clone, Debug)]
pub enum BotChatEvent {
    LevelUp(u32),
    Loot(String),
    Revived(Option<String>),
    GoingAfk,
    BackFromAfk,
    BreakStart,
    BreakEnd,
    Restocking,
    OutOfPotions,
    Browsing,
    /// Conversation opener addressed to a character the bot walked up to.
    Opener(String),
}

/// A line queued to be said after a "typing" delay.
#[derive(Clone, Debug)]
pub struct PendingLine {
    /// Unstyled text; the personality style is applied when said.
    pub text: String,
    pub say_at: Instant,
    /// Conversation depth (0 = opener, 1 = reply, ...).
    pub depth: u8,
    pub shout: bool,
    /// Real player to invite to the bot's party once the line is said.
    pub invite: Option<Entity>,
}

#[derive(Component)]
pub struct BotChatter {
    pub next_ambient_at: Instant,
    pub last_said_at: Option<Instant>,
    /// Conversation depth of the last line this bot said.
    pub last_said_depth: u8,
    pub pending: Option<PendingLine>,
    pub event: Option<(BotChatEvent, Instant)>,
    /// When the bot last announced a level up, so listeners know that line
    /// is a "ding" whatever words it used.
    pub leveled_at: Option<Instant>,
}

impl BotChatter {
    pub fn new(rng: &mut impl Rng, personality: &BotPersonality) -> Self {
        Self {
            // Stagger the first line over a whole interval.
            next_ambient_at: Instant::now()
                + Duration::from_secs_f32(
                    ambient_interval(rng, personality).as_secs_f32() * rng.gen_range(0.1..1.0),
                ),
            last_said_at: None,
            last_said_depth: 0,
            pending: None,
            event: None,
            leveled_at: None,
        }
    }

    /// Queues an event line after a short human delay. A newer event replaces
    /// an older unspoken one.
    pub fn notify(&mut self, rng: &mut impl Rng, event: BotChatEvent) {
        let delay = Duration::from_millis(rng.gen_range(900..3500));
        self.event = Some((event, Instant::now() + delay));
    }

    /// Queues a reply. Replies replace any unspoken line.
    pub fn queue_line(&mut self, line: PendingLine) {
        self.pending = Some(line);
    }
}

/// Time until a bot's next ambient line: chatty bots talk more often.
fn ambient_interval(rng: &mut impl Rng, personality: &BotPersonality) -> Duration {
    let base = rng.gen_range(150.0..420.0f32);
    Duration::from_secs_f32(base / (0.4 + personality.chattiness))
}

/// Human typing delay for a line of `len` characters.
pub fn typing_delay(rng: &mut impl Rng, len: usize) -> Duration {
    Duration::from_secs_f32(0.8 + 0.045 * len as f32 + rng.gen_range(0.3..1.8f32))
}

/// Minimum gap between ambient lines of different bots in one zone, so a
/// zone never fills up with unrelated chatter at once.
const ZONE_AMBIENT_GAP: Duration = Duration::from_secs(3);

/// Queued lines older than this (the bot was dead meanwhile) are dropped.
const STALE_LINE: Duration = Duration::from_secs(8);

/// Per-zone pacing for ambient lines and zone-wide shouts.
#[derive(Resource, Default)]
pub struct BotChatZoneLimits {
    last_ambient: HashMap<ZoneId, Instant>,
    next_shout_at: HashMap<ZoneId, Instant>,
}

/// Says a line: local chat (nearby clients) or a zone-wide shout, plus the
/// chat event other bots listen to.
#[allow(clippy::too_many_arguments)]
pub fn bot_say(
    server_messages: &mut ServerMessages,
    chat_message_events: &mut MessageWriter<ChatMessageEvent>,
    entity: Entity,
    client_entity: &ClientEntity,
    name: &str,
    zone_id: ZoneId,
    text: String,
    shout: bool,
) {
    if shout {
        server_messages.send_zone_message(
            zone_id,
            ServerMessage::ShoutChat {
                name: name.to_string(),
                text: text.clone(),
            },
        );
    } else {
        server_messages.send_entity_message(
            client_entity,
            ServerMessage::LocalChat {
                entity_id: client_entity.id,
                text: text.clone(),
            },
        );
    }

    chat_message_events.write(ChatMessageEvent {
        sender_entity: entity,
        sender_name: name.to_string(),
        zone_id,
        message: text,
        chat_type: if shout {
            ChatType::Shout
        } else {
            ChatType::Local
        },
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DayPhase {
    Morning,
    Day,
    Evening,
    Night,
}

fn zone_day_phase(world_time: &WorldTime, zone_data: &rose_data::ZoneData) -> DayPhase {
    if zone_data.day_cycle == 0 {
        return DayPhase::Day;
    }
    let time = world_time.ticks.get_world_time() % zone_data.day_cycle;
    if time >= zone_data.night_time || time < zone_data.morning_time {
        DayPhase::Night
    } else if time >= zone_data.evening_time {
        DayPhase::Evening
    } else if time >= zone_data.day_time {
        DayPhase::Day
    } else {
        DayPhase::Morning
    }
}

fn pick<'a>(rng: &mut impl Rng, lines: &[&'a str]) -> Option<&'a str> {
    lines.choose(rng).copied()
}

/// Line for a one-off event, or `None` when the bot keeps it to itself.
fn event_line(
    rng: &mut impl Rng,
    personality: &BotPersonality,
    event: &BotChatEvent,
) -> Option<String> {
    let chance = match event {
        BotChatEvent::LevelUp(_) => 0.85,
        BotChatEvent::Opener(_) => 0.9,
        BotChatEvent::Loot(_) => 0.4 + 0.4 * personality.chattiness,
        BotChatEvent::Revived(_) => 0.3 + 0.5 * personality.chattiness,
        BotChatEvent::GoingAfk | BotChatEvent::BackFromAfk => 0.5,
        BotChatEvent::BreakStart | BotChatEvent::BreakEnd => 0.2 + 0.5 * personality.chattiness,
        BotChatEvent::Restocking | BotChatEvent::OutOfPotions => {
            0.25 + 0.5 * personality.chattiness
        }
        BotChatEvent::Browsing => 0.15 + 0.4 * personality.chattiness,
    };
    if !rng.gen_bool(chance as f64) {
        return None;
    }

    let line = match event {
        BotChatEvent::LevelUp(level) => {
            let level = level.to_string();
            fill_template(pick(rng, LEVEL_UP_LINES)?, &[("level", level.as_str())])
        }
        BotChatEvent::Loot(item) => {
            fill_template(pick(rng, LOOT_LINES)?, &[("item", item.as_str())])
        }
        BotChatEvent::Revived(Some(killer)) if rng.gen_bool(0.6) => fill_template(
            pick(rng, REVIVED_KILLER_LINES)?,
            &[("killer", killer.as_str())],
        ),
        BotChatEvent::Revived(_) => pick(rng, REVIVED_LINES)?.to_string(),
        BotChatEvent::GoingAfk => pick(rng, AFK_LINES)?.to_string(),
        BotChatEvent::BackFromAfk => pick(rng, BACK_LINES)?.to_string(),
        BotChatEvent::BreakStart => pick(rng, BREAK_START_LINES)?.to_string(),
        BotChatEvent::BreakEnd => pick(rng, BREAK_END_LINES)?.to_string(),
        BotChatEvent::Restocking => pick(rng, RESTOCK_LINES)?.to_string(),
        BotChatEvent::OutOfPotions => pick(rng, OUT_OF_POTIONS_LINES)?.to_string(),
        BotChatEvent::Browsing => pick(rng, BROWSING_LINES)?.to_string(),
        BotChatEvent::Opener(name) => {
            fill_template(pick(rng, OPENER_LINES)?, &[("name", first_name(name))])
        }
    };
    Some(line)
}

/// What an ambient line can talk about.
struct AmbientContext<'a> {
    town: bool,
    zone_name: &'a str,
    phase: DayPhase,
    job: &'static str,
    xp_percent: u32,
    hp_ratio: f32,
    fighting: bool,
    sitting: bool,
    in_party: bool,
    monster: Option<&'a str>,
    item_for_sale: Option<&'a str>,
    archetype: BotArchetype,
}

/// Picks a context-appropriate ambient line. Returns the template and
/// whether it is a zone-wide shout.
fn ambient_template(rng: &mut impl Rng, ctx: &AmbientContext) -> Option<(&'static str, bool)> {
    let mut pools: Vec<(&[&'static str], u32, bool)> = Vec::with_capacity(8);

    if ctx.town {
        pools.push((TOWN_LINES, 6, false));
        match ctx.phase {
            DayPhase::Morning => pools.push((TOWN_MORNING_LINES, 2, false)),
            DayPhase::Evening => pools.push((TOWN_EVENING_LINES, 2, false)),
            DayPhase::Night => pools.push((TOWN_NIGHT_LINES, 2, false)),
            DayPhase::Day => {}
        }
        match ctx.archetype {
            BotArchetype::Trader => pools.push((TRADER_LINES, 4, false)),
            BotArchetype::Socialite => pools.push((SOCIAL_LINES, 4, false)),
            BotArchetype::Explorer => pools.push((EXPLORER_LINES, 3, false)),
            BotArchetype::Grinder => pools.push((GRINDER_TOWN_LINES, 3, false)),
            BotArchetype::Casual => pools.push((CASUAL_LINES, 3, false)),
        }
        // Occasional zone-wide shouts: LFG from social / grinding players,
        // trade shouts from traders with something to sell.
        if !ctx.in_party
            && matches!(
                ctx.archetype,
                BotArchetype::Socialite | BotArchetype::Grinder | BotArchetype::Explorer
            )
        {
            pools.push((SHOUT_LFG_LINES, 1, true));
        }
        if matches!(ctx.archetype, BotArchetype::Trader | BotArchetype::Casual) {
            if ctx.item_for_sale.is_some() {
                pools.push((SHOUT_WTS_LINES, 2, true));
            }
            pools.push((SHOUT_WTB_LINES, 1, true));
        }
    } else {
        if ctx.fighting && ctx.monster.is_some() {
            pools.push((FIELD_FIGHT_LINES, 4, false));
        }
        if ctx.hp_ratio < 0.5 && !ctx.sitting {
            pools.push((FIELD_LOW_HP_LINES, 3, false));
        }
        if ctx.sitting {
            pools.push((FIELD_REST_LINES, 4, false));
        }
        if ctx.in_party {
            pools.push((FIELD_PARTY_LINES, 3, false));
        }
        if ctx.xp_percent >= 85 {
            pools.push((FIELD_ALMOST_LINES, 2, false));
        } else {
            pools.push((FIELD_PROGRESS_LINES, 1, false));
        }
        pools.push((FIELD_GENERAL_LINES, 2, false));
    }

    let (lines, _, shout) = *pools.choose_weighted(rng, |(_, weight, _)| *weight).ok()?;
    let template = lines.choose(rng).copied()?;

    // Templates that need a monster or item are skipped when unknown.
    if (template.contains("{monster}") && ctx.monster.is_none())
        || (template.contains("{item}") && ctx.item_for_sale.is_none())
    {
        return None;
    }
    Some((template, shout))
}

#[derive(QueryData)]
#[query_data(mutable)]
pub struct BotChatterQuery<'w> {
    entity: Entity,
    client_entity: &'w ClientEntity,
    character_info: &'w CharacterInfo,
    position: &'w Position,
    command: &'w Command,
    level: &'w Level,
    experience_points: &'w ExperiencePoints,
    health_points: &'w HealthPoints,
    ability_values: &'w AbilityValues,
    party_membership: &'w PartyMembership,
    inventory: &'w Inventory,
    combat_target: Option<&'w BotCombatTarget>,
    personality: &'w BotPersonality,
    life: &'w mut BotLife,
    chatter: &'w mut BotChatter,
}

/// Says queued replies and event lines, and decides on ambient lines.
#[allow(clippy::too_many_arguments)]
pub fn bot_chatter_system(
    mut query: Query<
        BotChatterQuery,
        (
            With<BotBuild>,
            Without<LlmBuddyBot>,
            Without<VendorBot>,
            Without<Dead>,
        ),
    >,
    query_npc: Query<&Npc>,
    query_party: Query<&Party>,
    game_data: Res<GameData>,
    world_time: Res<WorldTime>,
    mut zone_limits: ResMut<BotChatZoneLimits>,
    mut server_messages: ResMut<ServerMessages>,
    mut chat_message_events: MessageWriter<ChatMessageEvent>,
    mut party_events: MessageWriter<PartyEvent>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    for mut bot in query.iter_mut() {
        // 1. A queued reply whose typing delay has passed.
        if bot
            .chatter
            .pending
            .as_ref()
            .is_some_and(|pending| pending.say_at <= now)
        {
            let Some(pending) = bot.chatter.pending.take() else {
                continue;
            };
            // AFK players do not answer, and a line that waited out a death
            // is no longer worth saying.
            if bot.life.activity == BotActivity::Afk
                || bot.command.is_dead()
                || now.saturating_duration_since(pending.say_at) > STALE_LINE
            {
                continue;
            }

            let text = bot.personality.stylize(&mut rng, &pending.text);
            bot_say(
                &mut server_messages,
                &mut chat_message_events,
                bot.entity,
                bot.client_entity,
                &bot.character_info.name,
                bot.position.zone_id,
                text,
                pending.shout,
            );
            bot.chatter.last_said_at = Some(now);
            bot.chatter.last_said_depth = pending.depth;

            if let Some(player) = pending.invite {
                if !bot.life.invited_players.contains(&player) {
                    bot.life.invited_players.push(player);
                }
                party_events.write(PartyEvent::Invite {
                    owner_entity: bot.entity,
                    invited_entity: player,
                });
            }
            continue;
        }

        // 2. A one-off event line.
        if bot.chatter.event.as_ref().is_some_and(|(_, at)| *at <= now) {
            let Some((event, at)) = bot.chatter.event.take() else {
                continue;
            };
            if now.saturating_duration_since(at) > STALE_LINE {
                continue;
            }
            if let Some(line) = event_line(&mut rng, bot.personality, &event) {
                let text = bot.personality.stylize(&mut rng, &line);
                bot_say(
                    &mut server_messages,
                    &mut chat_message_events,
                    bot.entity,
                    bot.client_entity,
                    &bot.character_info.name,
                    bot.position.zone_id,
                    text,
                    false,
                );
                bot.chatter.last_said_at = Some(now);
                if matches!(event, BotChatEvent::LevelUp(_)) {
                    bot.chatter.leveled_at = Some(now);
                }
                bot.chatter.last_said_depth = 0;
            }
            continue;
        }

        // 3. Ambient line.
        if now < bot.chatter.next_ambient_at {
            continue;
        }
        let next_in = ambient_interval(&mut rng, bot.personality);
        bot.chatter.next_ambient_at = now + next_in;

        if bot.life.activity == BotActivity::Afk || bot.chatter.pending.is_some() {
            continue;
        }
        // Don't talk over yourself.
        if bot
            .chatter
            .last_said_at
            .is_some_and(|last| now.saturating_duration_since(last) < Duration::from_secs(20))
        {
            continue;
        }

        let zone_id = bot.position.zone_id;
        if zone_limits
            .last_ambient
            .get(&zone_id)
            .is_some_and(|last| now.saturating_duration_since(*last) < ZONE_AMBIENT_GAP)
        {
            continue;
        }

        let Some(zone_data) = game_data.zones.get_zone(zone_id) else {
            continue;
        };

        let level = bot.level.level;
        let required_xp = game_data
            .ability_value_calculator
            .calculate_levelup_require_xp(level)
            .max(1);
        let xp_percent = ((bot.experience_points.xp.min(required_xp) * 100) / required_xp) as u32;
        let max_health = bot.ability_values.get_max_health().max(1);
        let monster = bot
            .combat_target
            .and_then(|target| query_npc.get(target.entity()).ok())
            .and_then(|npc| game_data.npcs.get_npc(npc.id))
            .map(|npc_data| npc_data.name.as_str())
            .filter(|name| !name.is_empty());
        let item_for_sale = bot
            .inventory
            .equipment
            .slots
            .iter()
            .flatten()
            .filter_map(|item| game_data.items.get_base_item(item.get_item_reference()))
            .map(|item_data| item_data.name.as_str())
            .find(|name| !name.is_empty());
        let in_party = bot
            .party_membership
            .party
            .is_some_and(|party| query_party.contains(party));

        let ctx = AmbientContext {
            town: is_town_zone(zone_data),
            zone_name: &zone_data.name,
            phase: zone_day_phase(&world_time, zone_data),
            job: job_name(bot.character_info.job),
            xp_percent,
            hp_ratio: bot.health_points.hp as f32 / max_health as f32,
            fighting: bot.combat_target.is_some(),
            sitting: bot.command.is_sit(),
            in_party,
            monster,
            item_for_sale,
            archetype: bot.personality.archetype,
        };

        let Some((template, shout)) = ambient_template(&mut rng, &ctx) else {
            continue;
        };

        if shout {
            if zone_limits
                .next_shout_at
                .get(&zone_id)
                .is_some_and(|next| now < *next)
            {
                continue;
            }
            zone_limits
                .next_shout_at
                .insert(zone_id, now + Duration::from_secs(rng.gen_range(75..150)));
        }

        let level_text = level.to_string();
        let next_text = (level + 1).to_string();
        let xp_text = xp_percent.to_string();
        let line = fill_template(
            template,
            &[
                ("zone", ctx.zone_name),
                ("level", level_text.as_str()),
                ("next", next_text.as_str()),
                ("xp", xp_text.as_str()),
                ("job", ctx.job),
                ("monster", ctx.monster.unwrap_or("")),
                ("item", ctx.item_for_sale.unwrap_or("")),
            ],
        );
        let text = bot.personality.stylize(&mut rng, &line);

        zone_limits.last_ambient.insert(zone_id, now);
        bot_say(
            &mut server_messages,
            &mut chat_message_events,
            bot.entity,
            bot.client_entity,
            &bot.character_info.name,
            zone_id,
            text,
            shout,
        );
        bot.chatter.last_said_at = Some(now);
        bot.chatter.last_said_depth = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_chat() {
        assert_eq!(classify_chat_topic("gz!"), BotChatTopic::Congrats);
        assert_eq!(classify_chat_topic("ding 25"), BotChatTopic::LevelUp);
        assert_eq!(
            classify_chat_topic("gz on the ding"),
            BotChatTopic::Congrats
        );
        assert_eq!(classify_chat_topic("thanks a lot"), BotChatTopic::Thanks);
        assert_eq!(classify_chat_topic("hey there"), BotChatTopic::Greeting);
        assert_eq!(classify_chat_topic("this is fine"), BotChatTopic::Random);
        assert_eq!(
            classify_chat_topic("anyone want to party?"),
            BotChatTopic::Party
        );
        assert_eq!(
            classify_chat_topic("where is the shop?"),
            BotChatTopic::Trade
        );
        assert_eq!(
            classify_chat_topic("what time is it?"),
            BotChatTopic::Question
        );
        assert_eq!(classify_chat_topic("lol"), BotChatTopic::Laugh);
    }

    #[test]
    fn fills_templates() {
        assert_eq!(
            fill_template("Ding! {level}! ({level})", &[("level", "12")]),
            "Ding! 12! (12)"
        );
        assert_eq!(first_name("Lena Ashvale"), "Lena");
    }
}
