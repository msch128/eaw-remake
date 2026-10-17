//! Per-ship render data that the simulation catalog does not carry: model scale, death effects and
//! the model bones belonging to each hardpoint.

use std::collections::{HashMap, HashSet};

use alamo_data::GameData;
use battle_sim::{Catalog, HpKind, ShipId, TargetClass};
use bevy::prelude::*;

use crate::flight::{FlightLook, FlightXml};

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
    /// `SFXEvent_Ambient_Moving`: played now and then while the unit moves (the fighters' flybys).
    pub ambient_moving: Option<String>,
    /// `SFXEvent_Ambient_Moving_Min_Delay_Seconds` / `_Max_Delay_Seconds`: the random gap between two of them.
    pub ambient_moving_delay: (f32, f32),
    /// `SFXEvent_Group_Move` (heroes: the line for a move order given to a group; estimate, the data does not say when).
    pub group_move: Option<String>,
    /// `SFXEvent_Group_Attack`.
    pub group_attack: Option<String>,
    /// `SFXEvent_Assist_Move` (units without own move line; `SFX_NA` / blank dropped).
    pub assist_move: Option<String>,
    /// `SFXEvent_Assist_Attack`.
    pub assist_attack: Option<String>,
    /// `Death_SFXEvent_End_Die` (end of the death sequence).
    pub death_end: Option<String>,
    /// Sounds of each `Unit_Ability` of `Unit_Abilities_Data`.
    pub abilities: Vec<AbilitySfx>,
}

/// Sound events of one `Unit_Ability` block.
#[derive(Debug, Clone, Default)]
pub struct AbilitySfx {
    /// `Type`, upper-case (`TURBO`, `TRACTOR_BEAM` ...); matches `AbilityDef::kind`.
    pub kind: String,
    /// `SFXEvent_GUI_Unit_Ability_Activated`.
    pub activated: Option<String>,
    /// `SFXEvent_GUI_Unit_Ability_Deactivated`.
    pub deactivated: Option<String>,
    /// `SFXEvent_Target_Ability` (the unit's line when the player picks a target for the ability).
    pub target: Option<String>,
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
    /// How it banks and pitches in flight (`Bank_Turn_Angle`, `Max_Rate_Of_Roll`, `Max_Lift`), see [`crate::flight`].
    pub flight: FlightLook,
    /// The closest two places of any squadron this type flies in (`Squadron_Offsets`), `None` for a type that is in none: the room a craft
    /// has around it, which its weave must fit ([`crate::flight::weave_scale`]).
    pub squadron_gap: Option<f32>,
    /// Look and sound of the unit's `SUPER_LASER` shot, if it has one.
    pub super_laser: Option<SuperLaserVisual>,
}

/// Look and sound of a `SUPER_LASER` shot: the `Super_Laser_Ability` the unit's `Unit_Ability` names (`GUI_Activated_Ability_Name`),
/// plus that `Unit_Ability`'s loop sound.
#[derive(Debug, Clone, PartialEq)]
pub struct SuperLaserVisual {
    /// `Owner_Particle_Bone_Name`: the bone or proxy the beam leaves from (`MuzzleA_00` on the Eclipse, the dish proxy on the Death Star).
    pub bone: Option<String>,
    /// `Beam_Texture_Name` (`tractor_beam01.tga` on the Eclipse); `None` keeps the plain beam line (the Death Star's data has no beam art).
    pub beam_texture: Option<String>,
    /// `Num_Anim_Frames`: frames of the beam texture.
    pub beam_frames: u32,
    /// `Line_Width` in world units (0 = not given).
    pub line_width: f32,
    /// `Line_Color` (`R G B A`, 0..255): tint of the beam texture.
    pub line_color: Color,
    /// `Fire_Continuation`: seconds the beam keeps firing (0 = not given).
    pub fire_seconds: f32,
    /// `Attack_Anim_Sub_Index`: the variant of the shooter's `ATTACK` animation that runs from the order to the shot; its
    /// `Fire_Frame` is the moment of the shot.
    pub attack_variant: u32,
    /// `SFXEvent_Special_Ability_Loop` of the `Unit_Ability`: the sound of the shot ([`crate::ships::ShotSounds`]).
    pub fire_loop: Option<String>,
}

impl SuperLaserVisual {
    /// Reads the shot of a resolved unit; `None` if it has no `SUPER_LASER` `Unit_Ability`.
    pub fn from_unit(r: &alamo_data::objects::Resolved) -> Option<Self> {
        let ability = r
            .element("unit_abilities_data")?
            .children_named("unit_ability")
            .find(|a| a.child_text("type").is_some_and(|t| t.trim().eq_ignore_ascii_case("SUPER_LASER")))?;
        let text = |e: &alamo_data::xml::Element, t: &str| e.child_text(t).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let name = text(ability, "gui_activated_ability_name");
        let lasers: Vec<&alamo_data::xml::Element> = r.elements_named("abilities").flat_map(|a| a.children_named("super_laser_ability")).collect();
        let laser = lasers
            .iter()
            .find(|l| name.as_deref().is_some_and(|n| l.attr("name").is_some_and(|a| a.trim().eq_ignore_ascii_case(n))))
            .or(lasers.first());
        let field = |t: &str| laser.and_then(|l| text(l, t));
        let num = |t: &str| field(t).and_then(|v| v.trim_end_matches(['f', 'F']).parse::<f32>().ok());
        let line_color = field("line_color")
            .map(|v| v.split([' ', ',']).filter_map(|c| c.trim().parse::<u8>().ok()).collect::<Vec<u8>>())
            .filter(|c| c.len() >= 3)
            .map_or(Color::WHITE, |c| Color::srgba_u8(c[0], c[1], c[2], c.get(3).copied().unwrap_or(255)));
        Some(Self {
            bone: field("owner_particle_bone_name"),
            beam_texture: field("beam_texture_name"),
            beam_frames: num("num_anim_frames").map_or(1, |n| (n as u32).max(1)),
            line_width: num("line_width").unwrap_or(0.0),
            line_color,
            fire_seconds: num("fire_continuation").unwrap_or(0.0),
            attack_variant: num("attack_anim_sub_index").map_or(0, |n| n as u32),
            fire_loop: text(ability, "sfxevent_special_ability_loop").filter(|n| !n.eq_ignore_ascii_case("sfx_na")),
        })
    }
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
    /// `Projectile_SFXEvent_Detonate_Reduced_By_Armor` per projectile definition (most laser bolts name only this one).
    pub proj_detonate_armor: Vec<Option<String>>,
    /// `Projectile_SFXEvent_Detonate_Shield` / `_Shield_Detonate` / `_Detonate_Reduced_By_Shield` per projectile definition.
    pub proj_detonate_shield: Vec<Option<String>>,
}

impl VisualCatalog {
    /// Reads the render fields of every ship in `catalog` from the game XML.
    pub fn build(data: &GameData, catalog: &Catalog, team_colors: Vec<Color>) -> Self {
        let mut ships = Vec::with_capacity(catalog.ships.len());
        // Types that fly in a squadron are small craft whatever their class (a Skipray blast boat squadron).
        let squadron_members: HashSet<ShipId> = catalog.ships.iter().flat_map(|d| d.members.iter().map(|&(m, _)| m)).collect();
        let mut gaps: HashMap<ShipId, f32> = HashMap::new();
        for squadron in &catalog.ships {
            let Some(gap) = squadron.closest_places() else { continue };
            for &(member, _) in &squadron.members {
                let g = gaps.entry(member).or_insert(gap);
                *g = g.min(gap);
            }
        }
        for (id, def) in catalog.ships.iter().enumerate() {
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
                ability: sfx("sfxevent_gui_unit_ability_activated")
                    .or_else(|| sfx("sfxevent_ability_activated"))
                    .or_else(|| sfx("sfxevent_special_ability")),
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
                ambient_moving: sfx("sfxevent_ambient_moving").filter(|n| !n.eq_ignore_ascii_case("sfx_na")),
                ambient_moving_delay: ambient_moving_delay(
                    r.as_ref().and_then(|r| r.num("sfxevent_ambient_moving_min_delay_seconds")),
                    r.as_ref().and_then(|r| r.num("sfxevent_ambient_moving_max_delay_seconds")),
                ),
                group_move: sfx("sfxevent_group_move").filter(|n| !n.eq_ignore_ascii_case("sfx_na")),
                group_attack: sfx("sfxevent_group_attack").filter(|n| !n.eq_ignore_ascii_case("sfx_na")),
                assist_move: sfx("sfxevent_assist_move").filter(|n| !n.eq_ignore_ascii_case("sfx_na")),
                assist_attack: sfx("sfxevent_assist_attack").filter(|n| !n.eq_ignore_ascii_case("sfx_na")),
                death_end: sfx("death_sfxevent_end_die"),
                abilities: r
                    .as_ref()
                    .and_then(|r| r.element("unit_abilities_data"))
                    .map(|data| {
                        data.children_named("unit_ability")
                            .map(|a| {
                                let tag = |t: &str| {
                                    a.child_text(t).map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("sfx_na"))
                                };
                                AbilitySfx {
                                    kind: a.child_text("type").unwrap_or_default().trim().to_ascii_uppercase(),
                                    activated: tag("sfxevent_gui_unit_ability_activated"),
                                    deactivated: tag("sfxevent_gui_unit_ability_deactivated"),
                                    target: tag("sfxevent_target_ability"),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            let xml_num = |tag: &str| r.as_ref().and_then(|r| r.num::<f32>(tag)).unwrap_or(0.0);
            let flight_xml = FlightXml {
                bank_turn_angle: xml_num("bank_turn_angle"),
                max_rate_of_roll: xml_num("max_rate_of_roll"),
                max_lift: xml_num("max_lift"),
                max_speed: xml_num("max_speed"),
            };
            let flight = FlightLook::new(def.class, squadron_members.contains(&(id as ShipId)), flight_xml, def.turn_rate, def.max_speed);
            ships.push(ShipVisual {
                model: def.model.clone(),
                scale,
                death_effects,
                death_clone_model,
                hardpoints,
                sfx: unit_sfx,
                hero: r.as_ref().and_then(|r| r.flag("is_named_hero")).unwrap_or(false),
                flight,
                squadron_gap: gaps.get(&(id as ShipId)).copied(),
                super_laser: r.as_ref().and_then(SuperLaserVisual::from_unit),
            });
        }
        let proj_tag = |tags: &[&str]| -> Vec<Option<String>> {
            catalog
                .projectiles
                .iter()
                .map(|p| {
                    let r = data.objects.resolve(&p.name)?;
                    tags.iter().find_map(|t| r.get(t).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()))
                })
                .collect()
        };
        let proj_detonate = proj_tag(&["projectile_sfxevent_detonate"]);
        let proj_detonate_armor = proj_tag(&["projectile_sfxevent_detonate_reduced_by_armor"]);
        let proj_detonate_shield = proj_tag(&[
            "projectile_sfxevent_detonate_shield",
            "projectile_sfxevent_shield_detonate",
            "projectile_sfxevent_detonate_reduced_by_shield",
        ]);
        Self { ships, team_colors, proj_detonate, proj_detonate_armor, proj_detonate_shield }
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
        TargetClass::Super | TargetClass::Structure => "Large_Explosion_Space_Empire_pd_medium_3_1250",
        _ => "Large_Explosion_Space_Empire_pd_medium_3_300",
    }
}

/// Gap in seconds between two `SFXEvent_Ambient_Moving` sounds when a unit names no delays: the data's most common pair (estimate:
/// 188 of the 261 space units with the event name 3 to 5 s; only the `Ravager` names none).
pub const DEFAULT_AMBIENT_MOVING_DELAY: (f32, f32) = (3.0, 5.0);
/// Shortest gap between two `SFXEvent_Ambient_Moving` sounds of one unit (guards against a 0 delay; the data's shortest is 1 s).
const MIN_AMBIENT_MOVING_DELAY: f32 = 0.25;

/// The `min..max` gap between two `SFXEvent_Ambient_Moving` sounds of a unit from its min / max delay tags.
pub fn ambient_moving_delay(min: Option<f32>, max: Option<f32>) -> (f32, f32) {
    let min = min.unwrap_or(DEFAULT_AMBIENT_MOVING_DELAY.0).max(MIN_AMBIENT_MOVING_DELAY);
    let max = max.unwrap_or(DEFAULT_AMBIENT_MOVING_DELAY.1).max(min);
    (min, max)
}

/// True for hardpoint kinds that deserve a marker the player can target.
pub fn is_markable(kind: HpKind) -> bool {
    !matches!(kind, HpKind::Facing | HpKind::Other)
}

/// Lower-cased bone name lookup for a loaded model.
pub type BoneMap = HashMap<String, Entity>;
