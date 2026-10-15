//! Display data for the simulation's catalogs: localized names and the galactic map models.

use alamo_data::GameData;
use bevy::prelude::Resource;
use galaxy_sim::{StarbaseTypeId, StructureTypeId, UnitTypeId, World};

/// Names and galactic models of everything the sim knows by index.
///
/// Built once from the game data; without it ([`PlayCatalog::plain`]) names are prettified object
/// ids and no fleet model is known (the map then draws a placeholder).
#[derive(Resource, Debug, Clone, Default)]
pub struct PlayCatalog {
    /// Display name per unit type.
    pub units: Vec<String>,
    /// `Galactic_Fleet_Override_Model_Name` per unit type (the Death Star has its own fleet icon).
    pub unit_models: Vec<Option<String>>,
    /// `Space_Model_Name` (the tactical ship model) per unit type.
    pub unit_space_models: Vec<Option<String>>,
    /// `Icon_Name` per unit type (hero portraits on the map).
    pub unit_icons: Vec<Option<String>>,
    /// `Land_Model_Name` per unit type (how heroes look on the galaxy map).
    pub unit_land_models: Vec<Option<String>>,
    /// Model of a fleet on the galactic map (`Galactic_Fleet` container, `w_planet_fleet.alo`).
    pub generic_fleet_model: Option<String>,
    /// Display name per structure type.
    pub structures: Vec<String>,
    /// Model per structure type (`Land_Model_Name` for ground, `Space_Model_Name` for orbital structures), for the planet view.
    pub structure_models: Vec<Option<String>>,
    /// Display name per star base level.
    pub starbases: Vec<String>,
    /// `Galactic_Model_Name` per star base level (the small station drawn next to the planet).
    pub starbase_models: Vec<Option<String>>,
    /// Display name per faction.
    pub factions: Vec<String>,
    /// The game's wording of the reasons a build item is refused.
    pub rules: RuleTexts,
}

/// The game's own strings for build requirements (`TEXT_REQUIRED_*`, `TEXT_PRODUCTION_*`, `TEXT_BUILD_*_LIMIT_*`), English defaults without the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleTexts {
    /// `TEXT_REQUIRED_TECH_LEVEL` ("Required : Tech Level"), the level follows.
    pub tech: String,
    /// `TEXT_REQUIRED_STAR_BASE_LEVEL` ("Required : Space Station Level"), the level follows.
    pub starbase: String,
    /// `TEXT_REQUIRED_SPECIAL_STRUCTURE` ("Required :"), the structure names follow.
    pub structure: String,
    /// `TEXT_REQUIRED_PLANET` ("Required :"), the planet names follow.
    pub planet: String,
    /// `TEXT_REQUIRED_PLANET_RESTRICTION`.
    pub planet_restriction: String,
    /// `TEXT_PRODUCTION_INSUFFICIENT_FUNDS`.
    pub funds: String,
    /// `TEXT_PRODUCTION_POP_CAP_REACHED`.
    pub pop_cap: String,
    /// `TEXT_PRODUCTION_QUEUE_FULL`.
    pub queue_full: String,
    /// The start of `TEXT_BUILD_1_LIMIT_UN` ("Max Build Limit:"), the limit follows.
    pub limit: String,
}

impl Default for RuleTexts {
    fn default() -> Self {
        Self {
            tech: "Required : Tech Level".into(),
            starbase: "Required : Space Station Level".into(),
            structure: "Required :".into(),
            planet: "Required :".into(),
            planet_restriction: "Cannot be built on this planet".into(),
            funds: "Insufficient funds".into(),
            pop_cap: "Population cap reached".into(),
            queue_full: "Production queue full".into(),
            limit: "Max Build Limit:".into(),
        }
    }
}

impl RuleTexts {
    /// The strings of the loaded game data; missing keys keep the English default.
    pub fn from_data(data: &GameData) -> Self {
        let d = Self::default();
        let get = |key: &str, default: String| data.text(key).map(str::trim).filter(|t| !t.is_empty()).map_or(default, str::to_owned);
        let limit = data.text("TEXT_BUILD_1_LIMIT_UN").and_then(|t| t.split(':').next()).map_or(d.limit.clone(), |p| format!("{}:", p.trim()));
        Self {
            tech: get("TEXT_REQUIRED_TECH_LEVEL", d.tech),
            starbase: get("TEXT_REQUIRED_STAR_BASE_LEVEL", d.starbase),
            structure: get("TEXT_REQUIRED_SPECIAL_STRUCTURE", d.structure),
            planet: get("TEXT_REQUIRED_PLANET", d.planet),
            planet_restriction: get("TEXT_REQUIRED_PLANET_RESTRICTION", d.planet_restriction),
            funds: get("TEXT_PRODUCTION_INSUFFICIENT_FUNDS", d.funds),
            pop_cap: get("TEXT_PRODUCTION_POP_CAP_REACHED", d.pop_cap),
            queue_full: get("TEXT_PRODUCTION_QUEUE_FULL", d.queue_full),
            limit,
        }
    }
}

/// Which model stands for a fleet on the galactic map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FleetModelStyle {
    /// The game's own look: one generic fleet model (`w_planet_fleet.alo`), tinted in the owner's
    /// color, except fleets with a unit that overrides it (Death Star).
    Generic,
    /// The tactical model of the fleet's strongest ship, shrunk to icon size (the default: an ISD reads as an ISD).
    #[default]
    Flagship,
}

/// `Star_Destroyer_Imperial` -> `Star Destroyer Imperial`.
pub fn prettify(object: &str) -> String {
    object.replace('_', " ")
}

impl PlayCatalog {
    /// Catalog with prettified object names only.
    pub fn plain(world: &World) -> Self {
        Self {
            units: world.units.iter().map(|u| prettify(&u.name)).collect(),
            unit_models: vec![None; world.units.len()],
            unit_space_models: vec![None; world.units.len()],
            unit_icons: vec![None; world.units.len()],
            unit_land_models: vec![None; world.units.len()],
            generic_fleet_model: None,
            structures: world.structures.iter().map(|s| prettify(&s.name)).collect(),
            structure_models: vec![None; world.structures.len()],
            starbases: world.starbases.iter().map(|s| prettify(&s.name)).collect(),
            factions: world.factions.iter().map(|f| prettify(&f.name)).collect(),
            starbase_models: vec![None; world.starbases.len()],
            rules: RuleTexts::default(),
        }
    }

    /// Catalog with English names from the string table and models from the unit definitions.
    pub fn from_data(world: &World, data: &GameData) -> Self {
        let text = |id: &Option<String>, fallback: &str| -> String {
            id.as_deref().and_then(|k| data.text(k)).filter(|t| !t.is_empty()).map_or_else(|| prettify(fallback), str::to_owned)
        };
        let mut units = Vec::with_capacity(world.units.len());
        let mut unit_models = Vec::with_capacity(world.units.len());
        let mut unit_space_models = Vec::with_capacity(world.units.len());
        let mut unit_icons = Vec::with_capacity(world.units.len());
        let mut unit_land_models = Vec::with_capacity(world.units.len());
        for u in &world.units {
            units.push(text(&u.text_id, &u.name));
            let r = data.objects.resolve(&u.name);
            let get = |tag: &str| r.as_ref().and_then(|r| r.string(tag)).filter(|m| !m.is_empty() && !m.to_ascii_lowercase().contains("invisible"));
            unit_models.push(get("galactic_fleet_override_model_name"));
            let space_model = get("space_model_name")
                .or_else(|| get("galactic_model_name"))
                .or_else(|| {
                    r.as_ref().and_then(|mr| {
                        let members = mr.list("squadron_units");
                        members.into_iter().find_map(|m| {
                            data.objects.resolve(&m)
                                .and_then(|sm| sm.string("space_model_name").or_else(|| sm.string("galactic_model_name")))
                                .filter(|m| !m.is_empty() && !m.to_ascii_lowercase().contains("invisible"))
                        })
                    })
                });
            unit_space_models.push(space_model);
            unit_icons.push(r.as_ref().and_then(|r| r.string("icon_name")).filter(|n| !n.trim().is_empty()));
            unit_land_models.push(r.as_ref().and_then(|r| r.string("land_model_name")).filter(|n| !n.trim().is_empty()));
        }
        let generic_fleet_model = data
            .objects
            .resolve("Galactic_Fleet")
            .and_then(|r| r.string("galactic_model_name"))
            .filter(|m| !m.is_empty() && !m.eq_ignore_ascii_case("w_planet_fleet.alo"));
        Self {
            units,
            unit_models,
            unit_space_models,
            unit_icons,
            unit_land_models,
            generic_fleet_model,
            structures: world.structures.iter().map(|s| text(&s.text_id, &s.name)).collect(),
            structure_models: world
                .structures
                .iter()
                .map(|s| {
                    let r = data.objects.resolve(&s.name)?;
                    let key = if s.orbital { "space_model_name" } else { "land_model_name" };
                    r.string(key).or_else(|| r.string("space_model_name")).filter(|m| !m.trim().is_empty())
                })
                .collect(),
            starbases: world
                .starbases
                .iter()
                .map(|s| text(&data.objects.star_base(&s.name).and_then(|d| d.text_id), &s.name))
                .collect(),
            starbase_models: world
                .starbases
                .iter()
                .map(|s| data.objects.resolve(&s.name).and_then(|r| r.string("galactic_model_name")).filter(|m| !m.trim().is_empty()))
                .collect(),
            factions: world
                .factions
                .iter()
                .map(|f| text(&data.faction(&f.name).and_then(|d| d.text_id.clone()), &f.name))
                .collect(),
            rules: RuleTexts::from_data(data),
        }
    }

    /// Display name of a unit type.
    pub fn unit(&self, id: UnitTypeId) -> &str {
        self.units.get(id as usize).map_or("?", String::as_str)
    }

    /// Model that represents a fleet on the map under `style`; `fleet_units` are the unit types in
    /// the fleet and `flagship` the representative one (see [`crate::geometry::flagship`]).
    pub fn fleet_model(&self, style: FleetModelStyle, fleet_units: impl Iterator<Item = UnitTypeId>, flagship: Option<UnitTypeId>) -> Option<&str> {
        match style {
            FleetModelStyle::Flagship => flagship.and_then(|u| self.unit_space_models.get(u as usize)).and_then(|m| m.as_deref()).or(self.generic_fleet_model.as_deref()),
            FleetModelStyle::Generic => {
                let mut units = fleet_units;
                units
                    .find_map(|u| self.unit_models.get(u as usize).and_then(|m| m.as_deref()))
                    .or(self.generic_fleet_model.as_deref())
            }
        }
    }

    /// Display name of a structure type.
    pub fn structure(&self, id: StructureTypeId) -> &str {
        self.structures.get(id as usize).map_or("?", String::as_str)
    }

    /// Display name of a star base level.
    pub fn starbase(&self, id: StarbaseTypeId) -> &str {
        self.starbases.get(id as usize).map_or("?", String::as_str)
    }

    /// Display name of a faction.
    pub fn faction(&self, id: u32) -> &str {
        self.factions.get(id as usize).map_or("?", String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prettify_replaces_underscores() {
        assert_eq!(prettify("Star_Destroyer"), "Star Destroyer");
    }
}

