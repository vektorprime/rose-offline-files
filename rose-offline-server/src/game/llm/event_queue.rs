//! Event Queue for the LLM Feedback System
//!
//! This module provides the event queue resource that buffers events for each bot
//! before they are sent to the LLM for processing.

use std::collections::{HashMap, VecDeque};

use bevy::prelude::Resource;
use uuid::Uuid;

use super::{EventPriority, LlmEvent, TimestampedLlmEvent};

/// The default maximum number of events per bot queue.
const DEFAULT_MAX_QUEUE_SIZE: usize = 50;

/// Resource holding queued events for each bot.
///
/// This queue buffers events that need to be processed by the LLM feedback system.
/// Events are stored per-bot and can be retrieved when the LLM is ready to process them.
///
/// # Example
///
/// ```ignore
/// use crate::game::llm::{LlmEventQueue, LlmEvent, EventPriority};
/// use uuid::Uuid;
///
/// let mut queue = LlmEventQueue::new();
/// let bot_id = Uuid::new_v4();
///
/// // Push an event
/// queue.push_event(
///     bot_id,
///     LlmEvent::PlayerChat {
///         bot_id,
///         player_name: "Alice".to_string(),
///         message: "Hello!".to_string(),
///     },
///     EventPriority::High,
///     0.0, // timestamp
/// );
///
/// // Check for high priority events
/// if queue.has_high_priority_events(bot_id) {
///     // Get all events (drains the queue)
///     let events = queue.get_events(bot_id);
/// }
/// ```
#[derive(Debug, Clone, Resource)]
pub struct LlmEventQueue {
    /// Events queued per bot
    queues: HashMap<Uuid, VecDeque<TimestampedLlmEvent>>,
    /// Maximum events per bot queue
    max_queue_size: usize,
}

impl Default for LlmEventQueue {
    fn default() -> Self {
        Self {
            queues: HashMap::new(),
            max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
        }
    }
}

impl LlmEventQueue {
    /// Creates a new empty event queue with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new event queue with a custom maximum queue size.
    pub fn with_max_size(max_queue_size: usize) -> Self {
        Self {
            queues: HashMap::new(),
            max_queue_size,
        }
    }

    /// Pushes an event to the queue for the specified bot.
    ///
    /// Events are inserted in priority order (high priority first).
    /// If the queue exceeds `max_queue_size`, the oldest events are removed.
    ///
    /// # Arguments
    ///
    /// * `bot_id` - The UUID of the bot this event belongs to
    /// * `event` - The event to queue
    /// * `priority` - The priority level for this event
    /// * `timestamp` - The game time when this event occurred (in seconds)
    pub fn push_event(
        &mut self,
        bot_id: Uuid,
        event: LlmEvent,
        priority: EventPriority,
        timestamp: f64,
    ) {
        let queue = self.queues.entry(bot_id).or_default();
        
        let timestamped_event = TimestampedLlmEvent::new(event, priority, timestamp);
        
        // Insert sorted by priority (high priority first)
        // Events with the same priority are added at the end of that priority group
        let insert_pos = queue
            .iter()
            .position(|e| e.priority < priority)
            .unwrap_or(queue.len());
        
        queue.insert(insert_pos, timestamped_event);
        
        // Auto-trim old events if queue is full
        while queue.len() > self.max_queue_size {
            queue.pop_front();
        }
    }

    /// Pushes an event with default priority for its type.
    ///
    /// This is a convenience method that uses the event's default priority.
    pub fn push_event_with_default_priority(
        &mut self,
        bot_id: Uuid,
        event: LlmEvent,
        timestamp: f64,
    ) {
        let priority = event.default_priority();
        self.push_event(bot_id, event, priority, timestamp);
    }

    /// Gets all events for a bot and drains the queue.
    ///
    /// Returns a vector of all queued events for the specified bot,
    /// ordered by priority (high first) then by insertion order.
    /// The queue for this bot is cleared after this call.
    ///
    /// # Arguments
    ///
    /// * `bot_id` - The UUID of the bot to get events for
    ///
    /// # Returns
    ///
    /// A vector of timestamped events, which may be empty if no events were queued.
    pub fn get_events(&mut self, bot_id: Uuid) -> Vec<TimestampedLlmEvent> {
        self.queues
            .remove(&bot_id)
            .map(|queue| queue.into_iter().collect())
            .unwrap_or_default()
    }

    /// Gets events for a bot without draining the queue.
    ///
    /// Returns a reference to the events for the specified bot.
    /// The events remain in the queue after this call.
    pub fn peek_events(&self, bot_id: Uuid) -> Option<&VecDeque<TimestampedLlmEvent>> {
        self.queues.get(&bot_id)
    }

    /// Checks if there are any high priority events for a bot.
    ///
    /// This can be used to trigger immediate LLM processing instead of
    /// waiting for the next poll interval.
    ///
    /// # Arguments
    ///
    /// * `bot_id` - The UUID of the bot to check
    ///
    /// # Returns
    ///
    /// `true` if there are any high priority events queued for this bot.
    pub fn has_high_priority_events(&self, bot_id: Uuid) -> bool {
        self.queues
            .get(&bot_id)
            .map(|queue| queue.iter().any(|e| e.priority == EventPriority::High))
            .unwrap_or(false)
    }

    /// Clears all events for a specific bot.
    ///
    /// # Arguments
    ///
    /// * `bot_id` - The UUID of the bot to clear events for
    pub fn clear_events(&mut self, bot_id: Uuid) {
        self.queues.remove(&bot_id);
    }

    /// Clears all events for all bots.
    pub fn clear_all(&mut self) {
        self.queues.clear();
    }

    /// Returns the number of events queued for a specific bot.
    ///
    /// # Arguments
    ///
    /// * `bot_id` - The UUID of the bot to count events for
    pub fn event_count(&self, bot_id: Uuid) -> usize {
        self.queues.get(&bot_id).map(|q| q.len()).unwrap_or(0)
    }

    /// Returns the total number of events across all bots.
    pub fn total_event_count(&self) -> usize {
        self.queues.values().map(|q| q.len()).sum()
    }

    /// Returns the number of bots with queued events.
    pub fn bot_count(&self) -> usize {
        self.queues.len()
    }

    /// Returns the list of bot IDs that have queued events.
    pub fn bots_with_events(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.queues.keys().copied()
    }

    /// Removes events older than the specified age.
    ///
    /// # Arguments
    ///
    /// * `current_time` - The current game time in seconds
    /// * `max_age_secs` - Maximum age of events to keep in seconds
    pub fn trim_old_events(&mut self, current_time: f64, max_age_secs: f64) {
        for queue in self.queues.values_mut() {
            queue.retain(|e| current_time - e.timestamp <= max_age_secs);
        }
        
        // Remove empty queues
        self.queues.retain(|_, queue| !queue.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_event(bot_id: Uuid, message: &str) -> LlmEvent {
        LlmEvent::PlayerChat {
            bot_id,
            player_name: "Test".to_string(),
            message: message.to_string(),
        }
    }

    #[test]
    fn test_new_queue_is_empty() {
        let queue = LlmEventQueue::new();
        let bot_id = Uuid::new_v4();
        assert_eq!(queue.event_count(bot_id), 0);
        assert!(!queue.has_high_priority_events(bot_id));
    }

    #[test]
    fn test_push_and_get_events() {
        let mut queue = LlmEventQueue::new();
        let bot_id = Uuid::new_v4();

        queue.push_event(
            bot_id,
            create_test_event(bot_id, "Hello"),
            EventPriority::Normal,
            1.0,
        );

        assert_eq!(queue.event_count(bot_id), 1);

        let events = queue.get_events(bot_id);
        assert_eq!(events.len(), 1);
        assert_eq!(queue.event_count(bot_id), 0); // Queue is drained
    }

    #[test]
    fn test_priority_ordering() {
        let mut queue = LlmEventQueue::new();
        let bot_id = Uuid::new_v4();

        // Push events in mixed priority order
        queue.push_event(
            bot_id,
            create_test_event(bot_id, "Low"),
            EventPriority::Low,
            1.0,
        );
        queue.push_event(
            bot_id,
            create_test_event(bot_id, "High"),
            EventPriority::High,
            2.0,
        );
        queue.push_event(
            bot_id,
            create_test_event(bot_id, "Normal"),
            EventPriority::Normal,
            3.0,
        );

        let events = queue.get_events(bot_id);
        assert_eq!(events.len(), 3);
        
        // High priority should be first
        assert_eq!(events[0].priority, EventPriority::High);
        assert_eq!(events[1].priority, EventPriority::Normal);
        assert_eq!(events[2].priority, EventPriority::Low);
    }

    #[test]
    fn test_has_high_priority_events() {
        let mut queue = LlmEventQueue::new();
        let bot_id = Uuid::new_v4();

        // No events
        assert!(!queue.has_high_priority_events(bot_id));

        // Low priority event
        queue.push_event(
            bot_id,
            create_test_event(bot_id, "Low"),
            EventPriority::Low,
            1.0,
        );
        assert!(!queue.has_high_priority_events(bot_id));

        // High priority event
        queue.push_event(
            bot_id,
            create_test_event(bot_id, "High"),
            EventPriority::High,
            2.0,
        );
        assert!(queue.has_high_priority_events(bot_id));
    }

    #[test]
    fn test_clear_events() {
        let mut queue = LlmEventQueue::new();
        let bot_id = Uuid::new_v4();

        queue.push_event(
            bot_id,
            create_test_event(bot_id, "Test"),
            EventPriority::Normal,
            1.0,
        );
        assert_eq!(queue.event_count(bot_id), 1);

        queue.clear_events(bot_id);
        assert_eq!(queue.event_count(bot_id), 0);
    }

    #[test]
    fn test_max_queue_size() {
        let mut queue = LlmEventQueue::with_max_size(3);
        let bot_id = Uuid::new_v4();

        // Push more events than the max
        for i in 0..5 {
            queue.push_event(
                bot_id,
                create_test_event(bot_id, &format!("Event {}", i)),
                EventPriority::Normal,
                i as f64,
            );
        }

        let events = queue.get_events(bot_id);
        assert_eq!(events.len(), 3);
        
        // Oldest events should be trimmed
        // Events 0 and 1 should be removed, keeping 2, 3, 4
        let messages: Vec<&str> = events
            .iter()
            .filter_map(|e| {
                if let LlmEvent::PlayerChat { message, .. } = &e.event {
                    Some(message.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(messages, vec!["Event 2", "Event 3", "Event 4"]);
    }

    #[test]
    fn test_trim_old_events() {
        let mut queue = LlmEventQueue::new();
        let bot_id = Uuid::new_v4();

        queue.push_event(
            bot_id,
            create_test_event(bot_id, "Old"),
            EventPriority::Normal,
            1.0,
        );
        queue.push_event(
            bot_id,
            create_test_event(bot_id, "New"),
            EventPriority::Normal,
            10.0,
        );

        // Trim events older than 5 seconds from time 10
        queue.trim_old_events(10.0, 5.0);

        let events = queue.get_events(bot_id);
        assert_eq!(events.len(), 1);
        
        if let LlmEvent::PlayerChat { message, .. } = &events[0].event {
            assert_eq!(message, "New");
        }
    }

    #[test]
    fn test_multiple_bots() {
        let mut queue = LlmEventQueue::new();
        let bot1 = Uuid::new_v4();
        let bot2 = Uuid::new_v4();

        queue.push_event(
            bot1,
            create_test_event(bot1, "Bot1"),
            EventPriority::Normal,
            1.0,
        );
        queue.push_event(
            bot2,
            create_test_event(bot2, "Bot2"),
            EventPriority::Normal,
            1.0,
        );

        assert_eq!(queue.bot_count(), 2);
        assert_eq!(queue.total_event_count(), 2);
        assert_eq!(queue.event_count(bot1), 1);
        assert_eq!(queue.event_count(bot2), 1);

        // Clearing one bot shouldn't affect the other
        queue.clear_events(bot1);
        assert_eq!(queue.bot_count(), 1);
        assert_eq!(queue.event_count(bot2), 1);
    }

    #[test]
    fn test_push_with_default_priority() {
        let mut queue = LlmEventQueue::new();
        let bot_id = Uuid::new_v4();

        // PlayerChat has default High priority
        queue.push_event_with_default_priority(
            bot_id,
            LlmEvent::PlayerChat {
                bot_id,
                player_name: "Test".to_string(),
                message: "Hello".to_string(),
            },
            1.0,
        );

        assert!(queue.has_high_priority_events(bot_id));

        let events = queue.get_events(bot_id);
        assert_eq!(events[0].priority, EventPriority::High);
    }
}
