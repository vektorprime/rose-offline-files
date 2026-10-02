use std::time::Duration;

use bevy::ecs::prelude::Component;

use rose_data::{
    CharacterMotionAction, CharacterMotionDatabase, MotionFileData, NpcDatabase, NpcId,
    NpcMotionAction, VehicleMotionAction,
};
use rose_game_common::components::CharacterGender;

/// The parts of a motion file the server uses. Copying these instead of cloning the whole
/// MotionFileData avoids a path allocation per motion for every spawn / motion refresh.
#[derive(Copy, Clone, Debug)]
pub struct MotionTiming {
    pub duration: Duration,
    pub total_attack_frames: usize,
}

impl MotionTiming {
    fn from_motion(motion: &MotionFileData) -> Self {
        Self {
            duration: motion.duration,
            total_attack_frames: motion.total_attack_frames,
        }
    }
}

// Only the motions the server reads are stored (see the get_* accessors and damage_system).
pub struct MotionDataCharacter {
    pub weapon_motion_type: usize,
    pub gender: CharacterGender,
    pub base_vehicle_motion_index: Option<usize>,
    pub attack1: Option<MotionTiming>,
    pub die: Option<MotionTiming>,
    pub hit: Option<MotionTiming>,
    pub pickup_dropped_item: Option<MotionTiming>,
    pub sitting: Option<MotionTiming>,
    pub standup: Option<MotionTiming>,
}

pub struct MotionDataNpc {
    pub npc_id: NpcId,
    pub attack: Option<MotionTiming>,
    pub hit: Option<MotionTiming>,
    pub die: Option<MotionTiming>,
}

#[derive(Component)]
pub enum MotionData {
    Character(MotionDataCharacter),
    Npc(MotionDataNpc),
}

impl MotionData {
    pub fn from_npc(npc_database: &NpcDatabase, npc_id: NpcId) -> Self {
        let get_motion = |action| {
            npc_database
                .get_npc_action_motion(npc_id, action)
                .map(|motion| MotionTiming::from_motion(motion))
        };

        Self::Npc(MotionDataNpc {
            npc_id,
            attack: get_motion(NpcMotionAction::Attack),
            hit: get_motion(NpcMotionAction::Hit),
            die: get_motion(NpcMotionAction::Die),
        })
    }

    pub fn from_character(
        character_motion_database: &CharacterMotionDatabase,
        weapon_motion_type: usize,
        gender: CharacterGender,
    ) -> Self {
        let gender_index = match gender {
            CharacterGender::Male => 0,
            CharacterGender::Female => 1,
        };
        let get_motion = |action| {
            character_motion_database
                .get_character_action_motion(action, weapon_motion_type, gender_index)
                .map(|motion| MotionTiming::from_motion(motion))
        };

        Self::Character(MotionDataCharacter {
            weapon_motion_type,
            gender,
            base_vehicle_motion_index: None,
            attack1: get_motion(CharacterMotionAction::Attack),
            die: get_motion(CharacterMotionAction::Die),
            hit: get_motion(CharacterMotionAction::Hit),
            pickup_dropped_item: get_motion(CharacterMotionAction::Pickitem),
            sitting: get_motion(CharacterMotionAction::Sitting),
            standup: get_motion(CharacterMotionAction::Standup),
        })
    }

    pub fn from_vehicle(
        character_motion_database: &CharacterMotionDatabase,
        base_vehicle_motion_index: usize,
        weapon_motion_type: usize,
    ) -> Self {
        let get_motion = |action| {
            character_motion_database
                .get_vehicle_action_motion(action, base_vehicle_motion_index, weapon_motion_type)
                .map(|motion| MotionTiming::from_motion(motion))
        };

        Self::Character(MotionDataCharacter {
            weapon_motion_type,
            gender: CharacterGender::Male,
            base_vehicle_motion_index: Some(base_vehicle_motion_index),
            attack1: get_motion(VehicleMotionAction::Attack1),
            die: get_motion(VehicleMotionAction::Die),
            hit: get_motion(VehicleMotionAction::Stop),
            pickup_dropped_item: get_motion(VehicleMotionAction::Stop),
            sitting: get_motion(VehicleMotionAction::Stop),
            standup: get_motion(VehicleMotionAction::Stop),
        })
    }

    pub fn get_attack(&self) -> Option<&MotionTiming> {
        match self {
            MotionData::Character(character) => character.attack1.as_ref(),
            MotionData::Npc(npc) => npc.attack.as_ref(),
        }
    }

    pub fn get_die(&self) -> Option<&MotionTiming> {
        match self {
            MotionData::Character(character) => character.die.as_ref(),
            MotionData::Npc(npc) => npc.die.as_ref(),
        }
    }

    pub fn get_pickup_item_drop(&self) -> Option<&MotionTiming> {
        match self {
            MotionData::Character(character) => character.pickup_dropped_item.as_ref(),
            MotionData::Npc(_) => None,
        }
    }

    pub fn get_sit_sitting(&self) -> Option<&MotionTiming> {
        match self {
            MotionData::Character(character) => character.sitting.as_ref(),
            MotionData::Npc(_) => None,
        }
    }

    pub fn get_sit_standing(&self) -> Option<&MotionTiming> {
        match self {
            MotionData::Character(character) => character.standup.as_ref(),
            MotionData::Npc(_) => None,
        }
    }
}
