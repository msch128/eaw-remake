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

/// What the game is currently doing, as far as music is concerned.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum MusicScene {
    /// Silence.
    #[default]
    None,
    /// Main menu theme.
    MainMenu,
    /// Strategic map of a faction (`"Rebel"`, `"Empire"`, `"Underworld"`).
    Galactic(String),
    /// Battle load screen (`Tactical_Battle_Loading_Loop_Event`; the Remake disables the older
    /// `Galactic_Battle_Pending_*` / `Tactical_Battle_Load_Screen_*` events).
    BattleLoading,
    /// Space battle of a faction; the ambient / battle playlist is chosen from the battle intensity.
    Space(String),
    /// Win or lose stinger.
    Result {
        /// Faction (`"Rebel"`, ...).
        faction: String,
        /// Win or lose.
        win: bool,
        /// Tactical (after a battle) or strategic (campaign end).
        tactical: bool,
    },
    /// Any music event by name.
    Event(String),
}

/// Pure (no Bevy) music selection: scene + battle intensity -> music event name, with the ambient /
/// battle hysteresis of EaW's tactical music (battle music starts when fighting begins and the calm
/// playlist only returns after a quiet spell).
#[derive(Debug, Clone)]
pub struct MusicPlanner {
    scene: MusicScene,
    intensity: f32,
    /// Intensity above which the battle playlist starts.
    pub battle_threshold: f32,
    /// Intensity below which (for [`calm_seconds`](Self::calm_seconds)) the ambient playlist returns.
    pub calm_threshold: f32,
    /// How long the battle must be calm before returning to ambient music.
    pub calm_seconds: f32,
    in_battle: bool,
    calm_for: f32,
}

impl Default for MusicPlanner {
    fn default() -> Self {
        Self {
            scene: MusicScene::None,
            intensity: 0.0,
            battle_threshold: 0.35,
            calm_threshold: 0.1,
            calm_seconds: 25.0,
            in_battle: false,
            calm_for: 0.0,
        }
    }
}

impl MusicPlanner {
    /// Switches the scene (resets the battle state).
    pub fn set_scene(&mut self, scene: MusicScene) {
        if self.scene != scene {
            self.scene = scene;
            self.in_battle = false;
            self.calm_for = 0.0;
        }
    }

    /// Battle intensity in 0..1 (only used in [`MusicScene::Space`]).
    pub fn set_battle_intensity(&mut self, intensity: f32) {
        self.intensity = intensity.clamp(0.0, 1.0);
    }

    /// The scene currently set.
    pub fn scene(&self) -> &MusicScene {
        &self.scene
    }

    /// True while the battle playlist is selected.
    pub fn in_battle(&self) -> bool {
        self.in_battle
    }

    /// Advances the battle / calm hysteresis by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        if !matches!(self.scene, MusicScene::Space(_)) {
            return;
        }
        if self.intensity >= self.battle_threshold {
            self.in_battle = true;
            self.calm_for = 0.0;
        } else if self.in_battle && self.intensity <= self.calm_threshold {
            self.calm_for += dt;
            if self.calm_for >= self.calm_seconds {
                self.in_battle = false;
                self.calm_for = 0.0;
            }
        } else {
            self.calm_for = 0.0;
        }
    }

    /// The music event for the current scene. Unknown factions fall back to Empire.
    pub fn wanted_event(&self, lib: &MusicLibrary) -> Option<String> {
        let faction_kind = |f: &str, make: &dyn Fn(String) -> MusicKind| -> Option<&MusicEvent> {
            lib.by_kind(&make(f.to_string()))
                .or_else(|| lib.by_kind(&make("Empire".into())))
        };
        let named = |n: &str| lib.get(n).map(|e| e.name.clone());
        match &self.scene {
            MusicScene::None => None,
            MusicScene::MainMenu => lib.by_kind(&MusicKind::MainMenu).map(|e| e.name.clone()),
            MusicScene::Galactic(f) => {
                faction_kind(f, &MusicKind::Galactic).map(|e| e.name.clone())
            }
            MusicScene::BattleLoading => named("Tactical_Battle_Loading_Loop_Event")
                .or_else(|| named("Tactical_Battle_Loading_Loop_Event_1")),
            MusicScene::Space(f) => if self.in_battle {
                faction_kind(f, &MusicKind::SpaceBattle)
            } else {
                faction_kind(f, &MusicKind::SpaceAmbient)
            }
            .map(|e| e.name.clone()),
            MusicScene::Result {
                faction,
                win,
                tactical,
            } => lib
                .iter()
                .find(|e| {
                    e.kind
                        == MusicKind::Result {
                            faction: faction.clone(),
                            win: *win,
                            tactical: *tactical,
                        }
                })
                .map(|e| e.name.clone()),
            MusicScene::Event(n) => named(n),
        }
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
