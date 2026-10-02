//! Realistic bot name generation.
//!
//! Bots used to be named "Bot_1_0" which looked fake in chat and overhead
//! name tags. This module provides a pool of fantasy-style names in the spirit
//! of the game's NPCs (Tryteh, Myad, Lena, Raffle, Sharlin) and hands out
//! unique names via the `BotNames` resource.

use std::collections::HashSet;

use bevy::prelude::{Deref, DerefMut, Resource};
use rand::Rng;

/// Pool of first names in the style of the game's NPCs.
const BOT_FIRST_NAMES: &[&str] = &[
    "Adrian", "Alaric", "Aldric", "Alysia", "Amara", "Anselm", "Arielle", "Arlen", "Aurora",
    "Bastian", "Belinda", "Bram", "Brenna", "Bryn", "Caelan", "Callista", "Cassian", "Cedric",
    "Cienna", "Corwin", "Daphne", "Darian", "Delia", "Dorian", "Duncan", "Eira", "Elandra",
    "Elian", "Elise", "Ember", "Emmeline", "Ephraim", "Esmé", "Evelyn", "Fenris", "Fiona",
    "Fleur", "Galen", "Gareth", "Giselle", "Gwendolyn", "Hadrian", "Hale", "Hana", "Hazel",
    "Hector", "Helena", "Ilyana", "Ingrid", "Isolde", "Ivor", "Jareth", "Jasmine", "Jasper",
    "Jocelyn", "Julian", "Kael", "Kara", "Kassandra", "Keira", "Kellan", "Kira", "Lachlan",
    "Landon", "Lark", "Lena", "Leona", "Liora", "Loren", "Lucian", "Lyra", "Maeve", "Magnus",
    "Maren", "Marlowe", "Mira", "Morgan", "Nadia", "Nerissa", "Niamh", "Nolan", "Odessa",
    "Ophelia", "Orin", "Pasha", "Petra", "Phoebe", "Quentin", "Rafe", "Rhea", "Rhys", "Riva",
    "Ronan", "Rowan", "Sable", "Saffron", "Saskia", "Seline", "Seraphine", "Shae", "Sienna",
    "Silas", "Soren", "Stellan", "Suri", "Sylvie", "Talia", "Tamsin", "Tarek", "Teagan",
    "Tess", "Thalia", "Theo", "Tobias", "Una", "Valeria", "Vesper", "Vireo", "Vivienne",
    "Wren", "Xander", "Yara", "Yvette", "Zander", "Zara", "Zelda", "Zephyr",
];

/// A "family" name, appended as a suffix to make names more unique.
const BOT_LAST_NAMES: &[&str] = &[
    "Ashvale", "Blackwood", "Bright", "Cinder", "Dawnvale", "Emberlyn", "Fairwind", "Foxglove",
    "Gale", "Halewind", "Ironwood", "Juniper", "Kestrel", "Lake", "Mistborn", "Northwind",
    "Oakheart", "Pine", "Quill", "Raven", "Salt", "Stormborn", "Thornwood", "Vale", "Winters",
    "Yarrow",
];

/// Handles out unique bot names.
#[derive(Resource, Deref, DerefMut, Default)]
pub struct BotNames {
    used: HashSet<String>,
}

impl BotNames {
    /// Returns a unique random bot name, retrying until it has not been used
    /// yet. Falls back to a numbered name if the pools are exhausted.
    pub fn next_unique_name(&mut self, rng: &mut impl Rng) -> String {
        for _ in 0..32 {
            let name = random_bot_name(rng);
            if self.used.insert(name.clone()) {
                return name;
            }
        }

        let fallback = format!("Traveler {}", self.used.len() + 1);
        self.used.insert(fallback.clone());
        fallback
    }
}

/// Generates a random bot name: a first name, sometimes with a family name.
pub fn random_bot_name(rng: &mut impl Rng) -> String {
    let first = BOT_FIRST_NAMES[rng.gen_range(0..BOT_FIRST_NAMES.len())];

    // A quarter of bots get a family name for variety.
    if rng.gen_bool(0.25) {
        let last = BOT_LAST_NAMES[rng.gen_range(0..BOT_LAST_NAMES.len())];
        format!("{} {}", first, last)
    } else {
        first.to_string()
    }
}
