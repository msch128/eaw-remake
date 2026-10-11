//! Typed galaxy data model built from the XML definitions.
//!
//! Every type keeps a `fields` map with the raw (trimmed) text of every child tag, keyed by
//! lower-cased tag name (first occurrence wins), so values without a typed accessor stay reachable.

use std::collections::HashMap;

use crate::xml::{Element, parse_bool, parse_floats, split_list};

/// Collects the first occurrence of every child tag as `lower-case name -> trimmed text`.
fn collect_fields(el: &Element) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    for c in &el.children {
        fields
            .entry(c.name.clone())
            .or_insert_with(|| c.text_trimmed().to_string());
    }
    fields
}

fn get<'a>(fields: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    fields.get(key).map(String::as_str)
}

fn get_string(fields: &HashMap<String, String>, key: &str) -> Option<String> {
    get(fields, key)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn get_num<T: std::str::FromStr + Default>(fields: &HashMap<String, String>, key: &str) -> T {
    get(fields, key)
        .and_then(|s| s.parse().ok())
        .unwrap_or_default()
}

fn get_bool(fields: &HashMap<String, String>, key: &str) -> bool {
    get(fields, key).is_some_and(parse_bool)
}

fn get_list(fields: &HashMap<String, String>, key: &str) -> Vec<String> {
    get(fields, key).map(split_list).unwrap_or_default()
}

fn get_vec3(fields: &HashMap<String, String>, key: &str) -> [f32; 3] {
    let v = get(fields, key).map(parse_floats).unwrap_or_default();
    [
        v.first().copied().unwrap_or(0.0),
        v.get(1).copied().unwrap_or(0.0),
        v.get(2).copied().unwrap_or(0.0),
    ]
}

fn get_color(fields: &HashMap<String, String>, key: &str) -> Option<[u8; 4]> {
    let v = get(fields, key).map(parse_floats)?;
    if v.len() < 3 {
        return None;
    }
    let c = |i: usize, d: f32| v.get(i).copied().unwrap_or(d).clamp(0.0, 255.0) as u8;
    Some([c(0, 0.0), c(1, 0.0), c(2, 0.0), c(3, 255.0)])
}

/// A `Planet` game object. Besides real planets this also covers the dummy/background
/// objects the galactic map uses (galaxy art, faction icons); see [`Planet::is_real_planet`].
#[derive(Debug, Clone, Default)]
pub struct Planet {
    /// Object name, e.g. `Coruscant_WW`.
    pub name: String,
    /// The `Description` attribute (e.g. `"Urban, System"`), if any.
    pub description: Option<String>,
    /// Display name text key (`Text_ID`).
    pub text_id: Option<String>,
    /// Encyclopedia text keys (`Encyclopedia_Text`).
    pub encyclopedia_text: Vec<String>,
    /// Text keys of all `Describe_*` tags (population, wildlife, terrain, ...) in file order.
    pub description_ids: Vec<String>,
    /// `Galactic_Position` x, y, z.
    pub position: [f32; 3],
    /// `Galactic_Model_Name` (.alo).
    pub galactic_model: Option<String>,
    /// `Destroyed_Galactic_Model_Name`.
    pub destroyed_model: Option<String>,
    /// `Terrain` type, e.g. `Urban`.
    pub terrain: Option<String>,
    /// `Behavior` list, e.g. `SELECTABLE, PLANET, PRODUCTION`.
    pub behavior: Vec<String>,
    /// `Planet_Credit_Value` (income).
    pub credit_value: i32,
    /// `Planet_Destroyed_Credit_Value`.
    pub destroyed_credit_value: i32,
    /// `Max_Space_Base` (number of space structure slots).
    pub max_space_base: i32,
    /// `Max_Ground_Base` (number of ground structure slots).
    pub max_ground_base: i32,
    /// `Special_Structures_Land`.
    pub special_structures_land: i32,
    /// `Special_Structures_Space`.
    pub special_structures_space: i32,
    /// `Planet_Surface_Accessible`.
    pub surface_accessible: bool,
    /// `Space_Tactical_Map` (.ted) used for space battles over this planet.
    pub space_tactical_map: Option<String>,
    /// `Destroyed_Space_Tactical_Map`.
    pub destroyed_space_tactical_map: Option<String>,
    /// `Land_Tactical_Map` (unused when ground combat is out of scope).
    pub land_tactical_map: Option<String>,
    /// `Planet_Ability_Name` text key / identifier.
    pub ability_name: Option<String>,
    /// `Planet_Ability_Description` text key.
    pub ability_description: Option<String>,
    /// `Abilities` list.
    pub abilities: Vec<String>,
    /// Sub-abilities (e.g. `Reduce_Production_Time_Ability`).
    pub sub_abilities: Vec<crate::space::SubAbility>,
    /// `Scale_Factor` of the galactic model.
    pub scale_factor: f32,
    /// `Show_Name`.
    pub show_name: bool,
    /// File (relative to `Data/Xml`) the definition came from.
    pub source_file: String,
    /// Raw child tags, see module docs.
    pub fields: HashMap<String, String>,
}

impl Planet {
    pub(crate) fn from_element(el: &Element, source_file: &str) -> Option<Self> {
        let name = el.attr("name")?.trim().to_string();
        let f = collect_fields(el);
        let description_ids = el
            .children
            .iter()
            .filter(|c| c.name.starts_with("describe_"))
            .map(|c| c.text_trimmed().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let mut sub_abilities = Vec::new();
        for abilities_el in el.children.iter().filter(|c| c.name == "abilities") {
            for a in &abilities_el.children {
                let sub_name = a.attr("name").unwrap_or_default().trim().to_string();
                let mut sub_f = HashMap::new();
                for c in &a.children {
                    sub_f.entry(c.name.clone()).or_insert_with(|| c.text_trimmed().to_string());
                }
                sub_abilities.push(crate::space::SubAbility {
                    name: sub_name,
                    kind: a.raw_name.clone(),
                    fields: sub_f,
                });
            }
        }
        let legacy_structures: i32 = get_num(&f, "special_structures");
        let land = f
            .get("special_structures_land")
            .map_or(legacy_structures, |_| {
                get_num(&f, "special_structures_land")
            });
        Some(Self {
            name,
            description: el.attr("description").map(str::to_string),
            text_id: get_string(&f, "text_id"),
            encyclopedia_text: get_list(&f, "encyclopedia_text"),
            description_ids,
            position: get_vec3(&f, "galactic_position"),
            galactic_model: get_string(&f, "galactic_model_name"),
            destroyed_model: get_string(&f, "destroyed_galactic_model_name"),
            terrain: get_string(&f, "terrain"),
            behavior: get_list(&f, "behavior"),
            credit_value: get_num(&f, "planet_credit_value"),
            destroyed_credit_value: get_num(&f, "planet_destroyed_credit_value"),
            max_space_base: get_num(&f, "max_space_base"),
            max_ground_base: get_num(&f, "max_ground_base"),
            special_structures_land: land,
            special_structures_space: get_num(&f, "special_structures_space"),
            surface_accessible: get_bool(&f, "planet_surface_accessible"),
            space_tactical_map: get_string(&f, "space_tactical_map"),
            destroyed_space_tactical_map: get_string(&f, "destroyed_space_tactical_map"),
            land_tactical_map: get_string(&f, "land_tactical_map"),
            ability_name: get_string(&f, "planet_ability_name"),
            ability_description: get_string(&f, "planet_ability_description"),
            abilities: get_list(&f, "abilities"),
            sub_abilities,
            scale_factor: get_num(&f, "scale_factor"),
            show_name: get_bool(&f, "show_name"),
            source_file: source_file.to_string(),
            fields: f,
        })
    }

    /// True for actual planets (behavior contains `PLANET`), false for background art,
    /// icons and dummies that share the `Planet` object type.
    pub fn is_real_planet(&self) -> bool {
        self.behavior
            .iter()
            .any(|b| b.eq_ignore_ascii_case("planet"))
    }
}

/// A hyperspace lane between two planets.
#[derive(Debug, Clone, Default)]
pub struct TradeRoute {
    /// Route name, e.g. `WW_001`.
    pub name: String,
    /// `Point_A` planet name.
    pub point_a: String,
    /// `Point_B` planet name.
    pub point_b: String,
    /// `HS_Speed_Factor` (hyperspace speed multiplier on this lane).
    pub hs_speed_factor: f32,
    /// `Political_Control_Gain`.
    pub political_control_gain: f32,
    /// `Credit_Gain_Factor`.
    pub credit_gain_factor: f32,
    /// `Visible_Line_Name`, a [`TradeRouteLine`] name.
    pub visible_line_name: Option<String>,
    /// File the definition came from.
    pub source_file: String,
    /// Raw child tags.
    pub fields: HashMap<String, String>,
}

impl TradeRoute {
    pub(crate) fn from_element(el: &Element, source_file: &str) -> Option<Self> {
        let name = el.attr("name")?.trim().to_string();
        let f = collect_fields(el);
        Some(Self {
            name,
            point_a: get(&f, "point_a").unwrap_or_default().to_string(),
            point_b: get(&f, "point_b").unwrap_or_default().to_string(),
            hs_speed_factor: get_num(&f, "hs_speed_factor"),
            political_control_gain: get_num(&f, "political_control_gain"),
            credit_gain_factor: get_num(&f, "credit_gain_factor"),
            visible_line_name: get_string(&f, "visible_line_name"),
            source_file: source_file.to_string(),
            fields: f,
        })
    }
}

/// Rendering style of a trade route line for one relation (`Neutral`, `Enemy`, `Ally`, ...).
#[derive(Debug, Clone, Default)]
pub struct LineSettings {
    /// `Settings_For_Faction` name.
    pub relation: String,
    /// `Color_Zoomed_Out` RGBA.
    pub color_zoomed_out: Option<[u8; 4]>,
    /// `Color_Zoomed_In` RGBA.
    pub color_zoomed_in: Option<[u8; 4]>,
    /// `Width`.
    pub width: f32,
    /// `Render_Mode`, e.g. `ADDITIVE`.
    pub render_mode: Option<String>,
}

/// A named visual style for trade routes (`TradeRouteLine` in `TRADEROUTELINES.XML`).
#[derive(Debug, Clone, Default)]
pub struct TradeRouteLine {
    /// Style name referenced by [`TradeRoute::visible_line_name`].
    pub name: String,
    /// Per-relation settings.
    pub settings: Vec<LineSettings>,
}

impl TradeRouteLine {
    pub(crate) fn from_element(el: &Element) -> Option<Self> {
        let name = el.attr("name")?.trim().to_string();
        let settings = el
            .children_named("settings_for_faction")
            .map(|s| {
                let f = collect_fields(s);
                LineSettings {
                    relation: s.attr("name").unwrap_or_default().trim().to_string(),
                    color_zoomed_out: get_color(&f, "color_zoomed_out"),
                    color_zoomed_in: get_color(&f, "color_zoomed_in"),
                    width: get_num(&f, "width"),
                    render_mode: get_string(&f, "render_mode"),
                }
            })
            .collect();
        Some(Self { name, settings })
    }
}

/// A faction definition.
#[derive(Debug, Clone, Default)]
pub struct Faction {
    /// Faction name, e.g. `Rebel`.
    pub name: String,
    /// Display name text key.
    pub text_id: Option<String>,
    /// `Color` RGBA.
    pub color: Option<[u8; 4]>,
    /// `Display_Font_Color` RGBA.
    pub display_font_color: Option<[u8; 4]>,
    /// `Is_Playable`.
    pub is_playable: bool,
    /// `Is_Neutral`.
    pub is_neutral: bool,
    /// `Can_Control_Planets` (defaults to true when absent).
    pub can_control_planets: bool,
    /// `Force_Alignment` (`Light`, `Dark`, `Neutral`).
    pub force_alignment: Option<String>,
    /// `Primary_Enemy`.
    pub primary_enemy: Option<String>,
    /// `Enemies` list.
    pub enemies: Vec<String>,
    /// `Icon_Name`.
    pub icon_name: Option<String>,
    /// `Alternate_Icon_Name`: the small emblem drawn next to planets on the galaxy map.
    pub alternate_icon_name: Option<String>,
    /// `Credits_Accumulation_Factor`.
    pub credits_accumulation_factor: f32,
    /// `Maintenance_Cost`.
    pub maintenance_cost: f32,
    /// `Hyperspace_Speed_Factor`.
    pub hyperspace_speed_factor: f32,
    /// `Space_Tactical_Unit_Cap`.
    pub space_unit_cap: i32,
    /// File the definition came from.
    pub source_file: String,
    /// Raw child tags.
    pub fields: HashMap<String, String>,
}

impl Faction {
    pub(crate) fn from_element(el: &Element, source_file: &str) -> Option<Self> {
        let name = el.attr("name")?.trim().to_string();
        let f = collect_fields(el);
        Some(Self {
            name,
            text_id: get_string(&f, "text_id"),
            color: get_color(&f, "color"),
            display_font_color: get_color(&f, "display_font_color"),
            is_playable: get_bool(&f, "is_playable"),
            is_neutral: get_bool(&f, "is_neutral"),
            can_control_planets: f.get("can_control_planets").is_none_or(|v| parse_bool(v)),
            force_alignment: get_string(&f, "force_alignment"),
            primary_enemy: get_string(&f, "primary_enemy"),
            enemies: get_list(&f, "enemies"),
            icon_name: get_string(&f, "icon_name"),
            alternate_icon_name: get_string(&f, "alternate_icon_name"),
            credits_accumulation_factor: get_num(&f, "credits_accumulation_factor"),
            maintenance_cost: get_num(&f, "maintenance_cost"),
            hyperspace_speed_factor: get_num(&f, "hyperspace_speed_factor"),
            space_unit_cap: get_num(&f, "space_tactical_unit_cap"),
            source_file: source_file.to_string(),
            fields: f,
        })
    }
}

/// One `Starting_Forces` line: `faction, planet, object type`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartingForce {
    /// Owning faction.
    pub faction: String,
    /// Planet the object starts on.
    pub planet: String,
    /// Game object type name (unit, structure, ...).
    pub object: String,
}

/// Per-faction settings of a campaign.
#[derive(Debug, Clone, Default)]
pub struct CampaignFaction {
    /// Faction name as written in the campaign.
    pub name: String,
    /// `Starting_Credits`.
    pub starting_credits: Option<i64>,
    /// `Starting_Tech_Level`.
    pub starting_tech_level: Option<i32>,
    /// `Max_Tech_Level`.
    pub max_tech_level: Option<i32>,
    /// `Home_Location` planet.
    pub home_location: Option<String>,
    /// `AI_Player_Control` AI plot name (e.g. `GC_EmpireAi`, `SandboxHuman`).
    pub ai_player_control: Option<String>,
    /// Story plot file (`Story_Name` / `<Faction>_Story_Name`).
    pub story_name: Option<String>,
    /// `Markup_Filename`.
    pub markup_filename: Option<String>,
    /// Distinct planets where this faction has starting forces (plus its home location),
    /// in first-seen order. Planet ownership is derived from these.
    pub presence_planets: Vec<String>,
}

/// A galactic conquest campaign (one entry in a `Campaigns_*.xml`).
#[derive(Debug, Clone, Default)]
pub struct Campaign {
    /// Campaign name, e.g. `GC_Full_Web_Way_Rebel`.
    pub name: String,
    /// `Campaign_Set` (groups campaigns of one galaxy size/map in the menu).
    pub set: Option<String>,
    /// `Sort_Order`.
    pub sort_order: i32,
    /// Display name text key.
    pub text_id: Option<String>,
    /// `Description_Text` text key.
    pub description_id: Option<String>,
    /// `Camera_Shift_X`.
    pub camera_shift_x: f32,
    /// `Camera_Shift_Y`.
    pub camera_shift_y: f32,
    /// `Camera_Distance`.
    pub camera_distance: f32,
    /// `Starting_Active_Player` (the human's faction; `Hostile` for set-only menu entries).
    pub starting_active_player: Option<String>,
    /// `Human_Victory_Conditions`.
    pub human_victory_conditions: Vec<String>,
    /// `Human_Victory_Planet_Names`.
    pub human_victory_planets: Vec<String>,
    /// `AI_Victory_Conditions`.
    pub ai_victory_conditions: Vec<String>,
    /// `AI_Victory_Planet_Names`.
    pub ai_victory_planets: Vec<String>,
    /// `Locations`: planet objects (including dummies/art) present in this campaign.
    pub locations: Vec<String>,
    /// `Trade_Routes`: trade route names present in this campaign.
    pub trade_routes: Vec<String>,
    /// Per-faction settings, in first-seen order.
    pub factions: Vec<CampaignFaction>,
    /// All `Starting_Forces` lines in file order.
    pub starting_forces: Vec<StartingForce>,
    /// File the definition came from.
    pub source_file: String,
    /// Raw child tags (first occurrence of each tag).
    pub fields: HashMap<String, String>,
}

impl Campaign {
    pub(crate) fn from_element(
        el: &Element,
        source_file: &str,
        warn: &mut dyn FnMut(u32, String),
    ) -> Option<Self> {
        let name = el.attr("name")?.trim().to_string();
        let f = collect_fields(el);
        let mut c = Campaign {
            name,
            set: get_string(&f, "campaign_set"),
            sort_order: get_num(&f, "sort_order"),
            text_id: get_string(&f, "text_id"),
            description_id: get_string(&f, "description_text"),
            camera_shift_x: get_num(&f, "camera_shift_x"),
            camera_shift_y: get_num(&f, "camera_shift_y"),
            camera_distance: get_num(&f, "camera_distance"),
            starting_active_player: get_string(&f, "starting_active_player"),
            human_victory_conditions: el
                .children_named("human_victory_conditions")
                .flat_map(|e| split_list(&e.text))
                .collect(),
            human_victory_planets: el
                .children_named("human_victory_planet_names")
                .flat_map(|e| split_list(&e.text))
                .collect(),
            ai_victory_conditions: el
                .children_named("ai_victory_conditions")
                .flat_map(|e| split_list(&e.text))
                .collect(),
            ai_victory_planets: el
                .children_named("ai_victory_planet_names")
                .flat_map(|e| split_list(&e.text))
                .collect(),
            locations: el
                .children_named("locations")
                .flat_map(|e| split_list(&e.text))
                .collect(),
            trade_routes: el
                .children_named("trade_routes")
                .flat_map(|e| split_list(&e.text))
                .collect(),
            source_file: source_file.to_string(),
            fields: f,
            ..Campaign::default()
        };

        for child in &el.children {
            let tag = child.name.as_str();
            let toks = split_list(&child.text);
            match tag {
                "starting_forces" => {
                    if let [faction, planet, object, ..] = toks.as_slice() {
                        c.starting_forces.push(StartingForce {
                            faction: faction.clone(),
                            planet: planet.clone(),
                            object: object.clone(),
                        });
                    } else {
                        warn(
                            child.line,
                            format!("malformed Starting_Forces '{}'", child.text_trimmed()),
                        );
                    }
                }
                "starting_credits"
                | "starting_tech_level"
                | "max_tech_level"
                | "home_location"
                | "ai_player_control"
                | "story_name"
                | "markup_filename" => {
                    let [faction, value, ..] = toks.as_slice() else {
                        warn(
                            child.line,
                            format!("malformed <{tag}> '{}'", child.text_trimmed()),
                        );
                        continue;
                    };
                    let value = value.clone();
                    let cf = c.faction_entry(faction);
                    match tag {
                        "starting_credits" => cf.starting_credits = value.parse().ok(),
                        "starting_tech_level" => cf.starting_tech_level = value.parse().ok(),
                        "max_tech_level" => cf.max_tech_level = value.parse().ok(),
                        "home_location" => cf.home_location = Some(value),
                        "ai_player_control" => cf.ai_player_control = Some(value),
                        "story_name" => cf.story_name = Some(value),
                        _ => cf.markup_filename = Some(value),
                    }
                }
                _ => {
                    if let Some(faction) = tag.strip_suffix("_story_name").filter(|f| !f.is_empty())
                    {
                        // `<Rebel_Story_Name>file.xml</Rebel_Story_Name>`; match the real faction
                        // spelling when the faction is already known, else use the tag prefix.
                        let faction = c
                            .factions
                            .iter()
                            .find(|x| x.name.eq_ignore_ascii_case(faction))
                            .map_or_else(|| faction.to_string(), |x| x.name.clone());
                        c.faction_entry(&faction).story_name =
                            Some(child.text_trimmed().to_string());
                    }
                }
            }
        }

        // Presence: home location plus every planet with starting forces.
        let forces = std::mem::take(&mut c.starting_forces);
        for sf in &forces {
            let cf = c.faction_entry(&sf.faction);
            if !cf
                .presence_planets
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&sf.planet))
            {
                cf.presence_planets.push(sf.planet.clone());
            }
        }
        for cf in &mut c.factions {
            if let Some(home) = cf.home_location.clone()
                && !cf
                    .presence_planets
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(&home))
            {
                cf.presence_planets.insert(0, home);
            }
        }
        c.starting_forces = forces;
        Some(c)
    }

    fn faction_entry(&mut self, name: &str) -> &mut CampaignFaction {
        let idx = match self
            .factions
            .iter()
            .position(|f| f.name.eq_ignore_ascii_case(name))
        {
            Some(i) => i,
            None => {
                self.factions.push(CampaignFaction {
                    name: name.to_string(),
                    ..CampaignFaction::default()
                });
                self.factions.len() - 1
            }
        };
        &mut self.factions[idx]
    }

    /// Per-faction settings by (case-insensitive) faction name.
    pub fn faction(&self, name: &str) -> Option<&CampaignFaction> {
        self.factions
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// True if this campaign actually defines a galaxy (has locations); menu-only entries
    /// such as `Web_Way_Casual` do not.
    pub fn has_galaxy(&self) -> bool {
        !self.locations.is_empty()
    }
}
