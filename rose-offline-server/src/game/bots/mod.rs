mod bot_accept_party_invite;
mod bot_attack_target;
mod bot_attack_threat;
mod bot_chat;
mod bot_find_monster_spawn;
mod bot_find_nearby_target;
mod bot_join_zone;
mod bot_pickup_item;
mod bot_revive;
mod bot_send_party_invite;
mod bot_sit_recover_hp;
mod bot_snowball_fight;
mod bot_use_attack_skill;
mod bot_use_buff_skill;
mod bot_use_heal_skill;
mod bot_visit_npc;

mod create_bot;
mod create_llm_buddy_bot;

pub use create_bot::{
    bot_build_artisan, bot_build_bourgeois, bot_build_champion, bot_build_cleric, bot_build_knight,
    bot_build_mage, bot_build_raider, bot_build_scout, bot_create_random_build,
    bot_create_with_build, spend_skill_points, spend_skill_points_with_bundle, spend_stat_points,
    BotBuild,
};
pub use create_llm_buddy_bot::{
    choose_equipment_items, create_llm_buddy_bot, get_bot_build, process_llm_bot_creations_system,
    LlmBuddyBotBuildType, LlmBuddyBotConfig,
};

use bot_accept_party_invite::{
    action_accept_party_invite, score_has_party_invite, AcceptPartyInvite, HasPartyInvite,
};
use bot_attack_target::{
    action_attack_target, score_should_attack_target, ActionAttackTarget, ShouldAttackTarget,
};
use bot_attack_threat::{
    action_attack_threat, score_threat_is_not_target, AttackThreat, ThreatIsNotTarget,
};
use bot_chat::{action_bot_chat_local, score_should_bot_chat, BotChatLocal, ShouldBotChat};
use bot_find_monster_spawn::{action_find_monster_spawn, FindMonsterSpawns};
use bot_find_nearby_target::{
    action_attack_random_nearby_target, score_find_nearby_target, AttackRandomNearbyTarget,
    FindNearbyTarget,
};
use bot_join_zone::{action_join_zone, score_is_teleporting, IsTeleporting, JoinZone};
use bot_pickup_item::{
    action_pickup_nearest_item_drop, score_find_nearby_item_drop_system, FindNearbyItemDrop,
    PickupNearestItemDrop,
};
use bot_revive::{action_revive_current_zone, score_is_dead, IsDead, ReviveCurrentZone};
use bot_send_party_invite::{
    action_party_invite_nearby_bot, score_can_party_invite_nearby_bot, CanPartyInviteNearbyBot,
    PartyInviteNearbyBot,
};
use bot_sit_recover_hp::{
    action_sit_recover_hp, score_should_sit_recover_hp, ShouldSitRecoverHp, SitRecoverHp,
};
use bot_snowball_fight::{action_snowball_fight, SnowballFight};
use bot_use_attack_skill::{
    action_use_attack_skill, score_should_use_attack_skill, ShouldUseAttackSkill, UseAttackSkill,
};
use bot_use_buff_skill::{
    action_use_buff_skill, score_should_use_buff_skill, ShouldUseBuffSkill, UseBuffSkill,
};
use bot_use_heal_skill::{
    action_use_heal_skill, score_should_use_heal_skill, ShouldUseHealSkill, UseHealSkill,
};
use bot_visit_npc::{action_visit_npc, score_should_visit_npc, ShouldVisitNpc, VisitNpc};

use bevy::{
    ecs::schedule::IntoScheduleConfigs,
    prelude::{
        Component, Entity, MessageWriter, Plugin, PreUpdate, Query, Res, Update, With, Without,
    },
};
use big_brain::{
    prelude::Highest,
    thinker::{Thinker, ThinkerBuilder},
    BigBrainPlugin, BigBrainSet,
};
use std::time::Duration;

use crate::game::{
    bundles::SkillListBundle,
    components::{
        AbilityValues, CharacterInfo, ClientEntity, Dead, ExperiencePoints, HealthPoints,
        Inventory, Level, ManaPoints, PartyMembership, SkillList, SkillPoints, Stamina,
        StatPoints, UnionMembership,
    },
    events::PartyEvent,
    resources::GameData,
};

const IDLE_DURATION: Duration = Duration::from_millis(250);

type BotQueryFilterAlive = (With<ClientEntity>, Without<Dead>);
type BotQueryFilterAliveNoTarget = (With<ClientEntity>, Without<Dead>, Without<BotCombatTarget>);

#[derive(Component)]
pub struct BotCombatTarget {
    entity: Entity,
}

#[derive(Clone, Debug)]
pub struct BotBehaviorConfig {
    pub threat_is_not_target_score: f32,
    pub use_attack_skill_score: f32,
    pub attack_target_min_score: f32,
    pub attack_target_max_score: f32,
    pub use_heal_skill_score: f32,
    pub use_heal_skill_min_health_percent: f32,
    pub pickup_item_score: f32,
    pub sit_recover_hp_score: f32,
    pub use_buff_skill_score: f32,
    pub find_nearby_target_score: f32,
    pub chat_score: f32,
    pub chat_chance_per_tick: f32,
    pub visit_npc_score: f32,
    pub visit_npc_chance_per_tick: f32,
}

impl Default for BotBehaviorConfig {
    fn default() -> Self {
        Self {
            threat_is_not_target_score: 0.9,
            use_attack_skill_score: 0.85,
            attack_target_min_score: 0.6,
            attack_target_max_score: 0.8,
            use_heal_skill_score: 0.7,
            use_heal_skill_min_health_percent: 0.45,
            pickup_item_score: 0.5,
            sit_recover_hp_score: 0.4,
            use_buff_skill_score: 0.3,
            find_nearby_target_score: 0.2,
            chat_score: 0.18,
            chat_chance_per_tick: 0.0025,
            visit_npc_score: 0.12,
            visit_npc_chance_per_tick: 0.0015,
        }
    }
}

pub struct BotPlugin;

impl Plugin for BotPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_plugins(BigBrainPlugin::new(PreUpdate)).add_systems(
            Update,
            (
                (
                    action_accept_party_invite,
                    action_attack_random_nearby_target,
                    action_attack_target,
                    action_attack_threat,
                    action_bot_chat_local,
                    action_find_monster_spawn,
                    action_join_zone,
                    action_party_invite_nearby_bot,
                    action_pickup_nearest_item_drop,
                    action_revive_current_zone,
                    action_sit_recover_hp,
                    action_snowball_fight,
                    action_use_attack_skill,
                    action_use_buff_skill,
                    action_use_heal_skill,
                    action_visit_npc,
                )
                    .in_set(BigBrainSet::Actions),
                (
                    score_can_party_invite_nearby_bot,
                    score_find_nearby_item_drop_system,
                    score_find_nearby_target,
                    score_has_party_invite,
                    score_is_dead,
                    score_is_teleporting,
                    score_should_bot_chat,
                    score_should_attack_target,
                    score_should_sit_recover_hp,
                    score_should_use_attack_skill,
                    score_should_use_buff_skill,
                    score_should_use_heal_skill,
                    score_should_visit_npc,
                    score_threat_is_not_target,
                )
                    .in_set(BigBrainSet::Scorers),
                bot_auto_accept_party_invites_system,
                bot_auto_progression_system,
            ),
        );
    }
}

pub fn bot_thinker() -> ThinkerBuilder {
    bot_thinker_with_config(&BotBehaviorConfig::default())
}

pub fn bot_thinker_with_config(config: &BotBehaviorConfig) -> ThinkerBuilder {
    Thinker::build()
        .picker(Highest)
        .when(IsDead { score: 1.0 }, ReviveCurrentZone)
        .when(IsTeleporting { score: 1.0 }, JoinZone)
        .when(HasPartyInvite { score: 1.0 }, AcceptPartyInvite)
        .when(
            ThreatIsNotTarget {
                score: config.threat_is_not_target_score,
            },
            AttackThreat,
        )
        .when(
            ShouldUseAttackSkill {
                score: config.use_attack_skill_score,
            },
            UseAttackSkill,
        )
        .when(
            ShouldAttackTarget {
                min_score: config.attack_target_min_score,
                max_score: config.attack_target_max_score,
            },
            ActionAttackTarget,
        )
        .when(
            ShouldUseHealSkill {
                score: config.use_heal_skill_score,
                min_health_percent: config.use_heal_skill_min_health_percent,
            },
            UseHealSkill,
        )
        .when(
            FindNearbyItemDrop {
                score: config.pickup_item_score,
            },
            PickupNearestItemDrop,
        )
        .when(
            ShouldSitRecoverHp {
                score: config.sit_recover_hp_score,
            },
            SitRecoverHp,
        )
        .when(
            ShouldUseBuffSkill {
                score: config.use_buff_skill_score,
            },
            UseBuffSkill,
        )
        .when(
            ShouldBotChat {
                score: config.chat_score,
                chance_per_tick: config.chat_chance_per_tick,
            },
            BotChatLocal,
        )
        .when(
            ShouldVisitNpc {
                score: config.visit_npc_score,
                chance_per_tick: config.visit_npc_chance_per_tick,
            },
            VisitNpc,
        )
        .when(
            FindNearbyTarget {
                score: config.find_nearby_target_score,
            },
            AttackRandomNearbyTarget,
        )
        .otherwise(FindMonsterSpawns)
}

pub fn bot_snowball_fight() -> ThinkerBuilder {
    Thinker::build()
        .picker(Highest)
        .otherwise(SnowballFight::default())
}

pub fn bot_auto_accept_party_invites_system(
    query: Query<(Entity, &PartyMembership), (With<BotBuild>, Without<Dead>)>,
    mut party_events: MessageWriter<PartyEvent>,
) {
    for (bot_entity, party_membership) in query.iter() {
        if party_membership.party.is_some() {
            continue;
        }

        if let Some(&owner_entity) = party_membership.pending_invites.first() {
            party_events.write(PartyEvent::AcceptInvite {
                owner_entity,
                invited_entity: bot_entity,
            });
        }
    }
}

pub fn bot_auto_progression_system(
    game_data: Res<GameData>,
    mut query: Query<(
        &BotBuild,
        &CharacterInfo,
        &Level,
        &mut StatPoints,
        &mut BasicStats,
        &mut SkillList,
        &mut SkillPoints,
        &mut AbilityValues,
        &mut ExperiencePoints,
        &mut Inventory,
        &mut Stamina,
        &mut UnionMembership,
        &mut HealthPoints,
        &mut ManaPoints,
    )>,
) {
    for (
        bot_build,
        character_info,
        level,
        mut stat_points,
        mut basic_stats,
        mut skill_list,
        mut skill_points,
        mut ability_values,
        mut experience_points,
        mut inventory,
        mut stamina,
        mut union_membership,
        mut health_points,
        mut mana_points,
    ) in query.iter_mut()
    {
        if stat_points.points > 0 {
            spend_stat_points(&game_data, bot_build, &mut stat_points, &mut basic_stats);
        }

        if skill_points.points > 0 {
            let mut skill_list_bundle = SkillListBundle {
                skill_list: &mut skill_list,
                skill_points: Some(&mut skill_points),
                game_client: None,
                ability_values: &mut ability_values,
                level,
                move_speed: None,
                team: None,
                character_info: Some(character_info),
                experience_points: Some(&mut experience_points),
                inventory: Some(&mut inventory),
                stamina: Some(&mut stamina),
                stat_points: Some(&mut stat_points),
                union_membership: Some(&mut union_membership),
                health_points: Some(&mut health_points),
                mana_points: Some(&mut mana_points),
            };

            spend_skill_points_with_bundle(&game_data, bot_build, &mut skill_list_bundle);
        }
    }
}
