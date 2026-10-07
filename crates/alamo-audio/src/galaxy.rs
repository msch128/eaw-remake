//! Galactic-conquest notifications: game event + player faction -> the advisor / HUD SFX event.
//!
//! EaW plays its campaign advisor lines ("Planet captured", "Fleet arrival", ...) as `Is_HUD_VO` SFX events
//! named `<F>HD_<Suffix>` in `SFXEventsHUD.xml` (`EHD_` Empire, `RHD_` Rebel, `UHD_` Underworld; the Remake
//! keeps the scheme). `SpeechEvents.xml` only holds scripted campaign / cinematic lines.

use crate::SfxLibrary;

/// A galactic-map happening that has an advisor line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GalaxyEvent {
    /// A unit finished production.
    UnitBuilt,
    /// A structure / upgrade finished construction.
    StructureBuilt,
    /// The player took a planet.
    PlanetCaptured,
    /// The player lost a planet.
    PlanetLost,
    /// A fleet arrived at its destination.
    FleetArrived,
    /// A battle is waiting to be fought.
    BattlePending,
    /// A battle ended.
    BattleEnded,
    /// A research item finished.
    ResearchComplete,
    /// The tech level went up.
    TechLevelGained,
    /// One of the player's heroes died.
    HeroLost,
    /// An enemy hero was eliminated.
    HeroEliminated,
    /// Population / unit cap reached.
    UnitCapReached,
    /// Credits gained (e.g. from a captured planet).
    CreditsGained,
    /// The fleet retreated successfully.
    RetreatSucceeded,
    /// The player was defeated.
    Defeated,
}

impl GalaxyEvent {
    /// All events, for tests and menus.
    pub const ALL: [GalaxyEvent; 15] = [
        Self::UnitBuilt,
        Self::StructureBuilt,
        Self::PlanetCaptured,
        Self::PlanetLost,
        Self::FleetArrived,
        Self::BattlePending,
        Self::BattleEnded,
        Self::ResearchComplete,
        Self::TechLevelGained,
        Self::HeroLost,
        Self::HeroEliminated,
        Self::UnitCapReached,
        Self::CreditsGained,
        Self::RetreatSucceeded,
        Self::Defeated,
    ];

    /// Event-name suffixes after the faction prefix, in order of preference.
    pub fn suffixes(self) -> &'static [&'static str] {
        match self {
            Self::UnitBuilt => &["New_Units", "Construction_Complete"],
            Self::StructureBuilt => &["Construction_Complete", "New_Construction"],
            Self::PlanetCaptured => &["Planet_Gained"],
            Self::PlanetLost => &["Planet_Lost"],
            Self::FleetArrived => &["Fleet_Arrival"],
            Self::BattlePending => &["Battle_Pending"],
            Self::BattleEnded => &["Battle_End"],
            Self::ResearchComplete => &["Research_Complete", "Tech_Upgrade_Complete"],
            Self::TechLevelGained => &["Gain_Tech_Level", "Tech_Upgrade_Complete"],
            Self::HeroLost => &["Hero_Lost"],
            Self::HeroEliminated => &["Hero_Eliminated"],
            Self::UnitCapReached => &["Unit_Cap_Reached_Strategic", "Unit_Cap_Reached"],
            Self::CreditsGained => &["Credits_Gained"],
            Self::RetreatSucceeded => &["Retreat_Success_Space"],
            Self::Defeated => &["Defeated"],
        }
    }
}

/// HUD event prefix of a faction: `E` Empire, `R` Rebel, `U` Underworld (unknown factions use Empire).
pub fn hud_prefix(faction: &str) -> char {
    match faction.trim().to_ascii_lowercase().as_str() {
        "rebel" | "rebellion" | "rebel_alliance" => 'R',
        "underworld" | "hutts" | "zann_consortium" => 'U',
        _ => 'E',
    }
}

/// The SFX event to play for `event` as seen by `faction`: the first candidate that exists and has samples.
pub fn resolve_galaxy_event(sfx: &SfxLibrary, faction: &str, event: GalaxyEvent) -> Option<String> {
    let prefix = hud_prefix(faction);
    event.suffixes().iter().find_map(|s| {
        let name = format!("{prefix}HD_{s}");
        sfx.get(&name)
            .filter(|e| !e.samples.is_empty())
            .map(|e| e.name.clone())
    })
}
