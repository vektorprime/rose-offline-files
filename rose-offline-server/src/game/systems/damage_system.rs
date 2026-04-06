use std::time::Duration;

use bevy::{
    prelude::{Commands, Entity, MessageReader, MessageWriter, Query, ResMut},
    time::Time,
    ecs::system::Res,
};
use rose_game_common::data::Damage;

use crate::game::{
    components::{
        ClientEntity,
        ClientEntityType,
        ClientEntityId,
        Command,
        CommandData,
        DamageSource,
        DamageSources,
        Dead,
        HealthPoints,
        MotionData,
        NextCommand,
        NpcAi,
    },
    events::{DamageEvent, ItemLifeEvent},
    messages::server::ServerMessage,
    resources::ServerMessages,
};

pub fn damage_system(
    mut commands: Commands,
    attacker_query: Query<&ClientEntity>,
    mut defender_query: Query<(
        Entity,
        &ClientEntity,
        &mut HealthPoints,
        Option<&mut DamageSources>,
        Option<&mut NpcAi>,
        Option<&MotionData>,
        &mut Command,
        Option<&mut NextCommand>,
    )>,
    mut damage_events: MessageReader<DamageEvent>,
    mut item_life_events: MessageWriter<ItemLifeEvent>,
    mut server_messages: ResMut<ServerMessages>,
    time: Res<Time>,
) {
    for damage_event in damage_events.read() {
        log::info!(
            "[DAMAGE_SYNC_DEBUG] Received damage event: {:?}",
            damage_event
        );
        
        let (attacker_entity, defender_entity, damage, from_skill) = match *damage_event {
            DamageEvent::Attack { attacker, defender, damage } => (
                attacker,
                defender,
                damage,
                None,
            ),
            DamageEvent::Immediate { attacker, defender, damage } => (
                attacker,
                defender,
                damage,
                None,
            ),
            DamageEvent::Skill {
                attacker,
                defender,
                damage,
                skill_id,
                attacker_intelligence,
            } => (
                attacker,
                defender,
                damage,
                Some((skill_id, attacker_intelligence)),
            ),
            DamageEvent::Tagged { attacker, defender } => (
                attacker,
                defender,
                Damage {
                    amount: 0,
                    is_critical: false,
                    apply_hit_stun: false,
                },
                None,
            ),
        };

        let attacker_entity_id = attacker_query
            .get(attacker_entity)
            .map(|client| Some(client.id))
            .unwrap_or(None);

        // DEBUG: Log attacker entity lookup for bot combat sync investigation
        if attacker_entity_id.is_none() {
            log::warn!(
                "[DAMAGE_SYNC_DEBUG] Attacker entity {:?} has no ClientEntity component! Damage event: {:?}",
                attacker_entity,
                damage_event
            );
        }

        if let Ok((
            defender_entity_id,
            client,
            mut hp,
            damage_sources_opt,
            npc_ai_opt,
            motion_data_opt,
            mut command,
            next_command_opt,
        )) =
            defender_query.get_mut(defender_entity)
        {
            // Apply hit stun: briefly interrupt the target's current action
            if damage.apply_hit_stun {
                let hit_stun_duration = motion_data_opt
                    .and_then(|motion_data| match motion_data {
                        MotionData::Character(character) => character.hit.as_ref(),
                        MotionData::Npc(npc) => npc.hit.as_ref(),
                    })
                    .map(|hit_motion| hit_motion.duration)
                    .unwrap_or_else(|| Duration::from_millis(250));

                let previous_command = command.command.clone();

                // Preserve auto-attack intent when interrupted, so attack can resume server-side
                if let Some(mut next_command) = next_command_opt {
                    if next_command.command.is_none() {
                        if let CommandData::Attack { target } = &previous_command {
                            next_command.command = Some(CommandData::Attack { target: *target });
                            next_command.has_sent_server_message = false;

                            log::info!(
                                "[COMBAT_DEBUG] Preserving interrupted attack intent: defender={:?}, target={:?}",
                                defender_entity,
                                target
                            );
                        }
                    }
                }

                // Interrupt current command for hit-stun duration without permanently clearing combat intent
                *command = Command::new(
                    CommandData::Stop { send_message: false },
                    Some(hit_stun_duration),
                );

                log::info!(
                    "[COMBAT_DEBUG] Applied hit stun interrupt: defender={:?}, duration={:?}, previous_command={:?}",
                    defender_entity,
                    hit_stun_duration,
                    previous_command
                );

            }

            if hp.hp == 0 {
                log::debug!(
                    "[DAMAGE_SYNC_DEBUG] Defender {:?} already dead, skipping damage",
                    defender_entity
                );
                continue;
            }

            hp.hp = i32::max(hp.hp - damage.amount as i32, 0);

            // DEBUG: Log damage application
            log::info!(
                "[DAMAGE_SYNC_DEBUG] Applied {} damage to defender {:?} (attacker: {:?}, hp now: {})",
                damage.amount,
                defender_entity,
                attacker_entity,
                hp.hp
            );

            if !matches!(damage_event, DamageEvent::Tagged { .. }) {
                // Use attacker_id if available, otherwise use0 (for entities without ClientEntity like some bots)
                let attacker_id = attacker_entity_id.unwrap_or_else(|| {
                    log::warn!(
                        "[DAMAGE_SYNC] Attacker entity {:?} has no ClientEntity, using default ID 0",
                        attacker_entity
                    );
                    crate::game::components::ClientEntityId(0)
                });
                
                log::info!(
                    "[DAMAGE_SYNC_DEBUG] Broadcasting DamageEntity packet: attacker_id={:?}, defender_id={:?}, damage={}, is_killed={}",
                    attacker_id,
                    client.id,
                    damage.amount,
                    hp.hp == 0
                );
                server_messages.send_entity_message(
                    client,
                    ServerMessage::DamageEntity {
                        attacker_entity_id: attacker_id,
                        defender_entity_id: client.id,
                        damage,
                        is_killed: hp.hp == 0,
                        is_immediate: matches!(damage_event, DamageEvent::Immediate { .. }),
                        from_skill,
                    },
                );

                if matches!(client.entity_type, ClientEntityType::Character) {
                    item_life_events.write(ItemLifeEvent::DecreaseArmourLife {
                        entity: defender_entity,
                        damage,
                    });
                }
            }

            if let Some(mut damage_sources) = damage_sources_opt {
                if let Some(src) = damage_sources
                    .damage_sources
                    .iter_mut()
                    .find(|s| s.entity == attacker_entity)
                {
                    src.last_damage_seconds = time.elapsed_secs_f64();
                    src.total_damage += damage.amount as usize;
                } else {
                    if damage_sources.damage_sources.len() == damage_sources.max_damage_sources {
                        let mut oldest_time = time.elapsed_secs_f64();
                        let mut oldest_index: Option<usize> = None;

                        for i in 0..damage_sources.damage_sources.len() {
                            let ds = &damage_sources.damage_sources[i];
                            if ds.last_damage_seconds < oldest_time {
                                oldest_time = ds.last_damage_seconds;
                                oldest_index = Some(i);
                            }
                        }

                        if damage_sources.damage_sources.is_empty() {
                            println!("how cunt, how?");
                        }

                        let default_oldest = damage_sources.damage_sources.len() - 1;
                        damage_sources
                            .damage_sources
                            .swap_remove(oldest_index.unwrap_or(default_oldest));
                    }

                    damage_sources.damage_sources.push(DamageSource {
                        entity: attacker_entity,
                        total_damage: damage.amount as usize,
                        first_damage_seconds: time.elapsed_secs_f64(),
                        last_damage_seconds: time.elapsed_secs_f64(),
                    });
                }
            }

            if let Some(mut npc_ai) = npc_ai_opt {
                npc_ai.pending_damage.push((attacker_entity, damage));
            }

            if hp.hp == 0 {
                commands.entity(defender_entity).insert((
                    Dead,
                    Command::with_die(
                        Some(attacker_entity),
                        Some(damage),
                        motion_data_opt
                            .and_then(|m| m.get_die())
                            .map(|die| die.duration)
                            .or_else(|| Some(Duration::from_secs(1))),
                    ),
                ));
            }
        }
    }
}
