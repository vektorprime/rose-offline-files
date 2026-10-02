//! Hearing and answering chat.
//!
//! Only bots within earshot of the speaker hear a line (the old capture
//! pushed every zone message into every bot in the zone, so bots across the
//! map answered lines the player never saw). For each line a few nearby bots
//! may decide to answer: a bot addressed by name almost always does, real
//! players get answered more readily than bots, and bot-to-bot exchanges die
//! out after a few lines. The answer is queued on the bot's `BotChatter` with
//! a typing delay and said by `bot_chatter_system`.

use std::time::{Duration, Instant};

use bevy::{
    ecs::query::QueryData,
    math::Vec3Swizzles,
    prelude::{Entity, Has, MessageReader, ParamSet, Query, Res, With, Without},
};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    components::{
        CharacterInfo, ChatType, ClientEntityType, Dead, GameClient, Level, LlmBuddyBot, Party,
        PartyMembership, Position,
    },
    events::ChatMessageEvent,
    resources::{ClientEntityList, GameConfig},
};

use super::{
    bot_chat::{
        classify_chat_topic, contains_word, fill_template, first_name, typing_delay, BotChatTopic,
        BotChatter, PendingLine,
    },
    bot_personality::{BotActivity, BotLife, BotPersonality},
    BotBuild, BotChatMemory, BotRole, VendorBot, BOT_CHAT_MEMORY_SIZE, BOT_CHAT_REPLY_COOLDOWN,
};

/// Bots within this distance of a speaker hear local chat.
const HEAR_RANGE: f32 = 1500.0;
/// Beyond this distance a bot is less likely to join in.
const NEAR_RANGE: f32 = 600.0;
/// Bot-to-bot conversations stop after this many lines.
const MAX_DEPTH: u8 = 3;
/// A bot's line counts as part of a conversation for this long.
const CONVERSATION_WINDOW: Duration = Duration::from_secs(20);
/// A bot line said this soon after it announced a level up is the "ding".
const LEVEL_UP_WINDOW: Duration = Duration::from_secs(2);
/// Level gap within which a bot offers a real player a party.
const PARTY_LEVEL_GAP: i32 = 10;
/// A bot does not answer the same person on the same topic twice within this.
const SAME_TOPIC_WINDOW: Duration = Duration::from_secs(60);

const GREETING_REPLIES: &[&str] = &[
    "Hi {name}!",
    "Hey {name}.",
    "Hello!",
    "Yo {name}",
    "Hiya!",
    "Hey, how's it going?",
];
const FAREWELL_REPLIES: &[&str] = &[
    "Bye {name}!",
    "Cya!",
    "Take care.",
    "See you around {name}.",
];
const THANKS_REPLIES: &[&str] = &["No problem!", "Np.", "Anytime.", "You're welcome {name}."];
const CONGRATS_THANKS_REPLIES: &[&str] = &["Ty!", "Thanks!", "Thank you :)", "Hehe, thanks."];
const CONGRATS_REPLIES: &[&str] = &[
    "Gz {name}!",
    "Grats!",
    "Congrats {name}!",
    "Nice, gz!",
    "Gratz :)",
];
const QUESTION_REPLIES: &[&str] = &[
    "Not much, you?",
    "Just hanging around.",
    "Not sure, sorry.",
    "Hmm, good question.",
    "Maybe ask one of the town NPCs?",
    "I don't know, I'm still new here.",
];
const LEVEL_QUESTION_REPLIES: &[&str] =
    &["I'm level {level}.", "Level {level} now.", "{level}, you?"];
const PARTY_INVITE_REPLIES: &[&str] = &[
    "Sure {name}, inviting you.",
    "I'll invite you!",
    "Yeah, join me {name}.",
    "Sure, sending an invite.",
];
const PARTY_BUSY_REPLIES: &[&str] = &[
    "I'm in a party already, sorry.",
    "Our party is full, sorry.",
];
const PARTY_OTHER_REPLIES: &[&str] = &[
    "What level are you?",
    "Maybe later.",
    "Where are you hunting?",
    "Not right now, sorry.",
];
const TRADE_REPLIES: &[&str] = &[
    "What are you selling?",
    "How much?",
    "Check the stalls near the plaza.",
    "No thanks.",
    "Potions are cheaper at the NPC.",
];
const HELP_REPLIES: &[&str] = &[
    "What do you need?",
    "On my way!",
    "Where are you?",
    "Hang in there!",
];
const BATTLE_REPLIES: &[&str] = &[
    "Nice.",
    "Careful out there.",
    "They respawn fast here.",
    "Good luck!",
];
const DEATH_REPLIES: &[&str] = &[
    "Rip.",
    "Oof.",
    "Happens to all of us.",
    "Rip {name}, next time.",
];
const LAUGH_REPLIES: &[&str] = &["Haha", "Lol", ":D", "Hehe"];
const RANDOM_REPLIES: &[&str] = &[
    "True.",
    "Yeah.",
    "Haha yeah.",
    "Same here.",
    "Hmm.",
    "Fair enough.",
    "Really?",
];

/// Chance that a hearing bot answers a line.
fn reply_chance(topic: BotChatTopic, addressed: bool, from_player: bool, depth: u8) -> f32 {
    if addressed {
        return if from_player {
            0.9
        } else if depth < MAX_DEPTH {
            0.75
        } else {
            0.0
        };
    }

    // Greetings, goodbyes and congratulations between bots are meant for
    // whoever they name; only answer those when addressed.
    if !from_player
        && matches!(
            topic,
            BotChatTopic::Greeting | BotChatTopic::Farewell | BotChatTopic::Congrats
        )
    {
        return 0.0;
    }

    let base = match topic {
        BotChatTopic::LevelUp => 0.6,
        BotChatTopic::Greeting => 0.45,
        BotChatTopic::Question => 0.45,
        BotChatTopic::Party => 0.4,
        BotChatTopic::Help => 0.35,
        BotChatTopic::Death => 0.35,
        BotChatTopic::Farewell => 0.3,
        BotChatTopic::Trade => 0.25,
        BotChatTopic::Battle => 0.2,
        BotChatTopic::Laugh => 0.15,
        BotChatTopic::Random => 0.12,
        // Thanks and congratulations are for whoever they were meant for.
        BotChatTopic::Thanks | BotChatTopic::Congrats => 0.0,
    };

    if from_player {
        base
    } else {
        match depth {
            0 => base * 0.6,
            1 => base * 0.2,
            _ => 0.0,
        }
    }
}

/// What a bot knows about itself when answering.
struct ReplyContext<'a> {
    sender_name: &'a str,
    message: &'a str,
    level: u32,
    in_party: bool,
    party_full: bool,
    offers_party: bool,
}

fn reply_text(rng: &mut impl Rng, topic: BotChatTopic, ctx: &ReplyContext) -> Option<String> {
    let pool: &[&str] = match topic {
        BotChatTopic::Greeting => GREETING_REPLIES,
        BotChatTopic::Farewell => FAREWELL_REPLIES,
        BotChatTopic::Thanks => THANKS_REPLIES,
        BotChatTopic::LevelUp => CONGRATS_REPLIES,
        BotChatTopic::Congrats => CONGRATS_THANKS_REPLIES,
        BotChatTopic::Question => {
            if contains_word(ctx.message, "level") || contains_word(ctx.message, "lvl") {
                LEVEL_QUESTION_REPLIES
            } else {
                QUESTION_REPLIES
            }
        }
        BotChatTopic::Party => {
            if ctx.offers_party {
                PARTY_INVITE_REPLIES
            } else if ctx.in_party && ctx.party_full {
                PARTY_BUSY_REPLIES
            } else {
                PARTY_OTHER_REPLIES
            }
        }
        BotChatTopic::Trade => TRADE_REPLIES,
        BotChatTopic::Help => HELP_REPLIES,
        BotChatTopic::Battle => BATTLE_REPLIES,
        BotChatTopic::Death => DEATH_REPLIES,
        BotChatTopic::Laugh => LAUGH_REPLIES,
        BotChatTopic::Random => RANDOM_REPLIES,
    };

    let template = pool.choose(rng)?;
    let level = ctx.level.to_string();
    Some(fill_template(
        template,
        &[("name", ctx.sender_name), ("level", level.as_str())],
    ))
}

#[derive(QueryData)]
#[query_data(mutable)]
pub struct HearingBotQuery<'w> {
    entity: Entity,
    character_info: &'w CharacterInfo,
    level: &'w Level,
    party_membership: &'w PartyMembership,
    role: Option<&'w BotRole>,
    personality: &'w BotPersonality,
    life: &'w BotLife,
    chatter: &'w mut BotChatter,
    memory: Option<&'w mut BotChatMemory>,
}

/// Lets nearby bots hear chat and queue answers.
#[allow(clippy::type_complexity)]
pub fn bot_chat_hear_system(
    mut chat_events: MessageReader<ChatMessageEvent>,
    mut queries: ParamSet<(
        // Characters only: NPC speech is not something players answer.
        Query<
            (
                &Position,
                Option<&BotChatter>,
                Option<&Level>,
                Has<GameClient>,
            ),
            With<CharacterInfo>,
        >,
        Query<
            HearingBotQuery,
            (
                With<BotBuild>,
                Without<LlmBuddyBot>,
                Without<VendorBot>,
                Without<Dead>,
            ),
        >,
    )>,
    query_party: Query<&Party>,
    client_entity_list: Res<ClientEntityList>,
    game_config: Res<GameConfig>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    for event in chat_events.read() {
        if event.chat_type != ChatType::Local && event.chat_type != ChatType::Shout {
            continue;
        }

        // Who said it, where, and how deep into a conversation the line is.
        let Some((sender_position, depth, sender_level, from_player, sender_leveled)) =
            queries.p0().get(event.sender_entity).ok().map(
                |(position, chatter, level, is_player)| {
                    let leveled = chatter.is_some_and(|chatter| {
                        chatter.leveled_at.is_some_and(|leveled_at| {
                            now.saturating_duration_since(leveled_at) < LEVEL_UP_WINDOW
                        })
                    });
                    let depth = chatter
                        .filter(|chatter| {
                            chatter.last_said_at.is_some_and(|said| {
                                now.saturating_duration_since(said) < CONVERSATION_WINDOW
                            })
                        })
                        .map_or(0, |chatter| chatter.last_said_depth);
                    (
                        position.clone(),
                        depth,
                        level.map(|level| level.level),
                        is_player,
                        leveled,
                    )
                },
            )
        else {
            continue;
        };

        let Some(zone_entities) = client_entity_list.get_zone(sender_position.zone_id) else {
            continue;
        };

        // A bot that just levelled up is announcing it, whatever it typed.
        let topic = if sender_leveled {
            BotChatTopic::LevelUp
        } else {
            classify_chat_topic(&event.message)
        };
        let lower = event.message.to_lowercase();
        let sender_first = first_name(&event.sender_name);

        let mut listeners: Vec<(Entity, f32, bool)> = Vec::new();
        {
            let query_bots = queries.p1();
            for (nearby_entity, nearby_position) in zone_entities.iter_entity_type_within_distance(
                sender_position.position.xy(),
                HEAR_RANGE,
                &[ClientEntityType::Character],
            ) {
                if nearby_entity == event.sender_entity {
                    continue;
                }
                let Ok(bot) = query_bots.get(nearby_entity) else {
                    continue;
                };
                let own_name = first_name(&bot.character_info.name).to_lowercase();
                let addressed = !own_name.is_empty() && contains_word(&lower, &own_name);
                let distance = nearby_position.xy().distance(sender_position.position.xy());
                listeners.push((nearby_entity, distance, addressed));
            }
        }
        if listeners.is_empty() {
            continue;
        }

        // Random order, but whoever was addressed answers first.
        listeners.shuffle(&mut rng);
        listeners.sort_by_key(|(_, _, addressed)| !*addressed);

        let mut replies_left = if topic == BotChatTopic::LevelUp { 3 } else { 1 };
        let mut query_bots = queries.p1();

        for (bot_entity, distance, addressed) in listeners {
            let Ok(mut bot) = query_bots.get_mut(bot_entity) else {
                continue;
            };

            if replies_left == 0
                || bot.chatter.pending.is_some()
                || bot.life.activity == BotActivity::Afk
            {
                continue;
            }

            // Players talking to a bot directly always get a hearing;
            // otherwise a bot answers now and then, and never the same
            // person on the same topic twice in a row.
            let direct_from_player = addressed && from_player;
            let on_cooldown = match bot.memory.as_ref() {
                Some(memory) => {
                    (!direct_from_player
                        && memory.last_reply_time.is_some_and(|last| {
                            now.saturating_duration_since(last) < BOT_CHAT_REPLY_COOLDOWN
                        }))
                        || memory.replied_to.iter().any(|(sender, replied_topic, at)| {
                            *sender == event.sender_entity
                                && *replied_topic == topic
                                && now.saturating_duration_since(*at) < SAME_TOPIC_WINDOW
                        })
                }
                None => {
                    !direct_from_player
                        && bot.chatter.last_said_at.is_some_and(|last| {
                            now.saturating_duration_since(last) < BOT_CHAT_REPLY_COOLDOWN
                        })
                }
            };
            if on_cooldown {
                continue;
            }

            let mut chance = reply_chance(topic, addressed, from_player, depth)
                * (0.4 + 0.6 * bot.personality.sociability);
            if !addressed && distance > NEAR_RANGE {
                chance *= 0.6;
            }
            if chance <= 0.0 || !rng.gen_bool(chance.clamp(0.0, 1.0) as f64) {
                continue;
            }

            let party = bot
                .party_membership
                .party
                .and_then(|party_entity| query_party.get(party_entity).ok());
            let in_party = party.is_some();
            let party_full = party.is_some_and(|party| party.members.len() >= 5);
            let can_lead = party.map_or(true, |party| party.owner == bot.entity && !party_full);
            let hunts = matches!(bot.role, Some(BotRole::Fighter | BotRole::Healer) | None);
            let level_close = sender_level.is_some_and(|sender_level| {
                (sender_level as i32 - bot.level.level as i32).abs() <= PARTY_LEVEL_GAP
            });
            let offers_party = topic == BotChatTopic::Party
                && from_player
                && game_config.bot_invite_players
                && hunts
                && can_lead
                && level_close
                && !bot.life.invited_players.contains(&event.sender_entity);

            let ctx = ReplyContext {
                sender_name: sender_first,
                message: &lower,
                level: bot.level.level,
                in_party,
                party_full,
                offers_party,
            };
            let Some(text) = reply_text(&mut rng, topic, &ctx) else {
                continue;
            };

            let say_at = now + typing_delay(&mut rng, text.len());
            bot.chatter.queue_line(PendingLine {
                text,
                say_at,
                depth: depth.saturating_add(1),
                shout: false,
                invite: offers_party.then_some(event.sender_entity),
            });
            if let Some(memory) = bot.memory.as_mut() {
                memory.last_reply_time = Some(now);
                memory
                    .replied_to
                    .push_back((event.sender_entity, topic, now));
                if memory.replied_to.len() > BOT_CHAT_MEMORY_SIZE {
                    memory.replied_to.pop_front();
                }
            }
            replies_left -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addressed_bots_answer_and_chains_die_out() {
        assert!(reply_chance(BotChatTopic::Random, true, true, 0) > 0.8);
        assert_eq!(
            reply_chance(BotChatTopic::Random, true, false, MAX_DEPTH),
            0.0
        );
        assert_eq!(reply_chance(BotChatTopic::Greeting, false, false, 2), 0.0);
        assert_eq!(reply_chance(BotChatTopic::Thanks, false, true, 0), 0.0);
    }
}
