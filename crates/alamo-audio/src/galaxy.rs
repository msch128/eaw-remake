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
    /// The player ordered a retreat in a battle.
    RetreatStarted,
    /// A hostile fleet arrived at one of the player's planets.
    EnemyFleet,
    /// Reinforcements were called in a battle and are on their way.
    ReinforcementsEnRoute,
    /// A space station of the player is being shot at (`Space_Station_Under_Attack`).
    StationUnderAttack,
    /// One of the player's ships was destroyed.
    UnitDestroyed,
    /// An enemy ship was destroyed.
    EnemyUnitDestroyed,
    /// One of the player's structures (station, hardpoint base) was destroyed.
    StructureDestroyed,
    /// An enemy structure was destroyed.
    EnemyStructureDestroyed,
    /// Reinforcements can be called in the running battle.
    ReinforcementsAvailable,
    /// Enemy ships are leaving the battle.
    EnemyFleeing,
    /// An enemy hero came into sight.
    EnemyHeroDetected,
    /// An enemy reinforcement wave arrived (estimate: `Enemy_Forces` is the closest HUD line).
    EnemyReinforcements,
    /// A unit started production ("Production started").
    ProductionStarted,
    /// A unit production was cancelled ("Production cancelled").
    ProductionCancelled,
    /// A structure started construction ("Building under construction").
    ConstructionStarted,
    /// A structure construction was cancelled ("Building cancelled").
    ConstructionCancelled,
    /// A starbase upgrade started.
    UpgradeStarted,
    /// A starbase upgrade was cancelled.
    UpgradeCancelled,
    /// A tech upgrade started research.
    TechUpgradeStarted,
    /// A tech upgrade was cancelled.
    TechUpgradeCancelled,
    /// A capital ship was lost in battle.
    CapitalShipLost,
    /// An own space station is suffering critical damage (< 25% hull).
    StationCriticalDamage,
    /// An own capital ship or station lost all shield protection.
    ShieldsDown,
}

impl GalaxyEvent {
    /// All events, for tests and menus.
    pub const ALL: [GalaxyEvent; 38] = [
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
        Self::RetreatStarted,
        Self::EnemyFleet,
        Self::ReinforcementsEnRoute,
        Self::StationUnderAttack,
        Self::UnitDestroyed,
        Self::EnemyUnitDestroyed,
        Self::StructureDestroyed,
        Self::EnemyStructureDestroyed,
        Self::ReinforcementsAvailable,
        Self::EnemyFleeing,
        Self::EnemyHeroDetected,
        Self::EnemyReinforcements,
        Self::ProductionStarted,
        Self::ProductionCancelled,
        Self::ConstructionStarted,
        Self::ConstructionCancelled,
        Self::UpgradeStarted,
        Self::UpgradeCancelled,
        Self::TechUpgradeStarted,
        Self::TechUpgradeCancelled,
        Self::CapitalShipLost,
        Self::StationCriticalDamage,
        Self::ShieldsDown,
    ];

    /// Event-name suffixes after the faction prefix, in order of preference.
    pub fn suffixes(self) -> &'static [&'static str] {
        match self {
            Self::UnitBuilt => &["New_Units", "Fleet_Ready", "Construction_Complete"],
            Self::StructureBuilt => &["Construction_Complete", "New_Construction"],
            Self::PlanetCaptured => &["Planet_Gained"],
            Self::PlanetLost => &["Planet_Lost"],
            Self::FleetArrived => &["Fleet_Arrival"],
            Self::BattlePending => &["Battle_Pending"],
            Self::BattleEnded => &["Battle_End"],
            Self::ResearchComplete => &["Research_Complete", "Tech_Upgrade_Complete"],
            Self::TechLevelGained => &["Gain_Tech_Level", "Tech_Upgrade_Complete"],
            Self::HeroLost => &["Hero_Lost"],
            Self::HeroEliminated => &["Enemy_Hero_Eliminated", "Hero_Eliminated"],
            Self::UnitCapReached => &["Unit_Cap_Reached_Strategic", "Unit_Cap_Reached"],
            Self::CreditsGained => &["Credits_Gained"],
            Self::RetreatSucceeded => &["Retreat_Success_Space"],
            Self::Defeated => &["Defeated"],
            Self::RetreatStarted => &["Retreat_Started_Space"],
            Self::EnemyFleet => &["Enemy_Fleet_Approaching", "Enemy_Fleet"],
            Self::ReinforcementsEnRoute => &["Reinforcements_En_Route"],
            Self::StationUnderAttack => &["Space_Station_Under_Attack", "Base_Under_Attack"],
            Self::UnitDestroyed => &["Unit_Destroyed"],
            Self::EnemyUnitDestroyed => &["Enemy_Unit_Destroyed"],
            Self::StructureDestroyed => &["Structure_Destroyed", "Base_Structure_Destroyed"],
            Self::EnemyStructureDestroyed => &["Enemy_Structure_Destroyed"],
            Self::ReinforcementsAvailable => &["Reinforcements_Available"],
            Self::EnemyFleeing => &["Enemy_Fleeing"],
            Self::EnemyHeroDetected => &["Enemy_Hero_Detected"],
            Self::EnemyReinforcements => &["Enemy_Forces", "Enemy_Sighted"],
            Self::ProductionStarted => &["Build_Vehicle", "Build_Troops"],
            Self::ProductionCancelled => &["Unit_Canceled", "Vehicle_Canceled", "Canceled"],
            Self::ConstructionStarted => &["Build_Structure", "New_Construction"],
            Self::ConstructionCancelled => &["Building_Canceled", "Canceled"],
            Self::UpgradeStarted => &["Upgrade_Progress", "Build_Structure"],
            Self::UpgradeCancelled => &["Upgrade_Canceled", "Building_Canceled"],
            Self::TechUpgradeStarted => &["Tech_Upgrade_Progress", "Researching"],
            Self::TechUpgradeCancelled => &["Tech_Upgrade_Canceled", "Research_Canceled"],
            Self::CapitalShipLost => &["Capital_Ship_Destroyed", "Unit_Destroyed"],
            Self::StationCriticalDamage => &["Space_Station_Damaged", "Space_Station_Under_Attack", "Base_Under_Attack"],
            Self::ShieldsDown => &["Shields_Down", "Shields_Depleted", "Shield_Generator_Destroyed"],
        }
    }
}

/// HUD event prefix of a faction: `E` Empire, `R` Rebel (and Republic), `U` Underworld (and CIS), after the faction icons of `FACTIONS.XML` (unknown factions use Empire).
pub fn hud_prefix(faction: &str) -> char {
    match faction.trim().to_ascii_lowercase().as_str() {
        "rebel" | "rebellion" | "rebel_alliance" | "republic" => 'R',
        "underworld" | "hutts" | "zann_consortium" | "cis" => 'U',
        _ => 'E',
    }
}

/// The SFX event to play for `event` as seen by `faction`: the first candidate that exists and has samples.
pub fn resolve_galaxy_event(sfx: &SfxLibrary, faction: &str, event: GalaxyEvent) -> Option<String> {
    let prefix = hud_prefix(faction);
    if let Some(name) = event.suffixes().iter().find_map(|s| {
        let name = format!("{prefix}HD_{s}");
        sfx.get(&name)
            .filter(|e| !e.samples.is_empty())
            .map(|e| e.name.clone())
    }) {
        return Some(name);
    }
    if prefix != 'E' {
        event.suffixes().iter().find_map(|s| {
            let name = format!("EHD_{s}");
            sfx.get(&name)
                .filter(|e| !e.samples.is_empty())
                .map(|e| e.name.clone())
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_and_production_suffixes() {
        assert_eq!(GalaxyEvent::ConstructionStarted.suffixes(), &["Build_Structure", "New_Construction"]);
        assert_eq!(GalaxyEvent::ConstructionCancelled.suffixes(), &["Building_Canceled", "Canceled"]);
        assert_eq!(GalaxyEvent::ProductionStarted.suffixes(), &["Build_Vehicle", "Build_Troops"]);
        assert_eq!(GalaxyEvent::ProductionCancelled.suffixes(), &["Unit_Canceled", "Vehicle_Canceled", "Canceled"]);
        assert_eq!(GalaxyEvent::UpgradeStarted.suffixes(), &["Upgrade_Progress", "Build_Structure"]);
        assert_eq!(GalaxyEvent::UpgradeCancelled.suffixes(), &["Upgrade_Canceled", "Building_Canceled"]);
    }
}
