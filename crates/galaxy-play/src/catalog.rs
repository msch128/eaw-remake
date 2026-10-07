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
    /// `Galactic_Model_Name` per unit type.
    pub unit_models: Vec<Option<String>>,
    /// Display name per structure type.
    pub structures: Vec<String>,
    /// Display name per star base level.
    pub starbases: Vec<String>,
    /// Display name per faction.
    pub factions: Vec<String>,
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
            structures: world.structures.iter().map(|s| prettify(&s.name)).collect(),
            starbases: world.starbases.iter().map(|s| prettify(&s.name)).collect(),
            factions: world.factions.iter().map(|f| prettify(&f.name)).collect(),
        }
    }

    /// Catalog with English names from the string table and models from the unit definitions.
    pub fn from_data(world: &World, data: &GameData) -> Self {
        let text = |id: &Option<String>, fallback: &str| -> String {
            id.as_deref().and_then(|k| data.text(k)).filter(|t| !t.is_empty()).map_or_else(|| prettify(fallback), str::to_owned)
        };
        let mut units = Vec::with_capacity(world.units.len());
        let mut unit_models = Vec::with_capacity(world.units.len());
        for u in &world.units {
            units.push(text(&u.text_id, &u.name));
            let model = data
                .objects
                .space_unit(&u.name)
                .and_then(|d| d.fields.get("galactic_model_name").cloned())
                .filter(|m| !m.is_empty() && !m.to_ascii_lowercase().contains("invisible"));
            unit_models.push(model);
        }
        Self {
            units,
            unit_models,
            structures: world.structures.iter().map(|s| text(&s.text_id, &s.name)).collect(),
            starbases: world
                .starbases
                .iter()
                .map(|s| text(&data.objects.star_base(&s.name).and_then(|d| d.text_id), &s.name))
                .collect(),
            factions: world
                .factions
                .iter()
                .map(|f| text(&data.faction(&f.name).and_then(|d| d.text_id.clone()), &f.name))
                .collect(),
        }
    }

    /// Display name of a unit type.
    pub fn unit(&self, id: UnitTypeId) -> &str {
        self.units.get(id as usize).map_or("?", String::as_str)
    }

    /// Galactic model file of a unit type.
    pub fn unit_model(&self, id: UnitTypeId) -> Option<&str> {
        self.unit_models.get(id as usize).and_then(|m| m.as_deref())
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
