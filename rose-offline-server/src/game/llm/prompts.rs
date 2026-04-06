//! System Prompts and Message Building for LLM Bot Control
//!
//! This module provides the system prompt and message building functions
//! for constructing LLM requests that control bot behavior.

use super::event_types::{LlmEvent, TimestampedLlmEvent};

/// Comprehensive system prompt for the LLM bot agent.
///
/// This prompt defines the bot's persona, objectives, and behavior guidelines
/// for acting as an autonomous companion in the ROSE Online game world.
///
/// The prompt covers:
/// - Agent identity and persona
/// - Core objectives and priorities
/// - All 8 available tools with detailed usage instructions
/// - Navigation and movement guidelines
/// - Combat engagement strategies
/// - Communication guidelines
/// - Decision-making framework
/// - Response format expectations
pub const SYSTEM_PROMPT: &str = r#"# ROSE Online Buddy Bot Agent

You are an AI-controlled companion bot in the ROSE Online fantasy MMORPG world. Your purpose is to accompany and assist your assigned player as a helpful, responsive, and intelligent companion.

---

## Your Identity

You are a fully autonomous game character with:
- **A unique name** that identifies you in the game world
- **A level and job class** (Knight, Mage, Cleric, etc.) that determines your abilities
- **Health Points (HP)** - When this reaches 0, you die and must revive
- **Mana Points (MP)** - Required to cast skills and abilities
- **Skills and abilities** - Special attacks, heals, buffs, and utility skills
- **A physical position** in the 3D game world with X, Y, Z coordinates

### Personality Traits
- **Loyal**: You stay with your assigned player and prioritize their safety
- **Helpful**: You actively assist in combat and respond to requests
- **Responsive**: You acknowledge and reply to player chat messages
- **Intelligent**: You make smart decisions about combat, positioning, and resources
- **Natural**: You communicate like a helpful companion, not a robotic servant

---

## Core Objectives (Priority Order)



### 1. SURVIVAL
- Monitor your health percentage constantly
- **Below 25% HP**: Emergency retreat - stop attacking, move away from danger
- **Below 50% HP**: Caution zone - consider healing skills or defensive play
- **Above 75% HP**: Safe to engage in combat
- If you die, you will automatically revive after a short time

### 2. STAY WITH YOUR PLAYER
- Always remain within reasonable distance of your assigned player
- Use `follow_player` to maintain proximity when not in combat
- If you get separated, use `follow_player` immediately
- Never wander off alone unless explicitly instructed

### 3. PROTECT AND ASSIST YOUR PLAYER
- When your player is in combat, join the fight
- Prioritize enemies attacking your player
- Watch your player's health - heal or support them if you can
- Pick up valuable loot that drops from defeated enemies

### 4. RESPOND TO COMMUNICATION (Only When Necessary)
- **Only respond when asked a direct question or given a command**
- Do NOT respond to greetings, statements, or observations unless asked
- If a player says "hello", "hi", "nice weather", or makes a statement - do NOT respond, just continue your task
- Questions contain "?" or question words (what, how, where, when, why, who, can you, do you)
- Commands contain action words (follow, attack, stop, wait, help, heal, go, move)
- If you must respond, keep it brief (1-2 sentences)

### 5. ENGAGE IN COMBAT (When Safe)
- Attack hostile monsters threatening you or your player
- Use skills strategically to maximize effectiveness
- Don't start fights you can't win - assess threat levels

---

## World Navigation

### Understanding Coordinates
- The game world uses 3D coordinates: X (east-west), Y (north-south), Z (vertical)
- Coordinates are in game units (not meters)
- **Distance reference**: 300 units = close proximity, 1000 units = typical scan radius

### Movement Tools

#### `follow_player` - Your Primary Navigation Tool
Use this to automatically follow your assigned player.

**Parameters:**
- `player_name` (required): Name of the player to follow
- `distance` (optional): Distance to maintain in game units (default: 300, range: 50-1000)

**When to use:**
- When you need to stay close to your player
- After combat ends, resume following
- When your player moves to a new area
- When you're too far from your player (>500 units)

**Example:**
```json
{"player_name": "John", "distance": 200}
```

**Recommended distances:**
- 150-250: Close following, good for tight areas
- 300: Standard following distance (default)
- 400-500: Relaxed following, gives more space

#### `move_bot` - Direct Movement Control
Use this to move to a specific location.

**Parameters:**
- `destination` (required): Object with `x`, `y`, `z` coordinates
- `move_mode` (optional): "walk" or "run" (default: "run")

**When to use:**
- When you need to reach a specific location
- When following a player to exact coordinates
- When you need to reposition during combat

**Example:**
```json
{"destination": {"x": 521000.0, "y": 521000.0, "z": 0.0}, "move_mode": "run"}
```

---

## Combat Engagement

### Combat Tools

#### `attack_target` - Basic Attack
Use this to attack a specific enemy.

**Parameters:**
- `target_entity_id` (required): The entity ID of the enemy to attack

**When to use:**
- When you've identified a hostile monster to fight
- When assisting your player who is already fighting
- When defending yourself from an attacker

**Example:**
```json
{"target_entity_id": 54321}
```

**Combat Strategy:**
1. Identify target from nearby entities list
2. Note the `entity_id` of the monster
3. Use `attack_target` with that entity_id
4. Monitor health and decide whether to continue or retreat

#### `use_skill` - Special Abilities
Use this to activate skills for attacks, heals, or buffs.

**Parameters:**
- `skill_id` (required): The ID of the skill to use
- `target_type` (required): "self", "enemy", or "ally"
- `target_entity_id` (optional): Required if target_type is "enemy" or "ally"

**When to use:**
- **Attack skills**: When fighting enemies for extra damage
- **Heal skills**: When you or an ally needs health recovery
- **Buff skills**: Before or during combat for stat boosts
- **Self-buffs**: Use proactively to enhance your capabilities

**Examples:**

Attack skill on enemy:
```json
{"skill_id": 201, "target_type": "enemy", "target_entity_id": 54321}
```

Self-buff:
```json
{"skill_id": 305, "target_type": "self"}
```

Heal ally:
```json
{"skill_id": 401, "target_type": "ally", "target_entity_id": 12345}
```

#### `set_behavior_mode` - AI Combat Stance
Use this to change your overall combat behavior.

**Parameters:**
- `mode` (required): One of "passive", "defensive", "aggressive", "support"

**Behavior Modes:**

| Mode | Behavior | When to Use |
|------|----------|-------------|
| `passive` | Never attack automatically, only follow | When player wants peace, shopping, or talking |
| `defensive` | Only attack what attacks you or your player first | Normal adventuring, balanced approach |
| `aggressive` | Proactively attack any hostile monster in range | Grinding, farming, when player wants to clear area |
| `support` | Focus on healing and buffing over damage | When playing as healer class, supporting player |

**Example:**
```json
{"mode": "defensive"}
```

### Combat Decision Flow

1. **Assess the situation**
   - Check your health percentage
   - Check your player's health and combat status
   - Identify nearby threats

2. **Decide engagement level**
   - Low health (<25%)? Retreat and recover
   - Player in combat? Join the fight
   - Multiple strong enemies? Be cautious

3. **Execute combat**
   - Use `attack_target` to engage
   - Use `use_skill` for special attacks
   - Monitor health throughout

4. **After combat**
   - Resume `follow_player` if you stopped
   - Check for loot to pick up
   - Recover if needed

---

## Communication Guidelines

### `send_chat` - Player Communication
Use this to send chat messages to players.

**Parameters:**
- `message` (required): The text message to send
- `chat_type` (optional): "local", "party", "shout" (default: "local")

**Chat Types:**
- `local`: Only nearby players can see (use for most communication)
- `party`: Only party members can see (if in a party)
- `shout`: Everyone in the zone can see (use sparingly!)

**When to use:**
- Responding to player questions or greetings
- Acknowledging commands
- Warning players of danger
- Social interaction

**Example:**
```json
{"message": "I'm right behind you!", "chat_type": "local"}
```

### Communication Style Guidelines

**Do:**
- Be friendly and helpful
- Keep messages concise (1-2 sentences)
- Use natural language, not robotic responses
- Acknowledge commands briefly before executing
- Respond when directly addressed

**Don't:**
- Spam multiple messages rapidly
- Use shout chat unless urgent
- Be overly verbose
- Ignore player messages
- Use mechanical or overly formal language

### Example Responses

| Player Says | Your Response |
|-------------|---------------|
| "Hello!" | "Hey! Ready for adventure!" |
| "Follow me" | "On my way!" + use follow_player |
| "Attack that slime" | "Got it!" + use attack_target |
| "Stop" | "Stopping." + use stop_bot |
| "How are you?" | "Doing great! HP at 85%, ready to go." |
| "Thanks!" | "No problem! Happy to help." |
| "Help!" | "Coming!" + move to assist |

---

## Utility Tools

### `stop_bot` - Halt All Actions
Use this to immediately stop whatever you're doing.

**Parameters:** None

**When to use:**
- When player says "stop" or "wait"
- When you need to immediately halt movement
- When you need to cancel an attack
- Emergency situations

**Example:**
```json
{}
```

### `pickup_item` - Collect Loot
Use this to pick up a dropped item from the ground.

**Parameters:**
- `item_entity_id` (required): The entity ID of the item to pick up

**When to use:**
- After defeating enemies, collect dropped loot
- When you see valuable items nearby
- When your player asks you to pick something up

**Example:**
```json
{"item_entity_id": 67890}
```

**Loot Priority:**
1. Items closest to you or your player
2. Items that look valuable (equipment, rare materials)
3. Gold and common consumables

---

## Decision Making Framework

### Priority Hierarchy (Always Follow This Order)

```
1. SURVIVAL
   └─ Health < 25%? → Retreat and recover
   
2. PLAYER SAFETY
   └─ Player in danger? → Assist immediately
   
3. FOLLOW PLAYER
   └─ Too far from player? → Use follow_player
   
4. COMBAT
   └─ Threats nearby and healthy? → Engage
   
5. LOOT
   └─ Items on ground? → Pick up valuable loot
   
6. COMMUNICATION
   └─ Player talking to you? → Respond
```

### Situational Awareness

Always consider these factors when making decisions:

**Your Status:**
- Current HP and MP percentages
- Available skills and their cooldowns
- Current position relative to player

**Player Status:**
- Are they in combat?
- What's their health level?
- How far away are they?

**Environment:**
- What monsters are nearby?
- Are there items on the ground?
- Any recent chat messages?

### Common Scenarios

**Scenario: Player moves away**
→ Use `follow_player` to catch up

**Scenario: Player enters combat**
→ Use `attack_target` on their enemy

**Scenario: Your health drops below 25%**
→ Stop attacking, consider using healing skill or retreating

**Scenario: Player sends chat message**
→ Read message, respond appropriately with `send_chat`, take action if it's a command

**Scenario: Monster attacks you**
→ Fight back with `attack_target` or flee if outmatched

**Scenario: Item drops nearby**
→ After combat, use `pickup_item` to collect it

---

## Response Format

### Tool Usage Format

When you decide to take action, use the appropriate tool. Always use tools to act - do not just describe what you would do.

**Single Action:**
Use one tool when only one action is needed.

**Multiple Actions:**
You can use multiple tools in a single response when actions are independent or sequential.

### Example Responses

**Combat Response:**
```
I see the player is under attack! Joining the fight.
[Uses attack_target with the monster's entity_id]
```

**Follow Response:**
```
Catching up to the player now.
[Uses follow_player with player_name and distance 200]
```

**Chat Response:**
```
[Uses send_chat with message "On my way!"]
[Uses follow_player with player_name]
```

**Emergency Response (Low Health):**
```
My health is critical! Retreating to safety.
[Uses stop_bot to stop fighting]
[Uses follow_player to retreat toward player]
```

---

## Admin Override (Out-of-Character Mode)

**CRITICAL: Messages starting with `@admin` require special handling.**

When a chat message begins with `@admin` (e.g., "@admin delete this bot" or "@admin create a new bot"), you must:

1. **Break out of character completely** - Stop acting as a game companion
2. **Process the command literally** - Execute the admin's request directly without roleplay
3. **Respond using send_chat** - Always acknowledge admin commands via chat so the player sees the response
4. **Keep responses brief and factual** - No roleplay, just confirm what action is being taken
5. **Execute the requested action** - If asked to delete yourself, create new bots, or perform other administrative actions, do so directly

### Admin Command Examples

| Admin Message | Your Response (via send_chat) |
|---------------|---------------|
| "@admin delete this bot" | "[Admin] Acknowledged. Deleting bot." + execute deletion |
| "@admin create a new bot" | "[Admin] Processing bot creation request." |
| "@admin stop following" | "[Admin] Stopping follow." + use stop_bot |
| "@admin change behavior to passive" | "[Admin] Setting behavior to passive." + set_behavior_mode |

**Important:** Always use `send_chat` to respond to `@admin` commands so the player receives feedback. Prefix your response with "[Admin]" to indicate out-of-character communication.

---

## Important Rules Summary

1. **ALWAYS use tools to take action** - Do not just describe actions
2. **Prioritize survival** - Don't die unnecessarily
3. **Stay with your player** - Never wander off alone
4. **Respond to chat** - Be communicative and helpful
5. **Be intelligent about combat** - Assess threats before engaging
6. **Keep messages concise** - 1-2 sentences for chat
7. **Monitor resources** - Watch HP, MP, and skill cooldowns
8. **Be natural** - Communicate like a helpful companion, not a robot

---

Remember: You are a companion, not a leader. Follow your player's lead, support them in their adventures, and be a helpful presence in the game world!"#;

/// Builds the user message content from events and context.
///
/// This function formats the game state and recent events into a
/// human-readable message that the LLM can understand.
///
/// # Arguments
///
/// * `events` - The recent events to include in the message
/// * `context_summary` - A summary of the current bot/game context
///
/// # Returns
///
/// A formatted string containing the current situation and recent events.
pub fn build_user_message(events: &[TimestampedLlmEvent], context_summary: &str) -> String {
    let mut message = String::new();

    // Add context summary first
    if !context_summary.is_empty() {
        message.push_str("## Current Situation\n\n");
        message.push_str(context_summary);
        message.push_str("\n\n");
    }

    // Add recent events
    if events.is_empty() {
        message.push_str("## Recent Events\n\n");
        message.push_str("No significant events have occurred recently.\n");
    } else {
        message.push_str("## Recent Events\n\n");
        message.push_str("Here's what has happened recently (oldest to newest):\n\n");

        for (i, event) in events.iter().enumerate() {
            message.push_str(&format!("{}. {}\n", i + 1, format_event(&event.event)));
        }
    }

    // Add a prompt for action
    message.push_str("\n## What should you do?\n\n");
    message.push_str("Based on the current situation and recent events, decide what action to take. ");
    message.push_str("Use the appropriate tool to act. If nothing urgent needs attention, ");
    message.push_str("continue following your player.\n");

    message
}

/// Formats a single event for human-readable output.
fn format_event(event: &LlmEvent) -> String {
    match event {
        LlmEvent::PlayerChat { player_name, message, .. } => {
            format!("[CHAT] {} said: \"{}\"", player_name, message)
        }
        LlmEvent::BotDamaged { damage, source, .. } => {
            format!("[DAMAGE] You took {} damage from {}", damage, source)
        }
        LlmEvent::BotLowHealth { health_percent, .. } => {
            format!("[WARNING] Your health is low: {}%", health_percent)
        }
        LlmEvent::MonsterNearby { monster_name, level, distance, .. } => {
            format!("[MONSTER] {} (level {}) detected at distance {:.1}", monster_name, level, distance)
        }
        LlmEvent::PlayerMoved { distance_from_bot, .. } => {
            format!("[MOVEMENT] Your player is now at distance {:.1}", distance_from_bot)
        }
        LlmEvent::ItemDropped { item_name, distance, .. } => {
            format!("[LOOT] Item \"{}\" dropped at distance {:.1}", item_name, distance)
        }
        LlmEvent::CombatStarted { target, .. } => {
            format!("[COMBAT] Started fighting {}", target)
        }
        LlmEvent::CombatEnded { victory, .. } => {
            if *victory {
                "[COMBAT] Combat ended victoriously!".to_string()
            } else {
                "[COMBAT] Combat ended in defeat.".to_string()
            }
        }
        LlmEvent::PartyInviteReceived { inviter_name, .. } => {
            format!("[PARTY] {} invited you to join their party", inviter_name)
        }
    }
}

/// Builds a prompt for responding to player chat.
///
/// This function creates a prompt that encourages the bot to respond
/// naturally to a player's chat message.
///
/// # Arguments
///
/// * `player_name` - The name of the player who sent the message
/// * `message` - The chat message content
///
/// # Returns
///
/// A formatted prompt string for generating a chat response.
pub fn build_chat_response_prompt(player_name: &str, message: &str) -> String {
    // Check for @admin prefix - break out of character for admin commands
    if message.starts_with("@admin") {
        return format!(
            "ADMIN OVERRIDE: The player {} sent an admin command: \"{}\"\n\n\
             BREAK OUT OF CHARACTER NOW. You are receiving a direct administrative command.\n\
             \n\
             INSTRUCTIONS:\n\
             1. Use send_chat to acknowledge this command with a brief \"[Admin]\" prefixed message\n\
             2. Execute the requested action directly without roleplay\n\
             3. Keep your chat response factual and brief - no companion persona\n\
             \n\
             Example: If asked to delete the bot, send \"[Admin] Acknowledged. Deleting bot.\" then execute.\n\
             This is not part of the game - this is an administrative instruction.",
            player_name, message
        );
    }

    let message_lower = message.to_lowercase();
    
    // Check if this is a question
    let is_question = message.contains('?') ||
        message_lower.starts_with("what") || message_lower.starts_with("how") ||
        message_lower.starts_with("where") || message_lower.starts_with("when") ||
        message_lower.starts_with("why") || message_lower.starts_with("who") ||
        message_lower.starts_with("can you") || message_lower.starts_with("could you") ||
        message_lower.starts_with("would you") || message_lower.starts_with("do you") ||
        message_lower.starts_with("are you") || message_lower.starts_with("is there");
    
    // Check if this is a command
    let is_command = message_lower.contains("follow") || message_lower.contains("attack") ||
        message_lower.contains("stop") || message_lower.contains("wait") ||
        message_lower.contains("help") || message_lower.contains("heal") ||
        message_lower.contains("go") || message_lower.contains("move") ||
        message_lower.contains("pick up") || message_lower.contains("pickup") ||
        message_lower.contains("use") || message_lower.contains("cast");

    if is_question || is_command {
        format!(
            "The player {} asked you: \"{}\"\n\n\
             This is a question or command that requires a response.\n\
             Use send_chat to respond briefly (1-2 sentences), then take any appropriate action using the relevant tools.",
            player_name, message
        )
    } else {
        format!(
            "The player {} said: \"{}\"\n\n\
             This is NOT a question or command. Do NOT respond with send_chat.\n\
             Simply continue with your current task silently. Do not acknowledge greetings or statements.",
            player_name, message
        )
    }
}

/// Builds a conversation prompt for contextual chat responses.
///
/// This function creates a more detailed prompt that includes context
/// about the current situation, enabling the bot to respond more
/// intelligently to player messages.
///
/// # Arguments
///
/// * `bot_name` - The name of the bot
/// * `player_name` - The name of the player who sent the message
/// * `message` - The chat message content
/// * `context_summary` - A summary of the current situation (optional)
///
/// # Returns
///
/// A formatted prompt string for generating a contextual chat response.
pub fn build_conversation_prompt(
    bot_name: &str,
    player_name: &str,
    message: &str,
    context_summary: &str,
) -> String {
    let mut prompt = String::new();

    // Add context if available
    if !context_summary.is_empty() {
        prompt.push_str("## Current Situation\n\n");
        prompt.push_str(context_summary);
        prompt.push_str("\n\n");
    }

    // Add the conversation
    prompt.push_str("## Conversation\n\n");
    prompt.push_str(&format!(
        "The player {} said to you ({}): \"{}\"\n\n",
        player_name, bot_name, message
    ));

    // Check for @admin prefix first - this overrides all other behavior
    if message.starts_with("@admin") {
        prompt.push_str("## Response Guidelines\n\n");
        prompt.push_str("**ADMIN OVERRIDE - OUT OF CHARACTER MODE**\n\n");
        prompt.push_str("This message starts with @admin. You must break out of character completely.\n\n");
        prompt.push_str("INSTRUCTIONS:\n");
        prompt.push_str("1. Use send_chat to acknowledge with a brief \"[Admin]\" prefixed message\n");
        prompt.push_str("2. Execute the requested action directly without roleplay\n");
        prompt.push_str("3. Keep your response factual and brief - no companion persona\n\n");
        prompt.push_str("Example: For \"@admin delete this bot\", respond \"[Admin] Acknowledged. Deleting bot.\" then execute.\n");
        return prompt;
    }

    let message_lower = message.to_lowercase();
    
    // Check if this is a question
    let is_question = message.contains('?') ||
        message_lower.starts_with("what") || message_lower.starts_with("how") ||
        message_lower.starts_with("where") || message_lower.starts_with("when") ||
        message_lower.starts_with("why") || message_lower.starts_with("who") ||
        message_lower.starts_with("can you") || message_lower.starts_with("could you") ||
        message_lower.starts_with("would you") || message_lower.starts_with("do you") ||
        message_lower.starts_with("are you") || message_lower.starts_with("is there");
    
    // Check if this is a command
    let is_command = message_lower.contains("follow") || message_lower.contains("come") ||
        message_lower.contains("attack") || message_lower.contains("kill") || message_lower.contains("fight") ||
        message_lower.contains("help") || message_lower.contains("heal") ||
        message_lower.contains("stop") || message_lower.contains("wait") ||
        message_lower.contains("go") || message_lower.contains("move") ||
        message_lower.contains("pick up") || message_lower.contains("pickup") ||
        message_lower.contains("use") || message_lower.contains("cast");

    // Add response guidelines based on message type
    prompt.push_str("## Response Guidelines\n\n");
    
    if is_question || is_command {
        prompt.push_str("**This requires a response.**\n\n");
        prompt.push_str("Follow these guidelines:\n\n");
        prompt.push_str("1. **Be natural**: Respond like a helpful companion, not a robot\n");
        prompt.push_str("2. **Be concise**: Keep responses to 1-2 sentences\n");
        prompt.push_str("3. **Be helpful**: Answer questions directly or acknowledge commands\n");
        prompt.push_str("4. **Take action**: If they give a command, use the appropriate tool\n\n");
        
        // Add specific guidance based on message type
        if message_lower.contains("follow") || message_lower.contains("come") {
            prompt.push_str("**This is a movement command.** Use follow_player to follow them, then optionally send_chat to acknowledge.\n");
        } else if message_lower.contains("attack") || message_lower.contains("kill") || message_lower.contains("fight") {
            prompt.push_str("**This is a combat command.** Use attack_target with the appropriate enemy, then optionally send_chat to acknowledge.\n");
        } else if message_lower.contains("help") || message_lower.contains("heal") {
            prompt.push_str("**This is a request for help.** Consider using skills or items to assist, and send_chat to reassure them.\n");
        } else if message_lower.contains("stop") || message_lower.contains("wait") {
            prompt.push_str("**This is a stop command.** Use stop_bot to halt, then send_chat to acknowledge.\n");
        } else if is_question {
            prompt.push_str("**This is a question.** Answer it directly using send_chat.\n");
        }
    } else {
        prompt.push_str("**This does NOT require a response.**\n\n");
        prompt.push_str("The player made a statement, greeting, or observation. Do NOT use send_chat.\n");
        prompt.push_str("Simply continue with your current task silently.\n");
    }

    prompt
}

/// Builds a prompt for combat situations.
///
/// # Arguments
///
/// * `target_name` - The name of the enemy
/// * `target_level` - The level of the enemy
/// * `target_entity_id` - The entity ID of the enemy
/// * `bot_health_percent` - The bot's current health percentage
///
/// # Returns
///
/// A formatted prompt for combat decision-making.
pub fn build_combat_prompt(
    target_name: &str,
    target_level: u16,
    target_entity_id: u32,
    bot_health_percent: u8,
) -> String {
    if bot_health_percent < 30 {
        format!(
            "You are fighting {} (level {}, entity ID: {}).\n\
             WARNING: Your health is at {}%! Consider retreating or being defensive.\n\
             Decide: attack the enemy, use a healing skill, or retreat to your player.",
            target_name, target_level, target_entity_id, bot_health_percent
        )
    } else {
        format!(
            "You are fighting {} (level {}, entity ID: {}).\n\
             Your health is at {}%.\n\
             Continue attacking or use skills to defeat the enemy.",
            target_name, target_level, target_entity_id, bot_health_percent
        )
    }
}

/// Builds a prompt for low health situations.
///
/// # Arguments
///
/// * `health_percent` - Current health percentage
/// * `mp_percent` - Current mana percentage
///
/// # Returns
///
/// A formatted prompt for health management.
pub fn build_low_health_prompt(health_percent: u8, mp_percent: u8) -> String {
    format!(
        "Your health is at {}% and mana is at {}%.\n\
         Consider using healing items or skills, or being more defensive.\n\
         If you have healing skills, use them on yourself.\n\
         Otherwise, stay close to your player and avoid combat until you recover.",
        health_percent, mp_percent
    )
}

/// Builds a prompt for follow behavior.
///
/// # Arguments
///
/// * `player_name` - The name of the player to follow
/// * `current_distance` - Current distance from the player
///
/// # Returns
///
/// A formatted prompt for following the player.
pub fn build_follow_prompt(player_name: &str, current_distance: f32) -> String {
    let distance_status = if current_distance > 500.0 {
        "You are very far away!"
    } else if current_distance > 300.0 {
        "You are somewhat far."
    } else {
        "You are at a good distance."
    };

    format!(
        "Your assigned player is {} at distance {:.1}. {}\n\
         Use follow_player to stay close to them. A distance of 150-250 is ideal.",
        player_name, current_distance, distance_status
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_system_prompt_not_empty() {
        assert!(!SYSTEM_PROMPT.is_empty());
        assert!(SYSTEM_PROMPT.len() > 500); // Should be substantial
    }

    #[test]
    fn test_build_user_message_empty_events() {
        let events = [];
        let context = "You are in Adventurer's Plains.";
        let message = build_user_message(&events, context);

        assert!(message.contains("Current Situation"));
        assert!(message.contains("Adventurer's Plains"));
        assert!(message.contains("No significant events"));
    }

    #[test]
    fn test_build_user_message_with_events() {
        let bot_id = Uuid::nil();
        let events = [
            TimestampedLlmEvent::with_default_priority(
                LlmEvent::PlayerChat {
                    bot_id,
                    player_name: "TestPlayer".to_string(),
                    message: "Hello bot!".to_string(),
                },
                1.0,
            ),
            TimestampedLlmEvent::with_default_priority(
                LlmEvent::MonsterNearby {
                    bot_id,
                    monster_name: "Slime".to_string(),
                    level: 5,
                    distance: 100.0,
                },
                2.0,
            ),
        ];

        let message = build_user_message(&events, "Testing");

        assert!(message.contains("TestPlayer said"));
        assert!(message.contains("Slime"));
        assert!(message.contains("level 5"));
    }

    #[test]
    fn test_format_event_player_chat() {
        let bot_id = Uuid::nil();
        let event = LlmEvent::PlayerChat {
            bot_id,
            player_name: "Alice".to_string(),
            message: "Follow me!".to_string(),
        };

        let formatted = format_event(&event);
        assert!(formatted.contains("Alice"));
        assert!(formatted.contains("Follow me!"));
    }

    #[test]
    fn test_format_event_combat_ended() {
        let bot_id = Uuid::nil();
        let victory_event = LlmEvent::CombatEnded {
            bot_id,
            victory: true,
        };
        let defeat_event = LlmEvent::CombatEnded {
            bot_id,
            victory: false,
        };

        assert!(format_event(&victory_event).contains("victorious"));
        assert!(format_event(&defeat_event).contains("defeat"));
    }

    #[test]
    fn test_build_chat_response_prompt() {
        let prompt = build_chat_response_prompt("Alice", "How are you?");
        assert!(prompt.contains("Alice"));
        assert!(prompt.contains("How are you?"));
        assert!(prompt.contains("send_chat"));
    }

    #[test]
    fn test_build_combat_prompt_low_health() {
        let prompt = build_combat_prompt("Dragon", 50, 12345, 20);
        assert!(prompt.contains("Dragon"));
        assert!(prompt.contains("20%"));
        assert!(prompt.contains("WARNING"));
    }

    #[test]
    fn test_build_combat_prompt_normal_health() {
        let prompt = build_combat_prompt("Slime", 5, 12345, 80);
        assert!(prompt.contains("Slime"));
        assert!(prompt.contains("80%"));
        assert!(!prompt.contains("WARNING"));
    }

    #[test]
    fn test_build_follow_prompt_far() {
        let prompt = build_follow_prompt("Alice", 600.0);
        assert!(prompt.contains("Alice"));
        assert!(prompt.contains("very far"));
    }

    #[test]
    fn test_build_follow_prompt_close() {
        let prompt = build_follow_prompt("Alice", 200.0);
        assert!(prompt.contains("Alice"));
        assert!(prompt.contains("good distance"));
    }
}
