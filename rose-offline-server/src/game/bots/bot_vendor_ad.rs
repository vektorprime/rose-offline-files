//! Vendor advertisements: vendor shop bots periodically shout about their
//! goods in zone chat, so the marketplaces feel alive.

use std::time::{Duration, Instant};

use bevy::prelude::{Commands, Component, Entity, MessageWriter, Query, ResMut, With};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    components::{CharacterInfo, ChatType, ClientEntity},
    events::ChatMessageEvent,
    messages::server::ServerMessage,
    resources::ServerMessages,
};

use super::{BotBuild, VendorBot};

/// Delay between vendor advertisements.
const AD_INTERVAL_MIN: Duration = Duration::from_secs(75);
const AD_INTERVAL_MAX: Duration = Duration::from_secs(135);

const AD_LINES: &[&str] = &[
    "Come check my shop! Fresh stock, fair prices!",
    "New arrivals today! Take a look!",
    "Best prices in town! Don't miss out!",
    "Need gear? I've got what you need!",
    "Special deals today, come see me!",
    "Fresh goods from the hunters! Step right up!",
    "Quality items, honest prices!",
    "Everything must go! ...not really, but come look!",
];

/// Tracks when a vendor bot will shout its next advertisement.
#[derive(Component, Default)]
pub struct BotVendorAdState {
    next_ad_at: Option<Instant>,
}

pub fn bot_vendor_ad_system(
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &ClientEntity,
            &CharacterInfo,
            Option<&mut BotVendorAdState>,
        ),
        (With<BotBuild>, With<VendorBot>),
    >,
    mut server_messages: ResMut<ServerMessages>,
    mut chat_message_events: MessageWriter<ChatMessageEvent>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    for (entity, client_entity, character_info, ad_state) in query.iter_mut() {
        // The first ad is scheduled once and remembered; re-rolling it every
        // tick (state never stored) meant the ad time was never reached.
        let Some(mut ad_state) = ad_state else {
            commands.entity(entity).insert(BotVendorAdState {
                next_ad_at: Some(now + rng.gen_range(AD_INTERVAL_MIN..=AD_INTERVAL_MAX)),
            });
            continue;
        };
        let Some(next_ad_at) = ad_state.next_ad_at else {
            ad_state.next_ad_at = Some(now + rng.gen_range(AD_INTERVAL_MIN..=AD_INTERVAL_MAX));
            continue;
        };

        if now < next_ad_at {
            continue;
        }

        // Shout an advertisement into the surrounding sectors.
        let ad = AD_LINES.choose(&mut rng).unwrap_or(&"").to_string();
        server_messages.send_entity_message(
            client_entity,
            ServerMessage::ShoutChat {
                name: character_info.name.clone(),
                text: ad.clone(),
            },
        );
        chat_message_events.write(ChatMessageEvent {
            sender_entity: entity,
            sender_name: character_info.name.clone(),
            zone_id: client_entity.zone_id,
            message: ad,
            chat_type: ChatType::Shout,
        });

        ad_state.next_ad_at = Some(now + rng.gen_range(AD_INTERVAL_MIN..=AD_INTERVAL_MAX));
    }
}
