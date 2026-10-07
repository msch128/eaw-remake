//! Music events (`Xml/MusicEvents.xml`): playlists with fade settings and a typed classification.
//!
//! Files are bare names in `Data/Audio/Music` (`Music.meg`). A `Files` list with several entries is a
//! shuffled playlist; `Loop = Yes` loops the playlist (single files loop the track).

use std::collections::HashMap;

use alamo_data::xml::{self, Warning, parse_bool};
use alamo_fs::FileSource;

/// What a music event is used for, derived from its name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MusicKind {
    /// `Main_Menu_Music_Event`.
    MainMenu,
    /// `Cinematic_*`.
    Cinematic,
    /// `Galactic_Map_<Faction>_Music_Event` (strategic map playlist).
    Galactic(String),
    /// `Space_Map_<Faction>_Ambient_Music_Event` (calm phase of a space battle).
    SpaceAmbient(String),
    /// `Space_Map_<Faction>_Battle_Music_Event` (fighting phase).
    SpaceBattle(String),
    /// `Space_Map_<Faction>_Battle_Death_Star_Music_Event`.
    SpaceBattleDeathStar(String),
    /// `Space_Map_Activate_Death_Star_Music_Event`.
    DeathStarActivate,
    /// `<Terrain>_Land_<Faction>_Ambient_Music_Event`.
    LandAmbient {
        /// Terrain (`Desert`, `Ice`, ...).
        terrain: String,
        /// Faction (`Rebel`, `Empire`).
        faction: String,
    },
    /// `<Terrain>_Land_<Faction>_Battle_Music_Event`.
    LandBattle {
        /// Terrain.
        terrain: String,
        /// Faction.
        faction: String,
    },
    /// `Tactical_Battle_*` loading / go loops.
    TacticalLoop,
    /// `<Faction>_Win_Tactical_Event` / `..._Strategic_Event` / `..._Lose_...` stingers.
    Result {
        /// Faction the stinger is for.
        faction: String,
        /// True for wins.
        win: bool,
        /// True for the tactical (battle) variant, false for strategic.
        tactical: bool,
    },
    /// Anything else (structures, credits, tests ...).
    Other,
}

/// One `MusicEvent`.
#[derive(Debug, Clone)]
pub struct MusicEvent {
    /// Event name.
    pub name: String,
    /// Track file names.
    pub files: Vec<String>,
    /// Volume in percent.
    pub volume_percent: f32,
    /// Fade-in of this event in seconds.
    pub fade_in: f32,
    /// How long the previously playing music fades out, seconds.
    pub fade_out_previous: f32,
    /// `Loop`.
    pub looping: bool,
    /// `MSS_Internal_Loop`: the file contains an intro followed by a loop section.
    pub internal_loop: bool,
    /// Classification.
    pub kind: MusicKind,
}

/// All music events.
#[derive(Debug, Default)]
pub struct MusicLibrary {
    events: HashMap<String, MusicEvent>,
}

impl MusicLibrary {
    /// Loads `Xml/MusicEvents.xml`.
    pub fn load(src: &dyn FileSource, warnings: &mut Vec<Warning>) -> Self {
        let mut events = HashMap::new();
        match src.read("Xml/MusicEvents.xml") {
            Ok(bytes) => {
                if let Some(root) = xml::parse(&bytes, "MusicEvents.xml", warnings) {
                    for e in root.children_named("musicevent") {
                        let Some(name) = e.attr("name") else { continue };
                        let f = |tag: &str, d: f32| {
                            e.child_text(tag)
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(d)
                        };
                        let ev = MusicEvent {
                            name: name.to_string(),
                            files: e
                                .child_text("files")
                                .map(xml::split_list)
                                .unwrap_or_default(),
                            volume_percent: f("volume_percent", 100.0),
                            fade_in: f("fade_in_seconds", 0.0),
                            fade_out_previous: f("fade_out_previous_seconds", 0.0),
                            looping: e.child_text("loop").is_some_and(parse_bool),
                            internal_loop: e
                                .child_text("mss_internal_loop")
                                .is_some_and(parse_bool),
                            kind: classify(name),
                        };
                        events.insert(name.to_ascii_lowercase(), ev);
                    }
                }
            }
            Err(e) => warnings.push(Warning {
                file: "MusicEvents.xml".into(),
                line: 0,
                message: e.to_string(),
            }),
        }
        Self { events }
    }

    /// Looks an event up by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&MusicEvent> {
        self.events.get(&name.to_ascii_lowercase())
    }

    /// First event of this kind.
    pub fn by_kind(&self, kind: &MusicKind) -> Option<&MusicEvent> {
        self.events
            .values()
            .filter(|e| &e.kind == kind)
            .min_by(|a, b| a.name.cmp(&b.name))
    }

    /// Number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True if empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Iterates all events.
    pub fn iter(&self) -> impl Iterator<Item = &MusicEvent> {
        self.events.values()
    }
}

/// Classifies a music event by its name.
pub fn classify(name: &str) -> MusicKind {
    let parts: Vec<&str> = name.split('_').collect();
    let lower: Vec<String> = parts.iter().map(|p| p.to_ascii_lowercase()).collect();
    let has = |s: &str| lower.iter().any(|p| p == s);
    let cap = |s: &str| s.to_string();
    let n = name.to_ascii_lowercase();
    if n == "main_menu_music_event" {
        MusicKind::MainMenu
    } else if n.starts_with("cinematic_") {
        MusicKind::Cinematic
    } else if n.starts_with("galactic_map_") && parts.len() >= 4 {
        MusicKind::Galactic(cap(parts[2]))
    } else if n.starts_with("space_map_activate_death_star") {
        MusicKind::DeathStarActivate
    } else if n.starts_with("space_map_") && has("death") && has("star") {
        MusicKind::SpaceBattleDeathStar(cap(parts[2]))
    } else if n.starts_with("space_map_") && has("ambient") {
        MusicKind::SpaceAmbient(cap(parts[2]))
    } else if n.starts_with("space_map_") && has("battle") {
        MusicKind::SpaceBattle(cap(parts[2]))
    } else if parts.len() >= 4 && lower[1] == "land" && has("ambient") {
        MusicKind::LandAmbient {
            terrain: cap(parts[0]),
            faction: cap(parts[2]),
        }
    } else if parts.len() >= 4 && lower[1] == "land" && has("battle") {
        MusicKind::LandBattle {
            terrain: cap(parts[0]),
            faction: cap(parts[2]),
        }
    } else if n.starts_with("tactical_battle_") {
        MusicKind::TacticalLoop
    } else if (has("win") || has("lose"))
        && (has("tactical") || has("strategic"))
        && parts.len() >= 4
    {
        MusicKind::Result {
            faction: cap(parts[0]),
            win: has("win"),
            tactical: has("tactical"),
        }
    } else {
        MusicKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        assert_eq!(
            classify("Galactic_Map_Rebel_Music_Event"),
            MusicKind::Galactic("Rebel".into())
        );
        assert_eq!(
            classify("Space_Map_Empire_Ambient_Music_Event"),
            MusicKind::SpaceAmbient("Empire".into())
        );
        assert_eq!(
            classify("Space_Map_Empire_Battle_Music_Event"),
            MusicKind::SpaceBattle("Empire".into())
        );
        assert_eq!(
            classify("Space_Map_Rebel_Battle_Death_Star_Music_Event"),
            MusicKind::SpaceBattleDeathStar("Rebel".into())
        );
        assert_eq!(
            classify("Desert_Land_Rebel_Battle_Music_Event"),
            MusicKind::LandBattle {
                terrain: "Desert".into(),
                faction: "Rebel".into()
            }
        );
        assert_eq!(
            classify("Empire_Win_Tactical_Event"),
            MusicKind::Result {
                faction: "Empire".into(),
                win: true,
                tactical: true
            }
        );
        assert_eq!(
            classify("Tactical_Battle_GO_Loop_Event"),
            MusicKind::TacticalLoop
        );
        assert_eq!(classify("Game_Credits_Music_Event"), MusicKind::Other);
    }
}
