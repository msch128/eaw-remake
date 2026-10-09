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
    /// SFX event played when the hardpoint fires (`Fire_SFXEvent`).
    pub fire_sfx: Option<String>,
    /// `Type`, e.g. `HARD_POINT_ENGINE` (upper-case); picks the unit's `SFXEvent_Hardpoint_Destroyed` line.
    pub hp_type: String,
}

/// Sound events of a unit type (names of `SFXEvents.xml` entries).
#[derive(Debug, Clone, Default)]
pub struct UnitSfx {
    /// `SFXEvent_Select`.
    pub select: Option<String>,
    /// `SFXEvent_Move`.
    pub mv: Option<String>,
    /// `SFXEvent_Attack`.
    pub attack: Option<String>,
    /// `Death_Explosion_SFXEvent`.
    pub death: Option<String>,
    /// `SFXEvent_Hardpoint_Destroyed`.
    pub hardpoint_destroyed: Option<String>,
    /// Every `SFXEvent_Hardpoint_Destroyed` entry as `(HARD_POINT_TYPE, event)`; `SFX_NA` entries are dropped.
    pub hardpoint_destroyed_by_type: Vec<(String, String)>,
    /// Every `SFXEvent_Attack_Hardpoint` entry as `(HARD_POINT_TYPE of the target, event)`: the line for an attack on one hardpoint.
    pub attack_hardpoint_by_type: Vec<(String, String)>,
    /// `SFXEvent_GUI_Unit_Ability_Activated`.
    pub ability: Option<String>,
    /// `SFXEvent_Fire` (fighters, which have no hardpoints).
    pub fire: Option<String>,
    /// `SFXEvent_Fleet_Move` (a move order for several units).
    pub fleet_move: Option<String>,
    /// `SFXEvent_Guard`.
    pub guard: Option<String>,
    /// `SFXEvent_Stop`.
    pub stop: Option<String>,
    /// `SFXEvent_Unit_Lost` (this unit died).
    pub unit_lost: Option<String>,
    /// `SFXEvent_Unit_Under_Attack`.
    pub under_attack: Option<String>,
    /// `SFXEvent_Health_Low_Warning` (own unit below `Health_Low_Percent_Threshold`).
    pub health_low: Option<String>,
    /// `SFXEvent_Health_Critical_Warning`.
    pub health_critical: Option<String>,
    /// `SFXEvent_Enemy_Damaged_Health_Low_Warning` (an enemy of this unit type is nearly dead).
    pub enemy_health_low: Option<String>,
    /// `SFXEvent_Enemy_Damaged_Health_Critical_Warning`.
    pub enemy_health_critical: Option<String>,
    /// `SFXEvent_Engine_Idle_Loop` (looping hum while the ship stands still).
    pub engine_idle: Option<String>,
    /// `SFXEvent_Engine_Moving_Loop`.
    pub engine_moving: Option<String>,
    /// `SFXEvent_Ambient_Loop` (constant machinery of stations and big ships).
    pub ambient_loop: Option<String>,
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
    /// Model of the `Death_Clone` object (the hull in pieces) and its animation model stem, for preloading.
    pub death_clone_model: Option<String>,
    /// One entry per simulation hardpoint.
    pub hardpoints: Vec<HpVisual>,
    /// Sound events of the unit (`SFXEvent_Select`, `_Move`, `_Attack`, `Death_Explosion_SFXEvent`).
    pub sfx: UnitSfx,
    /// `Is_Named_Hero`.
    pub hero: bool,
}

/// Visual data for every ship definition of a [`Catalog`], plus the team colour per side.
#[derive(Debug, Clone, Default)]
pub struct VisualCatalog {
    /// Indexed by [`ShipId`].
    pub ships: Vec<ShipVisual>,
    /// Team colour per side index.
    pub team_colors: Vec<Color>,
    /// `Projectile_SFXEvent_Detonate` per projectile definition.
    pub proj_detonate: Vec<Option<String>>,
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
            let death_clone_model = r
                .as_ref()
                .map(|r| r.list("death_clone"))
                .unwrap_or_default()
                .iter()
                .find_map(|c| data.objects.resolve(c).and_then(|c| c.string("space_model_name")))
                .filter(|m| !m.is_empty());
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
                        fire_sfx: field("fire_sfxevent"),
                        hp_type: hd.hp_type.trim().to_ascii_uppercase(),
                    }
                })
                .collect();
            let sfx = |k: &str| {
                r.as_ref()
                    .and_then(|r| r.get(k))
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            };
            let unit_sfx = UnitSfx {
                select: sfx("sfxevent_select"),
                mv: sfx("sfxevent_move"),
                attack: sfx("sfxevent_attack"),
                // `Death_SFXEvent_Start_Die` is the common tag (375 unit files); `Death_Explosion_SFXEvent` the older one.
                death: sfx("death_sfxevent_start_die").or_else(|| sfx("death_explosion_sfxevent")),
                hardpoint_destroyed: sfx("sfxevent_hardpoint_destroyed"),
                attack_hardpoint_by_type: r
                    .as_ref()
                    .map(|r| {
                        r.get_all("sfxevent_attack_hardpoint")
                            .into_iter()
                            .filter_map(|v| v.split_once(','))
                            .map(|(t, e)| (t.trim().to_ascii_uppercase(), e.trim().to_string()))
                            .filter(|(_, e)| !e.is_empty() && !e.eq_ignore_ascii_case("sfx_na"))
                            .collect()
                    })
                    .unwrap_or_default(),
                hardpoint_destroyed_by_type: r
                    .as_ref()
                    .map(|r| {
                        r.get_all("sfxevent_hardpoint_destroyed")
                            .into_iter()
                            .filter_map(|v| v.split_once(','))
                            .map(|(t, e)| (t.trim().to_ascii_uppercase(), e.trim().to_string()))
                            .filter(|(_, e)| !e.is_empty() && !e.eq_ignore_ascii_case("sfx_na"))
                            .collect()
                    })
                    .unwrap_or_default(),
                ability: sfx("sfxevent_gui_unit_ability_activated"),
                fire: sfx("sfxevent_fire"),
                fleet_move: sfx("sfxevent_fleet_move"),
                guard: sfx("sfxevent_guard"),
                stop: sfx("sfxevent_stop"),
                unit_lost: sfx("sfxevent_unit_lost"),
                under_attack: sfx("sfxevent_unit_under_attack"),
                health_low: sfx("sfxevent_health_low_warning"),
                health_critical: sfx("sfxevent_health_critical_warning"),
                enemy_health_low: sfx("sfxevent_enemy_damaged_health_low_warning"),
                enemy_health_critical: sfx("sfxevent_enemy_damaged_health_critical_warning"),
                engine_idle: sfx("sfxevent_engine_idle_loop"),
                engine_moving: sfx("sfxevent_engine_moving_loop"),
                ambient_loop: sfx("sfxevent_ambient_loop"),
            };
            ships.push(ShipVisual {
                model: def.model.clone(),
                scale,
                death_effects,
                death_clone_model,
                hardpoints,
                sfx: unit_sfx,
                hero: r.as_ref().and_then(|r| r.flag("is_named_hero")).unwrap_or(false),
            });
        }
        let proj_detonate = catalog
            .projectiles
            .iter()
            .map(|p| {
                data.objects
                    .resolve(&p.name)
                    .and_then(|r| r.get("projectile_sfxevent_detonate").map(|v| v.trim().to_string()))
                    .filter(|v| !v.is_empty())
            })
            .collect();
        Self { ships, team_colors, proj_detonate }
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
