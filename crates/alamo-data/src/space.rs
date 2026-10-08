//! Typed views of the object definitions that matter for galactic conquest and space combat:
//! ships and squadrons, hardpoints, projectiles, star bases and planetary structures.
//!
//! All views are built from an inheritance-resolved [`Resolved`] object, so values inherited via
//! `Variant_Of_Existing_Type` are already merged. Every view keeps a `fields` map with the first
//! occurrence of each resolved tag (lower-case name -> trimmed text) for anything not typed here.

use std::collections::HashMap;

use crate::objects::{GameObjects, Resolved};
use crate::xml::{Element, split_list};

fn field_map(r: &Resolved) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for e in r.elements() {
        // The first value wins, unless it is an empty placeholder (`<Applicable_Unit_Categories />` before the real one).
        let slot = m.entry(e.name.clone()).or_insert_with(String::new);
        if slot.is_empty() {
            *slot = e.text_trimmed().to_string();
        }
    }
    m
}

fn sub_fields(el: &Element) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for e in &el.children {
        m.entry(e.name.clone())
            .or_insert_with(|| e.text_trimmed().to_string());
    }
    m
}

/// Parses a `"Name, 3"` style entry.
fn name_count(s: &str) -> Option<(String, u32)> {
    let toks = split_list(s);
    match toks.as_slice() {
        [name] => Some((name.clone(), 1)),
        [name, n, ..] => Some((name.clone(), n.parse().unwrap_or(1))),
        [] => None,
    }
}

/// Coarse ship size class derived from `CategoryMask`, ordered from smallest to largest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum ShipCategory {
    /// Anything not recognized (heroes without mask, props, ...).
    #[default]
    Other,
    /// Space structure (station, platform).
    SpaceStructure,
    /// Fighter squadron member.
    Fighter,
    /// Bomber.
    Bomber,
    /// Transport.
    Transport,
    /// Corvette.
    Corvette,
    /// Frigate.
    Frigate,
    /// Capital ship.
    Capital,
    /// Super capital ship.
    Super,
}

impl ShipCategory {
    /// Picks the largest category named in a `CategoryMask` token list.
    pub fn from_mask(tokens: &[String]) -> Self {
        tokens
            .iter()
            .map(|t| match t.to_ascii_lowercase().as_str() {
                "super" | "supercapital" => ShipCategory::Super,
                "capital" => ShipCategory::Capital,
                "frigate" => ShipCategory::Frigate,
                "corvette" => ShipCategory::Corvette,
                "transport" => ShipCategory::Transport,
                "bomber" => ShipCategory::Bomber,
                "fighter" => ShipCategory::Fighter,
                "spacestructure" => ShipCategory::SpaceStructure,
                _ => ShipCategory::Other,
            })
            .max()
            .unwrap_or(ShipCategory::Other)
    }
}

/// The fighters a carrier or station launches at one tech level.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpawnedUnits {
    /// The N of `..._Tech_N` (0 to 4).
    pub tech: u8,
    /// `Starting_Spawned_Units_Tech_N` as `(squadron/unit, count)`: launched when the battle starts.
    pub starting: Vec<(String, u32)>,
    /// `Reserve_Spawned_Units_Tech_N`: replacements for destroyed squadrons.
    pub reserve: Vec<(String, u32)>,
}

/// The entry that applies to an owner of tech level `tech`: the highest level at or below it, else the lowest one the object has.
pub fn spawned_for_tech(list: &[SpawnedUnits], tech: u8) -> Option<&SpawnedUnits> {
    list.iter().filter(|s| s.tech <= tech).max_by_key(|s| s.tech).or_else(|| list.iter().min_by_key(|s| s.tech))
}

fn parse_spawned_by_tech(r: &Resolved) -> Vec<SpawnedUnits> {
    (0..=4u8)
        .filter_map(|n| {
            let get = |what: &str| -> Vec<(String, u32)> { r.get_all(&format!("{what}_spawned_units_tech_{n}")).into_iter().filter_map(name_count).collect() };
            let (starting, reserve) = (get("starting"), get("reserve"));
            (!starting.is_empty() || !reserve.is_empty()).then_some(SpawnedUnits { tech: n, starting, reserve })
        })
        .collect()
}

/// An entry of `Unit_Abilities_Data` (active ability bar button such as `FULL_SALVO`).
#[derive(Debug, Clone, Default)]
pub struct UnitAbility {
    /// `Type`, e.g. `FULL_SALVO`, `TRACTOR_BEAM`.
    pub kind: String,
    /// `Recharge_Seconds`.
    pub recharge_seconds: Option<f32>,
    /// `Expiration_Seconds`.
    pub expiration_seconds: Option<f32>,
    /// `GUI_Activated_Ability_Name`: the named sub-ability this button triggers.
    pub activated_ability: Option<String>,
    /// All `Mod_Multiplier` entries as `(UPPERCASE name, value)`, e.g. `("SPEED_MULTIPLIER", 2.0)`.
    pub mod_multipliers: Vec<(String, f32)>,
    /// Raw child tags.
    pub fields: HashMap<String, String>,
}

/// A named sub-object of `Abilities` (e.g. `Tractor_Beam_Attack_Ability`).
#[derive(Debug, Clone, Default)]
pub struct SubAbility {
    /// `Name` attribute.
    pub name: String,
    /// Tag name with original capitalization, i.e. the ability class.
    pub kind: String,
    /// Raw child tags.
    pub fields: HashMap<String, String>,
}

/// A `Planet_Income_Bonus_Ability`: how an object changes the income of its planet.
#[derive(Debug, Clone, Default)]
pub struct IncomeModifier {
    /// Ability name.
    pub name: String,
    /// `Specific_Mod_Source_Text` (text key shown in the economy tooltip).
    pub source_text: Option<String>,
    /// `Absolute_Income_Modifier` credits (negative for upkeep).
    pub absolute: f32,
    /// `Percentage_Income_Modifier` (fraction of base planet income).
    pub percentage: f32,
    /// `Additional_Multiplier_On_Mining_Colonies`.
    pub mining_colony_multiplier: Option<f32>,
}

fn parse_unit_abilities(r: &Resolved) -> Vec<UnitAbility> {
    r.elements_named("unit_abilities_data")
        .flat_map(|d| d.children_named("unit_ability"))
        .map(|a| {
            let f = sub_fields(a);
            UnitAbility {
                kind: f.get("type").cloned().unwrap_or_default(),
                recharge_seconds: f.get("recharge_seconds").and_then(|v| v.parse().ok()),
                expiration_seconds: f.get("expiration_seconds").and_then(|v| v.parse().ok()),
                activated_ability: f
                    .get("gui_activated_ability_name")
                    .filter(|s| !s.is_empty())
                    .cloned(),
                mod_multipliers: a
                    .children_named("mod_multiplier")
                    .filter_map(|m| {
                        let t = split_list(m.text_trimmed());
                        let value = t.get(1)?.trim_end_matches(['f', 'F']).parse().ok()?;
                        Some((t.first()?.to_ascii_uppercase(), value))
                    })
                    .collect(),
                fields: f,
            }
        })
        .collect()
}

fn parse_sub_abilities(r: &Resolved) -> (Vec<SubAbility>, Vec<IncomeModifier>) {
    let mut subs = Vec::new();
    let mut income = Vec::new();
    for a in r
        .elements_named("abilities")
        .flat_map(|d| d.children.iter())
    {
        let name = a.attr("name").unwrap_or_default().trim().to_string();
        let f = sub_fields(a);
        if a.name == "planet_income_bonus_ability" {
            let num = |k: &str| f.get(k).and_then(|v| v.parse::<f32>().ok());
            income.push(IncomeModifier {
                name: name.clone(),
                source_text: f
                    .get("specific_mod_source_text")
                    .filter(|s| !s.is_empty())
                    .cloned(),
                absolute: num("absolute_income_modifier").unwrap_or(0.0),
                percentage: num("percentage_income_modifier").unwrap_or(0.0),
                mining_colony_multiplier: num("additional_multiplier_on_mining_colonies"),
            });
        }
        subs.push(SubAbility {
            name,
            kind: a.raw_name.clone(),
            fields: f,
        });
    }
    (subs, income)
}

/// Weapon fired directly by the unit instead of through hardpoints (fighters, small craft).
#[derive(Debug, Clone, Default)]
pub struct DirectWeapon {
    /// `Projectile_Types` projectile names.
    pub projectile_types: Vec<String>,
    /// `Projectile_Fire_Pulse_Count`.
    pub pulse_count: u32,
    /// `Projectile_Fire_Pulse_Delay_Seconds`.
    pub pulse_delay_seconds: f32,
    /// `Projectile_Fire_Recharge_Seconds`.
    pub recharge_seconds: f32,
}

/// A ship, fighter, hero ship or squadron (`SpaceUnit`, `UniqueUnit`, `Squadron`, ...).
#[derive(Debug, Clone, Default)]
pub struct SpaceUnitDef {
    /// Object name.
    pub name: String,
    /// Root tag, e.g. `SpaceUnit`, `UniqueUnit`, `Squadron`.
    pub kind: String,
    /// Display name text key.
    pub text_id: Option<String>,
    /// `Encyclopedia_Text` keys.
    pub encyclopedia_text: Vec<String>,
    /// `Affiliation`: factions that may own/build the unit.
    pub affiliation: Vec<String>,
    /// `CategoryMask` tokens (e.g. `Capital`, `AntiFrigate`).
    pub category_mask: Vec<String>,
    /// Size class derived from the mask (for squadrons: from the first member).
    pub category: ShipCategory,
    /// `Ship_Class`.
    pub ship_class: Option<String>,
    /// `Property_Flags` (e.g. `SmallShip`).
    pub property_flags: Vec<String>,
    /// `Space_Model_Name` (.alo).
    pub space_model: Option<String>,
    /// `Scale_Factor`.
    pub scale_factor: f32,
    /// `Icon_Name`.
    pub icon: Option<String>,
    /// `Tactical_Health` (hull points).
    pub hull: f32,
    /// `Shield_Points`.
    pub shield_points: f32,
    /// `Shield_Refresh_Rate` (shield points per second).
    pub shield_refresh_rate: f32,
    /// `Armor_Type` (see [`crate::CombatTable`]).
    pub armor_type: Option<String>,
    /// `Shield_Armor_Type`.
    pub shield_armor_type: Option<String>,
    /// `Energy_Capacity`.
    pub energy_capacity: f32,
    /// `Energy_Refresh_Rate`.
    pub energy_refresh_rate: f32,
    /// `Max_Speed`.
    pub max_speed: f32,
    /// `Min_Speed`.
    pub min_speed: f32,
    /// `Max_Rate_Of_Turn`.
    pub max_rate_of_turn: f32,
    /// `Max_Rate_Of_Roll`.
    pub max_rate_of_roll: f32,
    /// `Bank_Turn_Angle`.
    pub bank_turn_angle: f32,
    /// `OverrideAcceleration`.
    pub acceleration: f32,
    /// `OverrideDeceleration`.
    pub deceleration: f32,
    /// `Hyperspace_Speed`.
    pub hyperspace_speed: f32,
    /// `Hyperspace` (can jump).
    pub can_hyperspace: bool,
    /// `Space_FOW_Reveal_Range`.
    pub fow_reveal_range: f32,
    /// `Targeting_Max_Attack_Distance`.
    pub targeting_max_distance: f32,
    /// `Targeting_Min_Attack_Distance`.
    pub targeting_min_distance: f32,
    /// `Targeting_Priority_Set` name.
    pub targeting_priority_set: Option<String>,
    /// `HardPoints` names, in order.
    pub hardpoints: Vec<String>,
    /// Weapon fired without hardpoints, if any.
    pub direct_weapon: Option<DirectWeapon>,
    /// `Build_Cost_Credits`.
    pub build_cost: f32,
    /// `Build_Time_Seconds`.
    pub build_time: f32,
    /// `Tech_Level`.
    pub tech_level: i32,
    /// `Population_Value`.
    pub population_value: i32,
    /// `Required_Star_Base_Level`.
    pub required_star_base_level: i32,
    /// `Required_Special_Structures` (structures the planet must have to build this).
    pub required_special_structures: Vec<String>,
    /// `Required_Planets` (the build planet must be one of these).
    pub required_planets: Vec<String>,
    /// `Required_Timeline`.
    pub required_timeline: i32,
    /// `Build_Limit_Current_Per_Player`.
    pub build_limit_current: Option<i32>,
    /// `Build_Limit_Lifetime_Per_Player`.
    pub build_limit_lifetime: Option<i32>,
    /// `Build_Initially_Locked`.
    pub build_initially_locked: bool,
    /// `AI_Combat_Power`.
    pub ai_combat_power: f32,
    /// `Autoresolve_Health` (used by auto-resolve, squadrons and stations).
    pub autoresolve_health: f32,
    /// `Squadron_Capacity` (how many squadrons a carrier launches).
    pub squadron_capacity: i32,
    /// For squadrons: member unit names (`Squadron_Units`).
    pub squadron_units: Vec<String>,
    /// For squadrons: formation offsets (`Squadron_Offsets`).
    pub squadron_offsets: Vec<[f32; 3]>,
    /// `Max_Squad_Size`.
    pub max_squad_size: Option<i32>,
    /// `Starting_Spawned_Units_Tech_N` / `Reserve_Spawned_Units_Tech_N` for every tech level N the object has entries for.
    pub spawned_by_tech: Vec<SpawnedUnits>,
    /// `Starting_Spawned_Units_Tech_0` as `(squadron/unit, count)`.
    pub starting_spawned_units: Vec<(String, u32)>,
    /// `Reserve_Spawned_Units_Tech_0` as `(squadron/unit, count)`.
    pub reserve_spawned_units: Vec<(String, u32)>,
    /// `Unit_Abilities_Data` entries.
    pub abilities: Vec<UnitAbility>,
    /// Named sub-abilities under `Abilities`.
    pub sub_abilities: Vec<SubAbility>,
    /// Income changes applied to the owner's planet (upkeep is negative).
    pub income_modifiers: Vec<IncomeModifier>,
    /// `Death_Clone` (wreck object), if any.
    pub death_clone: Option<String>,
    /// File the leaf definition came from.
    pub source_file: String,
    /// Raw resolved tags.
    pub fields: HashMap<String, String>,
}

impl SpaceUnitDef {
    fn from_resolved(r: &Resolved, store: &GameObjects) -> Self {
        let f32_or = |t: &str| r.num::<f32>(t).unwrap_or(0.0);
        let i32_or = |t: &str| r.num::<i32>(t).unwrap_or(0);
        let category_mask = r.list("categorymask");
        let squadron_units = r.list("squadron_units");
        let mut category = ShipCategory::from_mask(&category_mask);
        if category == ShipCategory::Other
            && let Some(first) = squadron_units.first().and_then(|u| store.resolve(u))
        {
            category = ShipCategory::from_mask(&first.list("categorymask"));
        }
        let (sub_abilities, income_modifiers) = parse_sub_abilities(r);
        let direct_weapon = r
            .get("projectile_types")
            .filter(|s| !s.is_empty())
            .map(|_| DirectWeapon {
                projectile_types: r.list("projectile_types"),
                pulse_count: r.num("projectile_fire_pulse_count").unwrap_or(1),
                pulse_delay_seconds: f32_or("projectile_fire_pulse_delay_seconds"),
                recharge_seconds: f32_or("projectile_fire_recharge_seconds"),
            });
        // The mod spells the tag `HardPoints`, some objects `Hardpoints`; both lower-case to one tag.
        Self {
            name: r.name().to_string(),
            kind: r.kind().to_string(),
            text_id: r.string("text_id"),
            encyclopedia_text: r.list("encyclopedia_text"),
            affiliation: r.list("affiliation"),
            category_mask,
            category,
            ship_class: r.string("ship_class"),
            property_flags: r.list("property_flags"),
            space_model: r.string("space_model_name"),
            scale_factor: r.num("scale_factor").unwrap_or(1.0),
            icon: r.string("icon_name"),
            hull: f32_or("tactical_health"),
            shield_points: f32_or("shield_points"),
            shield_refresh_rate: f32_or("shield_refresh_rate"),
            armor_type: r.string("armor_type"),
            shield_armor_type: r.string("shield_armor_type"),
            energy_capacity: f32_or("energy_capacity"),
            energy_refresh_rate: f32_or("energy_refresh_rate"),
            max_speed: f32_or("max_speed"),
            min_speed: f32_or("min_speed"),
            max_rate_of_turn: f32_or("max_rate_of_turn"),
            max_rate_of_roll: f32_or("max_rate_of_roll"),
            bank_turn_angle: f32_or("bank_turn_angle"),
            acceleration: f32_or("overrideacceleration"),
            deceleration: f32_or("overridedeceleration"),
            hyperspace_speed: f32_or("hyperspace_speed"),
            can_hyperspace: r.flag("hyperspace").unwrap_or(false),
            fow_reveal_range: f32_or("space_fow_reveal_range"),
            targeting_max_distance: f32_or("targeting_max_attack_distance"),
            targeting_min_distance: f32_or("targeting_min_attack_distance"),
            targeting_priority_set: r.string("targeting_priority_set"),
            hardpoints: r.list("hardpoints"),
            direct_weapon,
            build_cost: f32_or("build_cost_credits"),
            build_time: f32_or("build_time_seconds"),
            tech_level: i32_or("tech_level"),
            population_value: i32_or("population_value"),
            required_star_base_level: i32_or("required_star_base_level"),
            required_special_structures: r.list("required_special_structures"),
            required_planets: r.list("required_planets"),
            required_timeline: i32_or("required_timeline"),
            build_limit_current: r.num("build_limit_current_per_player"),
            build_limit_lifetime: r.num("build_limit_lifetime_per_player"),
            build_initially_locked: r.flag("build_initially_locked").unwrap_or(false),
            ai_combat_power: f32_or("ai_combat_power"),
            autoresolve_health: f32_or("autoresolve_health"),
            squadron_capacity: i32_or("squadron_capacity"),
            squadron_units,
            squadron_offsets: r
                .get_all("squadron_offsets")
                .iter()
                .map(|s| {
                    let v = crate::xml::parse_floats(s);
                    [
                        v.first().copied().unwrap_or(0.0),
                        v.get(1).copied().unwrap_or(0.0),
                        v.get(2).copied().unwrap_or(0.0),
                    ]
                })
                .collect(),
            max_squad_size: r.num("max_squad_size"),
            spawned_by_tech: parse_spawned_by_tech(r),
            starting_spawned_units: r
                .get_all("starting_spawned_units_tech_0")
                .into_iter()
                .filter_map(name_count)
                .collect(),
            reserve_spawned_units: r
                .get_all("reserve_spawned_units_tech_0")
                .into_iter()
                .filter_map(name_count)
                .collect(),
            abilities: parse_unit_abilities(r),
            sub_abilities,
            income_modifiers,
            death_clone: r.string("death_clone"),
            source_file: r.object.source_file.clone(),
            fields: field_map(r),
        }
    }

    /// True for fighter/bomber squadrons (`Squadron` objects).
    pub fn is_squadron(&self) -> bool {
        self.kind.eq_ignore_ascii_case("squadron")
    }

    /// Sum of absolute income modifiers (negative = upkeep).
    pub fn upkeep(&self) -> f32 {
        self.income_modifiers.iter().map(|m| m.absolute).sum()
    }
}

/// Turret rotation limits of a hardpoint.
#[derive(Debug, Clone, Default)]
pub struct TurretDef {
    /// `Turret_Bone_Name`.
    pub turret_bone: Option<String>,
    /// `Barrel_Bone_Name`.
    pub barrel_bone: Option<String>,
    /// `Turret_Rotate_Extent_Degrees`.
    pub rotate_extent: f32,
    /// `Turret_Elevate_Extent_Degrees`.
    pub elevate_extent: f32,
    /// `Turret_Rotate_Speed`.
    pub rotate_speed: f32,
    /// `Turret_Rest_Angle`.
    pub rest_angle: f32,
}

/// A `HardPoint`: a weapon, shield generator, engine, fighter bay etc. on a ship or station.
#[derive(Debug, Clone, Default)]
pub struct HardpointDef {
    /// Object name.
    pub name: String,
    /// `Type`, e.g. `HARD_POINT_WEAPON_LASER`, `HARD_POINT_ENGINE`, `HARD_POINT_SHIELD_GENERATOR`.
    pub hp_type: String,
    /// `Health`; `None` when the hardpoint has no own hit points.
    pub health: Option<f32>,
    /// `Is_Targetable`.
    pub is_targetable: bool,
    /// `Is_Destroyable`.
    pub is_destroyable: bool,
    /// `Tooltip_Text` key.
    pub tooltip_text: Option<String>,
    /// `Attachment_Bone`.
    pub attachment_bone: Option<String>,
    /// `Collision_Mesh`.
    pub collision_mesh: Option<String>,
    /// `Fire_Bone_A`.
    pub fire_bone_a: Option<String>,
    /// `Fire_Bone_B`.
    pub fire_bone_b: Option<String>,
    /// `Randomize_Between_Fire_Bones`.
    pub randomize_fire_bones: bool,
    /// `Fire_Projectile_Type` projectile name.
    pub projectile: Option<String>,
    /// `Fire_Min_Recharge_Seconds`.
    pub fire_min_recharge: f32,
    /// `Fire_Max_Recharge_Seconds`.
    pub fire_max_recharge: f32,
    /// `Fire_Pulse_Count`.
    pub fire_pulse_count: u32,
    /// `Fire_Pulse_Delay_Seconds`.
    pub fire_pulse_delay: f32,
    /// `Fire_Range_Distance`.
    pub fire_range: f32,
    /// `Fire_Min_Range_Distance`.
    pub fire_min_range: f32,
    /// `Fire_Cone_Width` in degrees.
    pub fire_cone_width: f32,
    /// `Fire_Cone_Height` in degrees.
    pub fire_cone_height: f32,
    /// `Fire_Inaccuracy_Distance` as `(target category, distance)`.
    pub fire_inaccuracy: Vec<(String, f32)>,
    /// `Fire_Category_Restrictions`: target categories this weapon may attack.
    pub fire_category_restrictions: Vec<String>,
    /// `Allow_Opportunity_Fire_When_Idle`.
    pub opportunity_fire_idle: bool,
    /// `Allow_Opportunity_Fire_When_Targeting`.
    pub opportunity_fire_targeting: bool,
    /// Turret limits when `Is_Turret`.
    pub turret: Option<TurretDef>,
    /// `Damage_Type` override (rare; most damage type comes from the projectile).
    pub damage_type: Option<String>,
    /// `Repair_Amount_Per_Frame` (repair hardpoints).
    pub repair_amount_per_frame: f32,
    /// `Repair_Cost_Per_Frame`.
    pub repair_cost_per_frame: f32,
    /// `Special_Ability_Name` this hardpoint enables.
    pub special_ability_name: Option<String>,
    /// File the definition came from.
    pub source_file: String,
    /// Raw resolved tags.
    pub fields: HashMap<String, String>,
}

impl HardpointDef {
    fn from_resolved(r: &Resolved) -> Self {
        let f32_or = |t: &str| r.num::<f32>(t).unwrap_or(0.0);
        let flag = |t: &str| r.flag(t).unwrap_or(false);
        let turret = flag("is_turret").then(|| TurretDef {
            turret_bone: r.string("turret_bone_name"),
            barrel_bone: r.string("barrel_bone_name"),
            rotate_extent: f32_or("turret_rotate_extent_degrees"),
            elevate_extent: f32_or("turret_elevate_extent_degrees"),
            rotate_speed: f32_or("turret_rotate_speed"),
            rest_angle: f32_or("turret_rest_angle"),
        });
        Self {
            name: r.name().to_string(),
            hp_type: r.get("type").unwrap_or_default().to_string(),
            health: r.num("health"),
            is_targetable: flag("is_targetable"),
            is_destroyable: flag("is_destroyable"),
            tooltip_text: r.string("tooltip_text"),
            attachment_bone: r.string("attachment_bone"),
            collision_mesh: r.string("collision_mesh"),
            fire_bone_a: r.string("fire_bone_a"),
            fire_bone_b: r.string("fire_bone_b"),
            randomize_fire_bones: flag("randomize_between_fire_bones"),
            projectile: r.string("fire_projectile_type"),
            fire_min_recharge: f32_or("fire_min_recharge_seconds"),
            fire_max_recharge: f32_or("fire_max_recharge_seconds"),
            fire_pulse_count: r.num("fire_pulse_count").unwrap_or(0),
            fire_pulse_delay: f32_or("fire_pulse_delay_seconds"),
            fire_range: f32_or("fire_range_distance"),
            fire_min_range: f32_or("fire_min_range_distance"),
            fire_cone_width: f32_or("fire_cone_width"),
            fire_cone_height: f32_or("fire_cone_height"),
            fire_inaccuracy: r
                .get_all("fire_inaccuracy_distance")
                .iter()
                .filter_map(|s| {
                    let t = split_list(s);
                    Some((t.first()?.clone(), t.get(1)?.parse().ok()?))
                })
                .collect(),
            fire_category_restrictions: r.list("fire_category_restrictions"),
            opportunity_fire_idle: flag("allow_opportunity_fire_when_idle"),
            opportunity_fire_targeting: flag("allow_opportunity_fire_when_targeting"),
            turret,
            damage_type: r.string("damage_type"),
            repair_amount_per_frame: f32_or("repair_amount_per_frame"),
            repair_cost_per_frame: f32_or("repair_cost_per_frame"),
            special_ability_name: r.string("special_ability_name"),
            source_file: r.object.source_file.clone(),
            fields: field_map(r),
        }
    }

    /// True for hardpoints that fire a projectile.
    pub fn is_weapon(&self) -> bool {
        self.projectile.is_some()
    }
}

/// A `Projectile` (laser bolt, missile, torpedo, bomb ...).
#[derive(Debug, Clone, Default)]
pub struct ProjectileDef {
    /// Object name.
    pub name: String,
    /// `Projectile_Category`, e.g. `LASER`, `MISSILE`.
    pub category: Option<String>,
    /// `Damage_Type`, a row key of the damage/armor table.
    pub damage_type: Option<String>,
    /// `Projectile_Damage`.
    pub damage: f32,
    /// `Max_Speed`.
    pub max_speed: f32,
    /// `Max_Rate_Of_Turn` (homing agility).
    pub max_rate_of_turn: f32,
    /// `Projectile_Max_Flight_Distance`.
    pub max_flight_distance: f32,
    /// `Projectile_Max_Lifetime`.
    pub max_lifetime: Option<f32>,
    /// `Projectile_Does_Shield_Damage`.
    pub does_shield_damage: bool,
    /// `Projectile_Does_Hitpoint_Damage`.
    pub does_hitpoint_damage: bool,
    /// `Projectile_Does_Energy_Damage`.
    pub does_energy_damage: bool,
    /// `Projectile_Energy_Per_Shot`.
    pub energy_per_shot: f32,
    /// `Projectile_Blast_Area_Range`.
    pub blast_range: f32,
    /// `Projectile_Blast_Area_Damage`.
    pub blast_damage: f32,
    /// `Projectile_Blast_Area_Max_Victims`.
    pub blast_max_victims: Option<i32>,
    /// `Projectile_Ion_Stun_Duration`.
    pub ion_stun_duration: f32,
    /// `Projectile_Ion_Stun_Speed_Reduction_Percent`.
    pub ion_stun_speed_reduction: f32,
    /// `Projectile_Ion_Stun_Shot_Rate_Reduction_Percent`.
    pub ion_stun_shot_rate_reduction: f32,
    /// `Projectile_Disables_Engines_When_Power_Drained`.
    pub disables_engines_when_drained: bool,
    /// `Projectile_Damages_Random_Hard_Points`.
    pub damages_random_hardpoints: bool,
    /// `Space_Model_Name`.
    pub space_model: Option<String>,
    /// `Behavior` list, e.g. `PROJECTILE`, `HIDE_WHEN_FOGGED`.
    pub behavior: Vec<String>,
    /// File the definition came from.
    pub source_file: String,
    /// Raw resolved tags.
    pub fields: HashMap<String, String>,
}

impl ProjectileDef {
    fn from_resolved(r: &Resolved) -> Self {
        let f32_or = |t: &str| r.num::<f32>(t).unwrap_or(0.0);
        let flag = |t: &str| r.flag(t).unwrap_or(false);
        Self {
            name: r.name().to_string(),
            category: r.string("projectile_category"),
            damage_type: r.string("damage_type"),
            damage: f32_or("projectile_damage"),
            max_speed: f32_or("max_speed"),
            max_rate_of_turn: f32_or("max_rate_of_turn"),
            max_flight_distance: f32_or("projectile_max_flight_distance"),
            max_lifetime: r.num("projectile_max_lifetime"),
            does_shield_damage: flag("projectile_does_shield_damage"),
            does_hitpoint_damage: flag("projectile_does_hitpoint_damage"),
            does_energy_damage: flag("projectile_does_energy_damage"),
            energy_per_shot: f32_or("projectile_energy_per_shot"),
            blast_range: f32_or("projectile_blast_area_range"),
            blast_damage: f32_or("projectile_blast_area_damage"),
            blast_max_victims: r.num("projectile_blast_area_max_victims"),
            ion_stun_duration: f32_or("projectile_ion_stun_duration"),
            ion_stun_speed_reduction: f32_or("projectile_ion_stun_speed_reduction_percent"),
            ion_stun_shot_rate_reduction: f32_or("projectile_ion_stun_shot_rate_reduction_percent"),
            disables_engines_when_drained: flag("projectile_disables_engines_when_power_drained"),
            damages_random_hardpoints: flag("projectile_damages_random_hard_points"),
            space_model: r.string("space_model_name"),
            behavior: r.list("behavior"),
            source_file: r.object.source_file.clone(),
            fields: field_map(r),
        }
    }
}

/// A space station (`StarBase`): one level of a planet's orbital base.
#[derive(Debug, Clone, Default)]
pub struct StarBaseDef {
    /// Object name, e.g. `Empire_Star_Base_2`.
    pub name: String,
    /// Display name text key.
    pub text_id: Option<String>,
    /// `Affiliation`.
    pub affiliation: Vec<String>,
    /// `Base_Level` (1-5).
    pub base_level: i32,
    /// `Prev_Level_Base`.
    pub prev_level: Option<String>,
    /// `Next_Level_Base`.
    pub next_level: Option<String>,
    /// `Space_Model_Name` (tactical).
    pub space_model: Option<String>,
    /// `Galactic_Model_Name`.
    pub galactic_model: Option<String>,
    /// `Tactical_Health`.
    pub hull: f32,
    /// `Shield_Points`.
    pub shield_points: f32,
    /// `Shield_Refresh_Rate`.
    pub shield_refresh_rate: f32,
    /// `Armor_Type`.
    pub armor_type: Option<String>,
    /// `Shield_Armor_Type`.
    pub shield_armor_type: Option<String>,
    /// `HardPoints` names.
    pub hardpoints: Vec<String>,
    /// `Build_Cost_Credits` (cost of this level, upgrade from the previous one).
    pub build_cost: f32,
    /// `Build_Time_Seconds`.
    pub build_time: f32,
    /// `Tech_Level`.
    pub tech_level: i32,
    /// `Required_Star_Base_Level`.
    pub required_star_base_level: i32,
    /// `Additional_Population_Capacity` granted by this level.
    pub population_capacity: i32,
    /// `Autoresolve_Health`.
    pub autoresolve_health: f32,
    /// `AI_Combat_Power`.
    pub ai_combat_power: f32,
    /// `Space_FOW_Reveal_Range`.
    pub fow_reveal_range: f32,
    /// `Reinforcement_Prevention_Radius`.
    pub reinforcement_prevention_radius: f32,
    /// `Starting_Spawned_Units_Tech_N` / `Reserve_Spawned_Units_Tech_N` for every tech level N the base has entries for.
    pub spawned_by_tech: Vec<SpawnedUnits>,
    /// `Starting_Spawned_Units_Tech_0`: fighter garrison at battle start.
    pub starting_spawned_units: Vec<(String, u32)>,
    /// `Reserve_Spawned_Units_Tech_0`: replacement garrison.
    pub reserve_spawned_units: Vec<(String, u32)>,
    /// Objects this level unlocks: `Tactical_Buildable_Objects_Campaign` entries (faction prefix dropped).
    pub unlocks: Vec<String>,
    /// Income changes (e.g. upkeep) applied to the planet.
    pub income_modifiers: Vec<IncomeModifier>,
    /// File the definition came from.
    pub source_file: String,
    /// Raw resolved tags.
    pub fields: HashMap<String, String>,
}

impl StarBaseDef {
    fn from_resolved(r: &Resolved) -> Self {
        let f32_or = |t: &str| r.num::<f32>(t).unwrap_or(0.0);
        let i32_or = |t: &str| r.num::<i32>(t).unwrap_or(0);
        let (_, income_modifiers) = parse_sub_abilities(r);
        // `Tactical_Buildable_Objects_Campaign = Faction, Obj1, Obj2, ...`
        let unlocks = r
            .get_all("tactical_buildable_objects_campaign")
            .iter()
            .flat_map(|s| split_list(s).into_iter().skip(1))
            .collect();
        Self {
            name: r.name().to_string(),
            text_id: r
                .get("text_id")
                .and_then(|s| split_list(s).into_iter().next()),
            affiliation: r.list("affiliation"),
            base_level: i32_or("base_level"),
            prev_level: r.string("prev_level_base"),
            next_level: r.string("next_level_base"),
            space_model: r.string("space_model_name"),
            galactic_model: r.string("galactic_model_name"),
            hull: f32_or("tactical_health"),
            shield_points: f32_or("shield_points"),
            shield_refresh_rate: f32_or("shield_refresh_rate"),
            armor_type: r.string("armor_type"),
            shield_armor_type: r.string("shield_armor_type"),
            hardpoints: r.list("hardpoints"),
            build_cost: f32_or("build_cost_credits"),
            build_time: f32_or("build_time_seconds"),
            tech_level: i32_or("tech_level"),
            required_star_base_level: i32_or("required_star_base_level"),
            population_capacity: i32_or("additional_population_capacity"),
            autoresolve_health: f32_or("autoresolve_health"),
            ai_combat_power: f32_or("ai_combat_power"),
            fow_reveal_range: f32_or("space_fow_reveal_range"),
            reinforcement_prevention_radius: f32_or("reinforcement_prevention_radius"),
            spawned_by_tech: parse_spawned_by_tech(r),
            starting_spawned_units: r
                .get_all("starting_spawned_units_tech_0")
                .into_iter()
                .filter_map(name_count)
                .collect(),
            reserve_spawned_units: r
                .get_all("reserve_spawned_units_tech_0")
                .into_iter()
                .filter_map(name_count)
                .collect(),
            unlocks,
            income_modifiers,
            source_file: r.object.source_file.clone(),
            fields: field_map(r),
        }
    }
}

/// A planetary structure (`SpecialStructure`, `SpaceStructure`, `TechBuilding`, ...): shipyards,
/// mines, defense grids, research buildings. Economy effects are in [`StructureDef::income_modifiers`].
#[derive(Debug, Clone, Default)]
pub struct StructureDef {
    /// Object name.
    pub name: String,
    /// Root tag.
    pub kind: String,
    /// Display name text key.
    pub text_id: Option<String>,
    /// `Affiliation`.
    pub affiliation: Vec<String>,
    /// `Base_Position` (`In_Base`, `Out_Of_Base`, ...).
    pub base_position: Option<String>,
    /// `Tactical_Health`.
    pub hull: f32,
    /// `Shield_Points`.
    pub shield_points: f32,
    /// `Armor_Type`.
    pub armor_type: Option<String>,
    /// `HardPoints` names (space-side weapons of orbital structures).
    pub hardpoints: Vec<String>,
    /// `Build_Cost_Credits`.
    pub build_cost: f32,
    /// `Build_Time_Seconds`.
    pub build_time: f32,
    /// `Tech_Level`.
    pub tech_level: i32,
    /// `Required_Star_Base_Level`.
    pub required_star_base_level: i32,
    /// `Required_Ground_Base_Level`.
    pub required_ground_base_level: i32,
    /// `Required_Special_Structures`.
    pub required_special_structures: Vec<String>,
    /// `Build_Max_Instances_Per_Planet`.
    pub max_instances_per_planet: Option<i32>,
    /// `Build_Initially_Locked`.
    pub build_initially_locked: bool,
    /// `Is_Dummy`.
    pub is_dummy: bool,
    /// `Space_Model_Name`.
    pub space_model: Option<String>,
    /// `Additional_Population_Capacity`.
    pub population_capacity: i32,
    /// `Tactical_Buildable_Objects_Campaign` entries (faction prefix dropped): what this unlocks.
    pub unlocks: Vec<String>,
    /// Income changes applied to the planet.
    pub income_modifiers: Vec<IncomeModifier>,
    /// Named sub-abilities.
    pub sub_abilities: Vec<SubAbility>,
    /// File the definition came from.
    pub source_file: String,
    /// Raw resolved tags.
    pub fields: HashMap<String, String>,
}

impl StructureDef {
    fn from_resolved(r: &Resolved) -> Self {
        let f32_or = |t: &str| r.num::<f32>(t).unwrap_or(0.0);
        let i32_or = |t: &str| r.num::<i32>(t).unwrap_or(0);
        let (sub_abilities, income_modifiers) = parse_sub_abilities(r);
        let unlocks = r
            .get_all("tactical_buildable_objects_campaign")
            .iter()
            .flat_map(|s| split_list(s).into_iter().skip(1))
            .collect();
        Self {
            name: r.name().to_string(),
            kind: r.kind().to_string(),
            text_id: r
                .get("text_id")
                .and_then(|s| split_list(s).into_iter().next()),
            affiliation: r.list("affiliation"),
            base_position: r.string("base_position"),
            hull: f32_or("tactical_health"),
            shield_points: f32_or("shield_points"),
            armor_type: r.string("armor_type"),
            hardpoints: r.list("hardpoints"),
            build_cost: f32_or("build_cost_credits"),
            build_time: f32_or("build_time_seconds"),
            tech_level: i32_or("tech_level"),
            required_star_base_level: i32_or("required_star_base_level"),
            required_ground_base_level: i32_or("required_ground_base_level"),
            required_special_structures: r.list("required_special_structures"),
            max_instances_per_planet: r.num("build_max_instances_per_planet"),
            build_initially_locked: r.flag("build_initially_locked").unwrap_or(false),
            is_dummy: r.flag("is_dummy").unwrap_or(false),
            space_model: r.string("space_model_name"),
            population_capacity: i32_or("additional_population_capacity"),
            unlocks,
            income_modifiers,
            sub_abilities,
            source_file: r.object.source_file.clone(),
            fields: field_map(r),
        }
    }

    /// Sum of absolute income modifiers in credits.
    pub fn absolute_income(&self) -> f32 {
        self.income_modifiers.iter().map(|m| m.absolute).sum()
    }
}

/// Root tags that denote ships, squadrons and hero ships.
const SPACE_UNIT_KINDS: [&str; 4] = ["spaceunit", "uniqueunit", "squadron", "transportunit"];
/// Root tags that denote planetary/orbital structures other than star bases.
const STRUCTURE_KINDS: [&str; 5] = [
    "specialstructure",
    "spacestructure",
    "techbuilding",
    "secondarystructure",
    "groundbase",
];

impl GameObjects {
    /// Typed ship/squadron definition (inheritance resolved). `None` if the object does not exist
    /// or is not a space unit kind.
    pub fn space_unit(&self, name: &str) -> Option<SpaceUnitDef> {
        let r = self.resolve(name)?;
        SPACE_UNIT_KINDS
            .contains(&r.kind().to_ascii_lowercase().as_str())
            .then(|| SpaceUnitDef::from_resolved(&r, self))
    }

    /// All ships and squadrons that have a space model or are squadrons (excludes ground-only
    /// transports and heroes that only exist on the ground).
    pub fn space_units(&self) -> Vec<SpaceUnitDef> {
        self.iter()
            .filter(|o| SPACE_UNIT_KINDS.contains(&o.kind.to_ascii_lowercase().as_str()))
            .filter_map(|o| self.space_unit(&o.name))
            .filter(|u| u.space_model.is_some() || u.is_squadron())
            .collect()
    }

    /// Typed hardpoint definition.
    pub fn hardpoint(&self, name: &str) -> Option<HardpointDef> {
        let r = self.resolve(name)?;
        r.kind()
            .eq_ignore_ascii_case("hardpoint")
            .then(|| HardpointDef::from_resolved(&r))
    }

    /// Hardpoints of a unit/base in declaration order; names that do not resolve are skipped.
    pub fn hardpoints_of(&self, names: &[String]) -> Vec<HardpointDef> {
        names.iter().filter_map(|n| self.hardpoint(n)).collect()
    }

    /// Typed projectile definition.
    pub fn projectile(&self, name: &str) -> Option<ProjectileDef> {
        let r = self.resolve(name)?;
        r.kind()
            .eq_ignore_ascii_case("projectile")
            .then(|| ProjectileDef::from_resolved(&r))
    }

    /// Typed star base definition.
    pub fn star_base(&self, name: &str) -> Option<StarBaseDef> {
        let r = self.resolve(name)?;
        r.kind()
            .eq_ignore_ascii_case("starbase")
            .then(|| StarBaseDef::from_resolved(&r))
    }

    /// All star base levels (including skirmish variants), in load order.
    pub fn star_bases(&self) -> Vec<StarBaseDef> {
        self.of_kind("starbase")
            .filter_map(|o| self.star_base(&o.name))
            .collect()
    }

    /// Typed planetary structure definition (special structures, tech buildings, ...).
    pub fn structure(&self, name: &str) -> Option<StructureDef> {
        let r = self.resolve(name)?;
        STRUCTURE_KINDS
            .contains(&r.kind().to_ascii_lowercase().as_str())
            .then(|| StructureDef::from_resolved(&r))
    }

    /// True if `name` is an object that makes its planet owned by the faction (star bases and
    /// structures).
    pub fn is_structure(&self, name: &str) -> bool {
        self.get(name).is_some_and(|o| {
            let k = o.kind.to_ascii_lowercase();
            k == "starbase" || STRUCTURE_KINDS.contains(&k.as_str())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::parse_str;

    fn store(xml: &str) -> GameObjects {
        let mut w = Vec::new();
        let root = parse_str(xml, "t.xml", &mut w).unwrap();
        let mut s = GameObjects::default();
        s.add_file_for_tests(root);
        s
    }

    #[test]
    fn typed_views() {
        let s = store(
            "<R>\
             <SpaceUnit Name=\"Ship\"><CategoryMask>Capital | AntiFrigate</CategoryMask><Tactical_Health>100</Tactical_Health>\
             <HardPoints>HP_A, HP_B</HardPoints><Affiliation>Empire, Rebel</Affiliation>\
             <Starting_Spawned_Units_Tech_0>Sq, 6</Starting_Spawned_Units_Tech_0>\
             <Abilities SubObjectList=\"Yes\"><Planet_Income_Bonus_Ability Name=\"Up\"><Absolute_Income_Modifier>-800</Absolute_Income_Modifier></Planet_Income_Bonus_Ability></Abilities>\
             <Unit_Abilities_Data><Unit_Ability><Type>FULL_SALVO</Type><Recharge_Seconds>60</Recharge_Seconds></Unit_Ability></Unit_Abilities_Data></SpaceUnit>\
             <Squadron Name=\"Sq\"><Squadron_Units>Ship, Ship</Squadron_Units><Squadron_Offsets>1,2,3</Squadron_Offsets></Squadron>\
             <HardPoint Name=\"HP_A\"><Type>HARD_POINT_WEAPON_LASER</Type><Fire_Projectile_Type>P</Fire_Projectile_Type>\
             <Fire_Inaccuracy_Distance>Capital, 5.0</Fire_Inaccuracy_Distance><Is_Targetable>Yes</Is_Targetable></HardPoint>\
             <Projectile Name=\"P\"><Projectile_Damage>12</Projectile_Damage><Damage_Type>Damage_Light_Laser</Damage_Type></Projectile></R>",
        );
        let u = s.space_unit("ship").unwrap();
        assert_eq!(u.category, ShipCategory::Capital);
        assert_eq!(u.hardpoints, ["HP_A", "HP_B"]);
        assert_eq!(u.hull, 100.0);
        assert_eq!(u.upkeep(), -800.0);
        assert_eq!(u.starting_spawned_units, [("Sq".to_string(), 6)]);
        assert_eq!(u.abilities[0].recharge_seconds, Some(60.0));
        let sq = s.space_unit("Sq").unwrap();
        assert!(sq.is_squadron());
        assert_eq!(sq.category, ShipCategory::Capital);
        assert_eq!(sq.squadron_offsets, [[1.0, 2.0, 3.0]]);
        let hp = s.hardpoint("HP_A").unwrap();
        assert!(hp.is_weapon() && hp.is_targetable);
        assert_eq!(hp.fire_inaccuracy, [("Capital".to_string(), 5.0)]);
        assert_eq!(s.hardpoints_of(&u.hardpoints).len(), 1);
        assert_eq!(s.projectile("p").unwrap().damage, 12.0);
        assert!(s.space_unit("HP_A").is_none());
    }
}
