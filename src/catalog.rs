//! Reference catalog of the Voyager Golden Record image sequence: 78 frames
//! per stereo channel, each grayscale ("bnw") or one member of an R/G/B
//! color triplet (three successive frames of the same picture composited as
//! red, green, blue).
//!
//! The triplet groupings are identical in two independent reference
//! decoders (foodini/voyager `voyager.cpp`, MarcBaeuerle/Golden-record-images
//! `src/main.ts`) and consistent with amazing-rando/voyager-decoder's flat
//! triplet index list; the labels are MarcBaeuerle's credits arrays,
//! transcribed verbatim (including source typos).
//!
//! Within a triplet the frame order is **blue, green, red** — verified
//! empirically on this rip (the Sunset and Monument Valley triplets only
//! produce natural colors blue-first). amazing-rando's README agrees;
//! foodini's and MarcBaeuerle's tables label the first member "red", which
//! composites those landmarks with red rock turned teal and blue sky turned
//! pink on our assets.

use crate::audio::WaveformChannel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRole {
    /// Standalone grayscale frame.
    Bnw,
    Red,
    Grn,
    Blu,
}

#[derive(Debug, Clone, Copy)]
pub struct CatalogEntry {
    /// "Title, Credit" as published.
    pub label: &'static str,
    pub color: ColorRole,
}

/// Number of image frames on each stereo channel.
pub const FRAMES_PER_CHANNEL: usize = 78;

/// The reference frame sequence for a channel.
pub fn channel_catalog(channel: WaveformChannel) -> &'static [CatalogEntry; FRAMES_PER_CHANNEL] {
    match channel {
        WaveformChannel::Left => &LEFT,
        WaveformChannel::Right => &RIGHT,
    }
}

/// Frame-index triplets `[red, green, blue]` for a channel, derived from
/// the color roles. The record stores each triplet blue-first, so the red
/// plane is the *last* of the three successive frames.
pub fn color_triplets(channel: WaveformChannel) -> Vec<[usize; 3]> {
    let catalog = channel_catalog(channel);
    let mut triplets = Vec::new();
    let mut i = 0;
    while i < catalog.len() {
        if catalog[i].color == ColorRole::Blu {
            debug_assert!(i + 2 < catalog.len());
            debug_assert_eq!(catalog[i + 1].color, ColorRole::Grn);
            debug_assert_eq!(catalog[i + 2].color, ColorRole::Red);
            triplets.push([i + 2, i + 1, i]);
            i += 3;
        } else {
            i += 1;
        }
    }
    triplets
}

use ColorRole::{Blu, Bnw, Grn, Red};

macro_rules! entry {
    ($label:literal, $color:expr) => {
        CatalogEntry {
            label: $label,
            color: $color,
        }
    };
}

static LEFT: [CatalogEntry; FRAMES_PER_CHANNEL] = [
    entry!("Calibration Circle, Jon Lomberg", Bnw),
    entry!("Location of Our Solar System, Frank Drake", Bnw),
    entry!("Mathematical Definitions, Frank Drake", Bnw),
    entry!("Physical Unit Definitions, Frank Drake", Bnw),
    entry!("The Solar System, Frank Drake", Bnw),
    entry!("The Solar System, Frank Drake", Bnw),
    entry!("The Sun, HALE Observatories", Bnw),
    entry!("Solar Spectrum, Cornell NAIC", Blu),
    entry!("Solar Spectrum, Cornell NAIC", Grn),
    entry!("Solar Spectrum, Cornell NAIC", Red),
    entry!("Mercury, NASA", Bnw),
    entry!("Mars, NASA", Bnw),
    entry!("Jupiter, NASA", Bnw),
    entry!("Home, NASA", Blu),
    entry!("Home, NASA", Grn),
    entry!("Home, NASA", Red),
    entry!("Clouds over Egypt, NASA", Blu),
    entry!("Clouds over Egypt, NASA", Grn),
    entry!("Clouds over Egypt, NASA", Red),
    entry!("DNA Bases, Frank Drake", Bnw),
    entry!("DNA Structure, Jon Lomberg", Bnw),
    entry!("DNA Structure, Jon Lomberg", Bnw),
    entry!("Cell Division, Turtox/Cambosco", Bnw),
    entry!("Human Anatomy, World Book Encyclopedia", Bnw),
    entry!("Human Anatomy, World Book Encyclopedia", Bnw),
    entry!("Human Anatomy, World Book Encyclopedia", Bnw),
    entry!("Human Anatomy, World Book Encyclopedia", Bnw),
    entry!("Human Anatomy, World Book Encyclopedia", Bnw),
    entry!("Human Anatomy, World Book Encyclopedia", Blu),
    entry!("Human Anatomy, World Book Encyclopedia", Grn),
    entry!("Human Anatomy, World Book Encyclopedia", Red),
    entry!("Human Anatomy, World Book Encyclopedia", Bnw),
    entry!("Human Anatomy, World Book Encyclopedia", Bnw),
    entry!("Human Sex Organs, Sinauer Associates Inc.", Bnw),
    entry!("Human Conception, Jon Lomberg", Bnw),
    entry!("Human Conception, Lennart Nilsson", Bnw),
    entry!("Fertilized Ovum, Lennart Nilsson", Bnw),
    entry!("Human Fetus, Jon Lomberg", Bnw),
    entry!("Human Fetus, Dr. Frank Allan", Bnw),
    entry!("Male and Female, Jon Lomberg", Bnw),
    entry!("Birth, Wayne Miller", Bnw),
    entry!("Nursing Mother, UN", Blu),
    entry!("Nursing Mother, UN", Grn),
    entry!("Nursing Mother, UN", Red),
    entry!("Father and Child, Davic Harvey", Blu),
    entry!("Father and Child, Davic Harvey", Grn),
    entry!("Father and Child, Davic Harvey", Red),
    entry!("Group of Children, Ruby Mera/UNICEF", Blu),
    entry!("Group of Children, Ruby Mera/UNICEF", Grn),
    entry!("Group of Children, Ruby Mera/UNICEF", Red),
    entry!("Family Portrait, Jon Lomberg", Bnw),
    entry!("Family Portrait, Nina Leen/Time inc.", Bnw),
    entry!("Continental Drift, Jon Lomberg", Bnw),
    entry!("Stucture of the Earth, Jon Lomberg", Bnw),
    entry!("Heron Island Australia, Jay M. Pasachoff", Bnw),
    entry!("Seashort Maine, Dick Smith", Bnw),
    entry!("Snake River and the Grand Tetons, Ansel Adams", Bnw),
    entry!("Sand Dunes, George F. Mobley", Bnw),
    entry!("Monument Valley, Ray Manley", Blu),
    entry!("Monument Valley, Ray Manley", Grn),
    entry!("Monument Valley, Ray Manley", Red),
    entry!("Forest scene with mushrooms, Bruce Dale", Blu),
    entry!("Forest scene with mushrooms, Bruce Dale", Grn),
    entry!("Forest scene with mushrooms, Bruce Dale", Red),
    entry!("Leaf, Arthur Herrick", Bnw),
    entry!("Fallen leaves, Jodi Cobb", Blu),
    entry!("Fallen leaves, Jodi Cobb", Grn),
    entry!("Fallen leaves, Jodi Cobb", Red),
    entry!("Snowflake over Sequoia, Josef Muench, R. Sisson", Blu),
    entry!("Snowflake over Sequoia, Josef Muench, R. Sisson", Grn),
    entry!("Snowflake over Sequoia, Josef Muench, R. Sisson", Red),
    entry!("Tree with daffodils, Gardens Winterthur", Blu),
    entry!("Tree with daffodils, Gardens Winterthur", Grn),
    entry!("Tree with daffodils, Gardens Winterthur", Red),
    entry!("Flying insect with flowers, Stephen Dalton", Bnw),
    entry!("Evolution of Vertibrates, Jon Lomberg", Bnw),
    entry!("Seashell, Herman Landshoff", Bnw),
    entry!("Dolphines, Thomas Nerbia", Bnw),
];

static RIGHT: [CatalogEntry; FRAMES_PER_CHANNEL] = [
    entry!("School of Fish, David Doubilet", Blu),
    entry!("School of Fish, David Doubilet", Grn),
    entry!("School of Fish, David Doubilet", Red),
    entry!("Tree Toad in Hand, David Wikstrom", Bnw),
    entry!("Crocodile, Peter Beard", Bnw),
    entry!("Eagle, Juan Antonio Fernandez", Bnw),
    entry!("Waterhole, South Africa Tourist Group.", Bnw),
    entry!("Chimp and Scientists, Wanna Goodall", Blu),
    entry!("Chimp and Scientists, Wanna Goodall", Grn),
    entry!("Chimp and Scientists, Wanna Goodall", Red),
    entry!("Bushmen Hunters, Jon Lomberg", Bnw),
    entry!("Bushmen Hunters, R. Farbman", Bnw),
    entry!("Guatemalan Man, UN", Bnw),
    entry!("Dancer from Bali, donna Grosvenor", Bnw),
    entry!("Andean girls, Joseph Scherschel", Bnw),
    entry!("Thai Craftsman, Dean Conger", Bnw),
    entry!("Domesticated ELephant, Peter Kunstadter", Bnw),
    entry!("Man with Glasses, Jonathan Blair", Bnw),
    entry!("Man with Dog, Bruce Baumann", Bnw),
    entry!("Mountain Climber, Gaston Rebuffat", Bnw),
    entry!("Gymnast Cathy Rigbey, Philip Neonian", Bnw),
    entry!("Olympic Sprinters, Picturepoint London", Bnw),
    entry!("Schoolroom, UN", Bnw),
    entry!("Children with Globe, UN", Bnw),
    entry!("Cotton harvest, Howell Walker", Bnw),
    entry!("Grape picker, David Moore", Bnw),
    entry!("Supermarket, Herman Eckelmann", Bnw),
    entry!("Diver with Fish, Jerry Greenberg", Blu),
    entry!("Diver with Fish, Jerry Greenberg", Grn),
    entry!("Diver with Fish, Jerry Greenberg", Red),
    entry!("Fishing Boat, UN", Bnw),
    entry!("Cooking Fish, Brian Seed", Bnw),
    entry!("Chinese Dinner Party, Michael Rougier", Bnw),
    entry!("Licking Eating and Drinking, Hermann Eckelmann", Bnw),
    entry!("The Great Wall of China, Edward Kim", Bnw),
    entry!("House Construction (African), UN", Bnw),
    entry!("Construction scene (Amish country), William Albert Allard", Bnw),
    entry!("House (Africa), UN", Bnw),
    entry!("House (New England), Robert Sisson", Bnw),
    entry!("Modern House (Cloudcroft New Mexico), Frank Drake", Bnw),
    entry!("House interior with artist and fire, Jim Amos", Blu),
    entry!("House interior with artist and fire, Jim Amos", Grn),
    entry!("House interior with artist and fire, Jim Amos", Red),
    entry!("Taj Mahal, David Carroll", Bnw),
    entry!("English city (Oxford), Douglas Gilbert", Bnw),
    entry!("Boston, Ted Spiegel", Bnw),
    entry!("UN Building Day, UN", Bnw),
    entry!("UN Building Night, UN", Blu),
    entry!("UN Building Night, UN", Grn),
    entry!("UN Building Night, UN", Red),
    entry!("Sydney Opera House, Mike Long", Bnw),
    entry!("Artisan with drill, Frank Hewlett", Bnw),
    entry!("Factory interior, Fred Ward", Blu),
    entry!("Factory interior, Fred Ward", Grn),
    entry!("Factory interior, Fred Ward", Red),
    entry!("Science Museum, Davic Cupp", Bnw),
    entry!("X-ray of Hand, Herman Eckelmann", Bnw),
    entry!("Microscope, UN", Bnw),
    entry!("Street Scene (Pakistin), UN", Bnw),
    entry!("Street (India), UN", Bnw),
    entry!("Highway with Trucks, Fred Ward", Bnw),
    entry!("Golden Gate Bridge, Ansel Adams", Bnw),
    entry!("Train, Gordon Gahan", Bnw),
    entry!("Airplane in Flight, Frank Drake", Bnw),
    entry!("Toronto Airport, Lawson Graphics", Bnw),
    entry!("Antartic Sno-Cat, National Geographic Society", Bnw),
    entry!("Radio Telescope, James P. Blair", Bnw),
    entry!("Arecibo Observatory, Herman Eckelmann", Bnw),
    entry!("Page from a Book, Cornell NAIC", Bnw),
    entry!("Astronaut in Space, NASA", Blu),
    entry!("Astronaut in Space, NASA", Grn),
    entry!("Astronaut in Space, NASA", Red),
    entry!("Titan Centaur Launch, NASA", Bnw),
    entry!("Sunset, David Harvey", Blu),
    entry!("Sunset, David Harvey", Grn),
    entry!("Sunset, David Harvey", Red),
    entry!("String Quartet, Philips Recordings", Bnw),
    entry!("Score of Quartet and Violin, Cornell NAIC", Bnw),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_triplets_match_references() {
        // First member of each group (the blue plane) at the reference
        // start indices; red is the last member.
        let starts: Vec<usize> = color_triplets(WaveformChannel::Left).iter().map(|t| t[2]).collect();
        assert_eq!(starts, vec![7, 13, 16, 28, 41, 44, 47, 58, 61, 65, 68, 71]);
        for t in color_triplets(WaveformChannel::Left) {
            assert_eq!(t[0], t[2] + 2, "red plane is the last frame of {t:?}");
            assert_eq!(t[1], t[2] + 1, "green plane is the middle frame of {t:?}");
        }
    }

    #[test]
    fn right_triplets_match_references() {
        let starts: Vec<usize> = color_triplets(WaveformChannel::Right).iter().map(|t| t[2]).collect();
        assert_eq!(starts, vec![0, 7, 27, 40, 47, 52, 69, 73]);
    }

    #[test]
    fn twenty_triplets_total() {
        let n = color_triplets(WaveformChannel::Left).len() + color_triplets(WaveformChannel::Right).len();
        assert_eq!(n, 20);
    }

    #[test]
    fn triplet_members_share_labels() {
        for channel in [WaveformChannel::Left, WaveformChannel::Right] {
            let catalog = channel_catalog(channel);
            for t in color_triplets(channel) {
                assert_eq!(catalog[t[0]].label, catalog[t[1]].label);
                assert_eq!(catalog[t[1]].label, catalog[t[2]].label);
            }
        }
    }

    #[test]
    fn roles_form_complete_triplets() {
        for channel in [WaveformChannel::Left, WaveformChannel::Right] {
            let catalog = channel_catalog(channel);
            let mut i = 0;
            while i < catalog.len() {
                match catalog[i].color {
                    ColorRole::Blu => {
                        assert_eq!(catalog[i + 1].color, ColorRole::Grn, "at {i}");
                        assert_eq!(catalog[i + 2].color, ColorRole::Red, "at {i}");
                        i += 3;
                    }
                    ColorRole::Bnw => i += 1,
                    other => panic!("orphan {other:?} at index {i}"),
                }
            }
        }
    }
}
