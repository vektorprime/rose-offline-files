//! Vendor advertisements: vendor shop bots periodically shout about their
//! goods to the surrounding area, naming items that are actually in their
//! store with their prices, the way players advertise a stall.

use std::time::{Duration, Instant};

use bevy::prelude::{Commands, Component, Entity, MessageWriter, Query, Res, ResMut, With};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    components::{CharacterInfo, ChatType, ClientEntity, Inventory, PersonalStore},
    events::ChatMessageEvent,
    messages::server::ServerMessage,
    resources::{GameData, ServerMessages},
};

use super::{bot_chat::fill_template, bot_personality::BotPersonality, BotBuild, VendorBot};

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

const ITEM_AD_LINES: &[&str] = &[
    "WTS {item} {price}z, come see my shop!",
    "Selling {item} for {price}z!",
    "{item} only {price}z, today only!",
    "Cheap {item}, {price}z. Stall is right here!",
];

const TWO_ITEM_AD_LINES: &[&str] = &[
    "WTS {item} {price}z, {item2} {price2}z!",
    "Selling {item} ({price}z) and {item2} ({price2}z), come look!",
];

/// Tracks when a vendor bot will shout its next advertisement.
#[derive(Component, Default)]
pub struct BotVendorAdState {
    next_ad_at: Option<Instant>,
}

/// Prices the way players write them: 850, 12k, 1.5m.
fn format_price(price: i64) -> String {
    if price >= 1_000_000 {
        format!("{:.1}m", price as f64 / 1_000_000.0)
    } else if price >= 10_000 {
        format!("{}k", price / 1000)
    } else if price >= 1000 {
        format!("{:.1}k", price as f64 / 1000.0)
    } else {
        price.to_string()
    }
}

/// An advertisement naming up to two items still for sale.
fn compose_ad(
    rng: &mut impl Rng,
    game_data: &GameData,
    store: &PersonalStore,
    inventory: &Inventory,
) -> String {
    let mut listed: Vec<(&str, i64)> = store
        .sell_items
        .iter()
        .flatten()
        .filter_map(|(slot, price)| {
            let item = inventory.get_item(*slot)?;
            let item_data = game_data.items.get_base_item(item.get_item_reference())?;
            (!item_data.name.is_empty()).then_some((item_data.name.as_str(), price.0))
        })
        .collect();
    listed.shuffle(rng);

    match listed.as_slice() {
        [] => generic_ad(rng),
        _ if rng.gen_bool(0.25) => generic_ad(rng),
        [(item, price)] => fill_one(rng, item, *price),
        [(item, price), (item2, price2), ..] if rng.gen_bool(0.4) => {
            let template = TWO_ITEM_AD_LINES.choose(rng).copied().unwrap_or("");
            let price = format_price(*price);
            let price2 = format_price(*price2);
            fill_template(
                template,
                &[
                    ("item", *item),
                    ("price", price.as_str()),
                    ("item2", *item2),
                    ("price2", price2.as_str()),
                ],
            )
        }
        [(item, price), ..] => fill_one(rng, item, *price),
    }
}

fn generic_ad(rng: &mut impl Rng) -> String {
    AD_LINES.choose(rng).copied().unwrap_or("").to_string()
}

fn fill_one(rng: &mut impl Rng, item: &str, price: i64) -> String {
    let template = ITEM_AD_LINES.choose(rng).copied().unwrap_or("");
    let price = format_price(price);
    fill_template(template, &[("item", item), ("price", price.as_str())])
}

#[allow(clippy::type_complexity)]
pub fn bot_vendor_ad_system(
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &ClientEntity,
            &CharacterInfo,
            Option<&PersonalStore>,
            &Inventory,
            Option<&BotPersonality>,
            Option<&mut BotVendorAdState>,
        ),
        (With<BotBuild>, With<VendorBot>),
    >,
    game_data: Res<GameData>,
    mut server_messages: ResMut<ServerMessages>,
    mut chat_message_events: MessageWriter<ChatMessageEvent>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    for (entity, client_entity, character_info, store, inventory, personality, ad_state) in
        query.iter_mut()
    {
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
        let ad = match store {
            Some(store) => compose_ad(&mut rng, &game_data, store, inventory),
            None => generic_ad(&mut rng),
        };
        let ad = match personality {
            Some(personality) => personality.stylize(&mut rng, &ad),
            None => ad,
        };
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

#[cfg(test)]
mod tests {
    use super::format_price;

    #[test]
    fn prices_are_written_like_players_do() {
        assert_eq!(format_price(850), "850");
        assert_eq!(format_price(1500), "1.5k");
        assert_eq!(format_price(12_000), "12k");
        assert_eq!(format_price(2_500_000), "2.5m");
    }
}
