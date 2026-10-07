//! Combat balance tables from `Gameconstants.xml`: damage types, armor types and the
//! damage-to-armor multiplier matrix.

use std::collections::HashMap;

use crate::xml::{Element, split_list};

/// Damage type vs armor type multipliers (`Damage_To_Armor_Mod`). Armor types include shield
/// armor types such as `Shield_Starship`, so the same lookup serves hull and shield damage.
#[derive(Debug, Clone, Default)]
pub struct CombatTable {
    /// `Damage_Types` in declaration order.
    pub damage_types: Vec<String>,
    /// `Armor_Types` in declaration order (hull and shield armor types).
    pub armor_types: Vec<String>,
    mods: HashMap<(String, String), f32>,
}

impl CombatTable {
    /// Reads the table from the root element of `Gameconstants.xml`.
    pub fn from_constants(root: &Element) -> Self {
        let list = |tag: &str| {
            root.child(tag)
                .map(|e| split_list(&e.text))
                .unwrap_or_default()
        };
        let mut mods = HashMap::new();
        for e in root.children_named("damage_to_armor_mod") {
            if let [damage, armor, value, ..] = split_list(&e.text).as_slice()
                && let Ok(v) = value.parse::<f32>()
            {
                // First entry wins, like the other tables.
                mods.entry((damage.to_ascii_lowercase(), armor.to_ascii_lowercase()))
                    .or_insert(v);
            }
        }
        Self {
            damage_types: list("damage_types"),
            armor_types: list("armor_types"),
            mods,
        }
    }

    /// Damage multiplier for `damage_type` against `armor_type`; 1.0 when no entry exists.
    pub fn multiplier(&self, damage_type: &str, armor_type: &str) -> f32 {
        self.mods
            .get(&(
                damage_type.trim().to_ascii_lowercase(),
                armor_type.trim().to_ascii_lowercase(),
            ))
            .copied()
            .unwrap_or(1.0)
    }

    /// Number of explicit matrix entries.
    pub fn len(&self) -> usize {
        self.mods.len()
    }

    /// True if no matrix entries were loaded.
    pub fn is_empty(&self) -> bool {
        self.mods.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::parse_str;

    #[test]
    fn parses_matrix() {
        let mut w = Vec::new();
        let root = parse_str(
            "<GameConstants><Damage_Types>D1, D2</Damage_Types><Armor_Types>A1,\n A2</Armor_Types>\
             <Damage_To_Armor_Mod> D1, A1, 0.5 </Damage_To_Armor_Mod><Damage_To_Armor_Mod>D1,A1,9</Damage_To_Armor_Mod></GameConstants>",
            "g",
            &mut w,
        )
        .unwrap();
        let t = CombatTable::from_constants(&root);
        assert_eq!(t.damage_types, ["D1", "D2"]);
        assert_eq!(t.armor_types, ["A1", "A2"]);
        assert_eq!(t.multiplier("d1", "A1"), 0.5);
        assert_eq!(t.multiplier("D2", "A1"), 1.0);
        assert_eq!(t.len(), 1);
    }
}
