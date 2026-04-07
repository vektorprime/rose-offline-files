use std::sync::Arc;
use std::time::Duration;

use bevy::{
    app::ScheduleRunnerPlugin,
    ecs::schedule::IntoScheduleConfigs,
    prelude::{
        App, ApplyDeferred, Last, PluginGroup, PostUpdate, PreUpdate, Startup,
        Update,
    },
    MinimalPlugins,
};
use crossbeam_channel::Receiver;

#[cfg(feature = "llm-feedback")]
use crate::game::llm::{
    LlmClientResource, LlmCommandSenderResource, LlmConfig, LlmEventQueue, LlmFeedbackState,
    llm_event_collector_system, llm_feedback_system, llm_process_responses_system,
    llm_cleanup_stale_responses_system,
};
use crate::game::{
    api::LlmBotManager,
    bots::{BotPlugin, process_llm_bot_creations_system},
    events::{
        BankEvent, ChatCommandEvent, ChatMessageEvent, ClanEvent, DamageEvent, EquipmentEvent, ItemLifeEvent,
        NpcStoreEvent, PartyEvent, PartyMemberEvent, PersonalStoreEvent, PickupItemEvent,
        QuestTriggerEvent, ReviveEvent, RewardItemEvent, RewardXpEvent, SaveEvent, SkillEvent,
        UseAmmoEvent, UseItemEvent,
    },
    messages::control::ControlMessage,
    resources::{
        BotList, ClientEntityList, ControlChannel, EconomyVariables, GameConfig, GameData, LoginTokens, ServerList,
        ServerMessages, WorldRates, WorldTime, WorldVariables, ZoneList,
    },
    systems::{
        ability_values_changed_system, ability_values_update_character_system,
        ability_values_update_npc_system, bank_system, chat_commands_system, clan_system,
        client_entity_visibility_system, command_system, control_server_system, damage_system,
        driving_time_system, equipment_event_system, experience_points_system, expire_time_system,
        game_server_authentication_system, game_server_join_system, game_server_main_system,
        item_life_system, llm_buddy_bot_auto_accept_party_system, llm_buddy_chat_capture_system, llm_buddy_follow_system,
        llm_buddy_status_update_system, llm_bot_teleport_to_player_on_login_system, login_server_authentication_system, login_server_system,
        monster_spawn_system, npc_ai_system, npc_store_system, party_member_event_system,
        party_member_update_info_system, party_system, party_update_average_level_system,
        passive_recovery_system, personal_store_system, pickup_item_system, process_llm_bot_commands_system,
        quest_system, restore_llm_buddy_bots_system, revive_event_system, reward_item_system, 
        save_system, server_messages_system, skill_effect_system, startup_bots_system, startup_clans_system,
        startup_zones_system, status_effect_system, update_character_motion_data_system,
        update_npc_motion_data_system, update_position_system, use_ammo_system, use_item_system, 
        weight_system, world_server_authentication_system, world_server_system, world_time_system, 
        LlmBotCommandReceiver, LlmBotManagerResource,
    },
};

pub struct GameWorld {
    control_rx: Receiver<ControlMessage>,
    llm_bot_manager: Option<LlmBotManager>,
}

impl GameWorld {
    pub fn new(control_rx: Receiver<ControlMessage>) -> Self {
        Self {
            control_rx,
            llm_bot_manager: None,
        }
    }

    /// Create a GameWorld with an LLM Bot Manager for API integration
    pub fn with_llm_bot_manager(control_rx: Receiver<ControlMessage>, llm_bot_manager: LlmBotManager) -> Self {
        Self {
            control_rx,
            llm_bot_manager: Some(llm_bot_manager),
        }
    }

    pub fn run(&mut self, game_config: GameConfig, game_data: GameData) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
            Duration::from_secs_f64(1.0 / 60.0),
        )));
        app.add_plugins(BotPlugin);

        app.insert_resource(BotList::new());
        app.insert_resource(ClientEntityList::new(&game_data.zones));
        app.insert_resource(ControlChannel::new(self.control_rx.clone()));
        app.insert_resource(LoginTokens::new());
        app.insert_resource(ServerList::new());
        app.insert_resource(ServerMessages::new());
        app.insert_resource(WorldRates::new());
        app.insert_resource(WorldTime::new());
        app.insert_resource(WorldVariables::new());
        app.insert_resource(EconomyVariables::new());
        app.insert_resource(ZoneList::new());
        app.insert_resource(game_config);
        app.insert_resource(game_data);

        // Initialize LLM Bot Manager resources if available
        let command_sender = if let Some(manager) = &self.llm_bot_manager {
            // Get the receiver reference first
            let receiver_ref = manager.command_receiver();
            log::info!("GameWorld: Got receiver reference from manager, is_empty: {}", receiver_ref.is_empty());
            
            // Clone the receiver for the resource
            let receiver = receiver_ref.clone();
            log::info!("GameWorld: Cloned receiver for LlmBotCommandReceiver, is_empty: {}", receiver.is_empty());
            
            let bots_map = manager.bots_map();
            log::info!("LlmBotManagerResource using bots map pointer: {:p}", Arc::as_ptr(&bots_map));
            app.insert_resource(LlmBotManagerResource {
                bots_map,
                pending_creates: std::collections::HashMap::new(),
                pending_deletes: Vec::new(),
            });
            app.insert_resource(LlmBotCommandReceiver::new(receiver));
            log::info!("LLM Buddy Bot API integration enabled - process_llm_bot_commands_system will be scheduled in PreUpdate");
            
            // Return the command sender for LLM feedback system
            Some(manager.command_sender())
        } else {
            // Insert default empty resources
            app.insert_resource(LlmBotManagerResource::default());
            // Insert a dummy receiver that will never receive commands
            let (tx, rx) = crossbeam_channel::unbounded();
            drop(tx); // Close the sender so the receiver never gets anything
            app.insert_resource(LlmBotCommandReceiver::new(rx));
            None
        };

        // Initialize LLM Feedback System resources when feature is enabled
        #[cfg(feature = "llm-feedback")]
        {
            // Initialize LLM configuration
            let llm_config = LlmConfig::default();
            
            // Initialize LLM event queue
            app.insert_resource(LlmEventQueue::new());
            
            // Initialize LLM feedback state
            app.insert_resource(LlmFeedbackState::new());
            
            // Initialize LLM client if possible
            match LlmClientResource::new(&llm_config) {
                Ok(client_resource) => {
                    log::info!("LLM Client initialized successfully");
                    app.insert_resource(client_resource);
                }
                Err(e) => {
                    log::warn!("Failed to initialize LLM Client: {:?}", e);
                    // Don't insert the client resource - systems will check for its presence
                }
            }
            
            // Initialize command sender resource if we have a manager
            if let Some(sender) = command_sender {
                app.insert_resource(LlmCommandSenderResource::new(sender));
                log::info!("LLM Command Sender resource initialized");
            }
            
            log::info!("LLM Feedback System resources initialized (feature enabled)");
        }

        app.add_message::<BankEvent>()
            .add_message::<ChatCommandEvent>()
            .add_message::<ChatMessageEvent>()
            .add_message::<ClanEvent>()
            .add_message::<DamageEvent>()
            .add_message::<EquipmentEvent>()
            .add_message::<ItemLifeEvent>()
            .add_message::<NpcStoreEvent>()
            .add_message::<PartyEvent>()
            .add_message::<PartyMemberEvent>()
            .add_message::<PersonalStoreEvent>()
            .add_message::<PickupItemEvent>()
            .add_message::<QuestTriggerEvent>()
            .add_message::<ReviveEvent>()
            .add_message::<RewardItemEvent>()
            .add_message::<RewardXpEvent>()
            .add_message::<SaveEvent>()
            .add_message::<SkillEvent>()
            .add_message::<UseAmmoEvent>()
            .add_message::<UseItemEvent>();

        /*
        Stage order:
        - CoreSet::First
        - CoreSet::PreUpdate
        - GameStages::Input
        - CoreSet::Update
        - CoreSet::PostUpdate
        - CoreSet::Last
        */
        // Note: Bot restoration is now handled when the assigned player logs in
        // via llm_bot_teleport_to_player_on_login_system, not at server startup
        app.add_systems(Startup, (startup_clans_system, startup_bots_system, startup_zones_system));

        app.add_systems(
            PreUpdate,
            (
                (
                    world_time_system,
                    control_server_system,
                    login_server_authentication_system,
                    login_server_system,
                    world_server_authentication_system,
                    world_server_system,
                    game_server_authentication_system,
                    game_server_join_system,
                    (game_server_main_system, revive_event_system).chain(),
                    chat_commands_system,
                    monster_spawn_system,
                    npc_ai_system,
                    expire_time_system,
                    status_effect_system,
                    passive_recovery_system,
                    driving_time_system,
                    // LLM Buddy Bot command processing
                    process_llm_bot_commands_system,
                    process_llm_bot_creations_system,
                ),
                ApplyDeferred,
                (
                    (
                        (
                            update_character_motion_data_system,
                            update_npc_motion_data_system,
                            update_position_system,
                        ),
                        command_system,
                        (use_ammo_system, pickup_item_system),
                    )
                        .chain(),
                    (
                        party_member_event_system,
                        party_system,
                        // LLM buddy bots auto-accept party invites after party_system processes invites
                        llm_buddy_bot_auto_accept_party_system.after(party_system),
                        party_member_update_info_system,
                    )
                        .chain(),
                    clan_system,
                ),
            )
                .chain(),
        );

        app.add_systems(
            Update,
            (
                bank_system,
                personal_store_system,
                npc_store_system,
                quest_system,
                use_item_system,
                reward_item_system,
                damage_system.before(item_life_system),
                skill_effect_system.before(item_life_system),
                item_life_system,
                equipment_event_system.after(item_life_system),
                // LLM Buddy Bot systems
                llm_buddy_follow_system,
                llm_buddy_chat_capture_system,
            ),
        );

        // LLM Feedback Systems (only when feature is enabled)
        #[cfg(feature = "llm-feedback")]
        {
            // Event collector runs in PreUpdate to collect events before feedback processing
            app.add_systems(
                PreUpdate,
                (
                    llm_event_collector_system,
                ),
            );

            // Feedback system runs in Update to process events and query LLM
            app.add_systems(
                Update,
                (
                    llm_feedback_system,
                    llm_process_responses_system,
                    crate::game::systems::llm_bot_admin_command_system,
                ),
            );

            // Cleanup runs in Last to remove stale responses
            app.add_systems(
                Last,
                llm_cleanup_stale_responses_system,
            );

            log::info!("LLM Feedback Systems registered (feature enabled)");
        }

        app.add_systems(
            PostUpdate,
            (
                weight_system,
                experience_points_system,
                party_update_average_level_system.after(experience_points_system),
                client_entity_visibility_system,
                // Teleport LLM bots to their assigned player when player logs in
                llm_bot_teleport_to_player_on_login_system.after(client_entity_visibility_system),
            ),
        );

        app.add_systems(
            Last,
            (
                ability_values_update_character_system.before(ability_values_changed_system),
                ability_values_update_npc_system.before(ability_values_changed_system),
                ability_values_changed_system,
                // LLM Buddy Bot status update
                llm_buddy_status_update_system,
                server_messages_system,
                save_system,
            ),
        );

        app.run();
    }
}
