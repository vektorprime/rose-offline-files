use bevy::{
    ecs::query::QueryData,
    prelude::{Component, MessageWriter, Query, ResMut, With},
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    components::{CharacterInfo, ChatType, ClientEntity, Command, Position},
    events::ChatMessageEvent,
    messages::server::ServerMessage,
    resources::ServerMessages,
};

use super::BotQueryFilterAlive;

/// Topics used to tag chat lines so bots can reply appropriately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotChatTopic {
    Greeting,
    Battle,
    Party,
    Trade,
    Help,
    Level,
    Random,
}

pub struct BotChatLine {
    pub text: &'static str,
    pub topic: BotChatTopic,
}

pub const BOT_CHAT_LINES: &[BotChatLine] = &[
    // Greetings
    BotChatLine { text: "Hello there!", topic: BotChatTopic::Greeting },
    BotChatLine { text: "Good day to you!", topic: BotChatTopic::Greeting },
    BotChatLine { text: "Hi! Beautiful weather we're having.", topic: BotChatTopic::Greeting },
    BotChatLine { text: "Hey, nice to meet you.", topic: BotChatTopic::Greeting },
    // Battle
    BotChatLine { text: "That monster never stood a chance.", topic: BotChatTopic::Battle },
    BotChatLine { text: "Patrolling this area.", topic: BotChatTopic::Battle },
    BotChatLine { text: "Keeping the monsters under control.", topic: BotChatTopic::Battle },
    BotChatLine { text: "Stay sharp out here, they're dangerous.", topic: BotChatTopic::Battle },
    BotChatLine { text: "This hunting spot is great for training.", topic: BotChatTopic::Battle },
    BotChatLine { text: "Don't wander too far from the party.", topic: BotChatTopic::Battle },
    // Party
    BotChatLine { text: "Anyone need a party?", topic: BotChatTopic::Party },
    BotChatLine { text: "Need heals?", topic: BotChatTopic::Party },
    BotChatLine { text: "We could use a healer in our party.", topic: BotChatTopic::Party },
    BotChatLine { text: "Let's group up and hunt together!", topic: BotChatTopic::Party },
    BotChatLine { text: "Thanks for the invite!", topic: BotChatTopic::Party },
    // Trade
    BotChatLine { text: "Selling some spare items if anyone's interested.", topic: BotChatTopic::Trade },
    BotChatLine { text: "Looking for a good deal on gear.", topic: BotChatTopic::Trade },
    BotChatLine { text: "Anyone selling potions?", topic: BotChatTopic::Trade },
    // Help
    BotChatLine { text: "Need a hand with anything?", topic: BotChatTopic::Help },
    BotChatLine { text: "I can help you with that quest.", topic: BotChatTopic::Help },
    BotChatLine { text: "Follow me, I know the way.", topic: BotChatTopic::Help },
    // Level
    BotChatLine { text: "I finally leveled up!", topic: BotChatTopic::Level },
    BotChatLine { text: "Training is paying off.", topic: BotChatTopic::Level },
    BotChatLine { text: "Getting stronger every day.", topic: BotChatTopic::Level },
    // Ambience
    BotChatLine { text: "Staying sharp out here.", topic: BotChatTopic::Random },
    BotChatLine { text: "Let's keep moving.", topic: BotChatTopic::Random },
    BotChatLine { text: "What a peaceful spot.", topic: BotChatTopic::Random },
    BotChatLine { text: "I wonder what's over that hill.", topic: BotChatTopic::Random },
    BotChatLine { text: "It's quiet today.", topic: BotChatTopic::Random },
];

/// True when `word` occurs in `text` as a whole word (not inside another word).
fn contains_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let after = text[start + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Classifies a chat message into a topic using simple keyword matching.
pub fn classify_chat_topic(message: &str) -> BotChatTopic {
    let lower = message.to_lowercase();

    if lower.contains("party")
        || lower.contains("group")
        || lower.contains("invite")
        || lower.contains("heal")
        || lower.contains("join")
    {
        BotChatTopic::Party
    } else if lower.contains("help")
        || lower.contains("quest")
        || lower.contains("need a hand")
        || lower.contains("where")
    {
        BotChatTopic::Help
    } else if lower.contains("sell")
        || lower.contains("buy")
        || lower.contains("trade")
        || lower.contains("item")
        || lower.contains("pot")
    {
        BotChatTopic::Trade
    } else if contains_word(&lower, "hi")
        || lower.contains("hello")
        // Whole words: "hi" / "hey" also occur inside "this", "which", "they".
        || contains_word(&lower, "hey")
        || lower.contains("greetings")
    {
        BotChatTopic::Greeting
    } else if lower.contains("level")
        || lower.contains("xp")
        || lower.contains("lv")
        || lower.contains("train")
    {
        BotChatTopic::Level
    } else if lower.contains("fight")
        || lower.contains("attack")
        || lower.contains("kill")
        || lower.contains("hunt")
        || lower.contains("monster")
    {
        BotChatTopic::Battle
    } else {
        BotChatTopic::Random
    }
}

/// Picks a random line from the pool matching `topic`, falling back to any line.
pub fn line_for_topic(rng: &mut impl Rng, topic: BotChatTopic) -> &'static str {
    let matches: Vec<&BotChatLine> = BOT_CHAT_LINES
        .iter()
        .filter(|line| line.topic == topic)
        .collect();

    if matches.is_empty() || rng.gen_bool(0.5) {
        BOT_CHAT_LINES
            .choose(rng)
            .map(|line| line.text)
            .unwrap_or("...")
    } else {
        matches
            .choose(rng)
            .map(|line| line.text)
            .unwrap_or("...")
    }
}

/// Picks a completely random line.
pub fn random_chat_line(rng: &mut impl Rng) -> &'static str {
    BOT_CHAT_LINES
        .choose(rng)
        .map(|line| line.text)
        .unwrap_or("...")
}

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldBotChat {
    pub score: f32,
    pub chance_per_tick: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct BotChatLocal;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    command: &'w Command,
    client_entity: &'w ClientEntity,
    character_info: &'w CharacterInfo,
    position: &'w Position,
}

pub fn score_should_bot_chat(
    mut query: Query<(&ShouldBotChat, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
) {
    let mut rng = rand::thread_rng();

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(bot) = query_bot.get(entity) else {
            continue;
        };

        if !bot.command.is_stop() {
            continue;
        }

        if rng.gen_bool(scorer.chance_per_tick as f64) {
            score.set(scorer.score);
        }
    }
}

pub fn action_bot_chat_local(
    mut query: Query<(&Actor, &mut ActionState), With<BotChatLocal>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    mut server_messages: ResMut<ServerMessages>,
    mut chat_message_events: MessageWriter<ChatMessageEvent>,
) {
    let mut rng = rand::thread_rng();

    for (&Actor(entity), mut state) in query.iter_mut() {
        match *state {
            ActionState::Requested => {
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                let Ok(bot) = query_bot.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let message = random_chat_line(&mut rng).to_string();

                server_messages.send_entity_message(
                    bot.client_entity,
                    ServerMessage::LocalChat {
                        entity_id: bot.client_entity.id,
                        text: message.clone(),
                    },
                );

                chat_message_events.write(ChatMessageEvent {
                    sender_entity: entity,
                    sender_name: bot.character_info.name.clone(),
                    zone_id: bot.position.zone_id,
                    message,
                    chat_type: ChatType::Local,
                });

                *state = ActionState::Success;
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
