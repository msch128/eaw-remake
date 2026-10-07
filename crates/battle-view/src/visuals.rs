//! Per-ship render data that the simulation catalog does not carry: model scale, death effects and
//! the model bones belonging to each hardpoint.

use std::collections::HashMap;

use alamo_data::GameData;
use battle_sim::{Catalog, HpKind, ShipId, TargetClass};
use bevy::prelude::*;

/// Visual description of one hardpoint (same index as in [`battle_sim::ShipDef::hardpoints`]).
#[derive(Debug, Clone, Default)]
pub struct HpVisual {
    /// Model bone the hardpoint mesh hangs on (`Attachment_Bone`).
    pub bone: Option<String>,
    /// Bone whose proxy particle systems are damage fires (`Damage_Particles`).
    pub damage_bone: Option<String>,
    /// Particle object played when the hardpoint blows up (`Death_Explosion_Particles`).
    pub death_effect: Option<String>,
}

/// Visual description of one ship definition.
#[derive(Debug, Clone, Default)]
pub struct ShipVisual {
    /// Model file (`Empire_Imperial_SD.ALO`); `None` for squadron definitions.
    pub model: Option<String>,
    /// `Scale_Factor`.
    pub scale: f32,
    /// Particle objects played when the unit dies (fighters, `Death_Explosions`).
    pub death_effects: Vec<String>,
    /// One entry per simulation hardpoint.
    pub hardpoints: Vec<HpVisual>,
}

/// Visual data for every ship definition of a [`Catalog`], plus the team colour per side.
#[derive(Debug, Clone, Default)]
pub struct VisualCatalog {
    /// Indexed by [`ShipId`].
    pub ships: Vec<ShipVisual>,
    /// Team colour per side index.
    pub team_colors: Vec<Color>,
}

impl VisualCatalog {
    /// Reads the render fields of every ship in `catalog` from the game XML.
    pub fn build(data: &GameData, catalog: &Catalog, team_colors: Vec<Color>) -> Self {
        let mut ships = Vec::with_capacity(catalog.ships.len());
        for def in &catalog.ships {
            let r = data.objects.resolve(&def.name);
            let scale = r
                .as_ref()
                .and_then(|r| r.num::<f32>("scale_factor"))
                .filter(|s| *s > 0.0)
                .unwrap_or(1.0);
            let death_effects = r
                .as_ref()
                .map(|r| r.list("death_explosions"))
                .unwrap_or_default();
            let hardpoints = def
                .hardpoints
                .iter()
                .map(|h| {
                    let Some(hd) = data.objects.hardpoint(&h.name) else {
                        return HpVisual::default();
                    };
                    let field = |k: &str| {
                        hd.fields
                            .get(k)
                            .map(|v| v.trim().to_string())
                            .filter(|v| !v.is_empty())
                    };
                    HpVisual {
                        bone: hd.attachment_bone.clone().filter(|b| !b.is_empty()),
                        damage_bone: field("damage_particles"),
                        death_effect: field("death_explosion_particles"),
                    }
                })
                .collect();
            ships.push(ShipVisual {
                model: def.model.clone(),
                scale,
                death_effects,
                hardpoints,
            });
        }
        Self { ships, team_colors }
    }

    /// Visual data of a ship definition.
    pub fn ship(&self, id: ShipId) -> &ShipVisual {
        &self.ships[id as usize]
    }

    /// Team colour of a side (white for unknown sides).
    pub fn team_color(&self, side: u8) -> Color {
        self.team_colors
            .get(side as usize)
            .copied()
            .unwrap_or(Color::WHITE)
    }
}

/// Particle object to play when a ship of `class` blows up (Remake/vanilla names).
pub fn ship_death_effect(class: TargetClass) -> &'static str {
    match class {
        TargetClass::Fighter | TargetClass::Bomber => "Small_Explosion_Space_Fighter",
        TargetClass::Transport | TargetClass::Corvette => "Large_Explosion_Space",
        TargetClass::Frigate => "Large_Explosion_Space_Empire_pd_medium_3_300",
        TargetClass::Capital => "Large_Explosion_Space_Empire_pd_medium_3_750",
        TargetClass::Super => "Large_Explosion_Space_Empire_pd_medium_3_1250",
        _ => "Large_Explosion_Space_Empire_pd_medium_3_300",
    }
}

/// True for hardpoint kinds that deserve a marker the player can target.
pub fn is_markable(kind: HpKind) -> bool {
    !matches!(kind, HpKind::Facing | HpKind::Other)
}

/// Lower-cased bone name lookup for a loaded model.
pub type BoneMap = HashMap<String, Entity>;
