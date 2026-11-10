//! Builds the unit, structure and star base catalogs from the object definitions.

use std::collections::{HashMap, HashSet};

use alamo_data::{Campaign, GameData, HardpointDef, ProjectileDef, ShipCategory, SpaceUnitDef};

use crate::world::*;

fn key(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

/// `Required_Special_Structures` as groups: names separated by whitespace or commas are all required, names joined by `|` are alternatives.
pub(crate) fn parse_requirement(raw: &str) -> Vec<Vec<String>> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut join = false;
    for tok in raw.replace('|', " | ").split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()) {
        if tok == "|" {
            join = true;
            continue;
        }
        match groups.last_mut() {
            Some(last) if join => last.push(tok.to_string()),
            _ => groups.push(vec![tok.to_string()]),
        }
        join = false;
    }
    groups
}

/// A name list (`Required_Planets`): separated by whitespace, commas or `|`.
pub(crate) fn name_list(raw: &str) -> Vec<String> {
    raw.split(|c: char| c.is_whitespace() || c == ',' || c == '|').filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// Requirement data of a structure, kept until every name can be resolved.
struct StructRaw {
    groups: Vec<Vec<String>>,
    planets: Vec<String>,
    /// Has space-side weapons (hardpoints or a ground-to-space special weapon).
    space_reach: bool,
    unlocks: Vec<String>,
    /// Names of the structures that stand in orbit in a battle in its place (see `StructureType::battle_form`).
    forms: Vec<String>,
}

/// Memoizing converter from hardpoints/projectiles to [`CombatStats`].
struct StatsBuilder<'a> {
    data: &'a GameData,
    damage: &'a DamageMatrix,
    projectiles: HashMap<String, Option<ProjectileDef>>,
}

impl<'a> StatsBuilder<'a> {
    fn new(data: &'a GameData, damage: &'a DamageMatrix) -> Self {
        Self { data, damage, projectiles: HashMap::new() }
    }

    fn projectile(&mut self, name: &str) -> Option<&ProjectileDef> {
        let data = self.data;
        self.projectiles.entry(key(name)).or_insert_with(|| data.objects.projectile(name)).as_ref()
    }

    /// Weapon group for one projectile fired `pulses` times per cycle.
    fn weapon(&mut self, projectile: &str, pulses: u32, cycle: f32, range: f32) -> Option<Weapon> {
        let damage = self.damage;
        let p = self.projectile(projectile)?;
        let dmg = p.damage.max(p.blast_damage);
        if dmg <= 0.0 || !(p.does_shield_damage || p.does_hitpoint_damage) {
            return None;
        }
        let dps = dmg * pulses.max(1) as f32 / cycle.max(0.2);
        Some(Weapon {
            damage_type: p.damage_type.as_deref().and_then(|d| damage.damage(d)),
            hull_dps: if p.does_hitpoint_damage { dps } else { 0.0 },
            shield_dps: if p.does_shield_damage { dps } else { 0.0 },
            range,
        })
    }

    /// The weapon of a ground-to-space structure: its projectile fired `Projectile_Fire_Pulse_Count` times per
    /// `Projectile_Fire_Recharge_Seconds` (plus the pulse delays). The structure has no hull of its own in the fight (it is not a target).
    fn special_weapon(&mut self, fields: &HashMap<String, String>) -> Option<CombatStats> {
        let num = |k: &str| fields.get(k).and_then(|v| v.trim().parse::<f32>().ok());
        let projectile = name_list(fields.get("projectile_types")?).into_iter().next()?;
        let pulses = num("projectile_fire_pulse_count").unwrap_or(1.0).max(1.0) as u32;
        let cycle = num("projectile_fire_recharge_seconds").unwrap_or(80.0) + (pulses - 1) as f32 * num("projectile_fire_pulse_delay_seconds").unwrap_or(0.5);
        let weapon = self.weapon(&projectile, pulses, cycle, 0.0)?;
        let mut stats = CombatStats { hull: 1.0, ..CombatStats::default() };
        stats.add_weapon(weapon);
        Some(stats)
    }

    fn hardpoint_weapon(&mut self, hp: &HardpointDef) -> Option<Weapon> {
        let proj = hp.projectile.as_deref()?;
        let pulses = hp.fire_pulse_count.max(1);
        let recharge = (hp.fire_min_recharge + hp.fire_max_recharge) * 0.5;
        let cycle = recharge + (pulses - 1) as f32 * hp.fire_pulse_delay;
        self.weapon(proj, pulses, cycle, hp.fire_range)
    }

    #[allow(clippy::too_many_arguments)]
    fn stats(
        &mut self,
        hull: f32,
        shield: f32,
        regen: f32,
        armor: Option<&str>,
        shield_armor: Option<&str>,
        hardpoints: &[String],
        direct: Option<&alamo_data::DirectWeapon>,
    ) -> CombatStats {
        let mut s = CombatStats {
            hull,
            shield,
            shield_regen: regen,
            armor: armor.and_then(|a| self.damage.armor(a)),
            shield_armor: shield_armor.and_then(|a| self.damage.armor(a)),
            weapons: Vec::new(),
        };
        let data = self.data;
        for hp in data.objects.hardpoints_of(hardpoints) {
            if let Some(w) = self.hardpoint_weapon(&hp) {
                s.add_weapon(w);
            }
        }
        if let Some(d) = direct
            && let Some(p) = d.projectile_types.first()
        {
            let cycle = d.recharge_seconds + (d.pulse_count.max(1) - 1) as f32 * d.pulse_delay_seconds;
            if let Some(w) = self.weapon(p, d.pulse_count, cycle, 0.0) {
                s.add_weapon(w);
            }
        }
        s
    }
}

/// Collects the production discounts of an object's `Abilities`.
fn production_mods(subs: &[alamo_data::SubAbility]) -> Vec<ProductionMod> {
    let list = |s: Option<&String>| -> Vec<String> {
        s.map(|v| v.split(|c: char| c == ',' || c == '|' || c.is_whitespace()).filter(|t| !t.is_empty()).map(str::to_ascii_lowercase).collect()).unwrap_or_default()
    };
    let num = |v: Option<&String>| v.and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
    let yes = |k: &str, a: &alamo_data::SubAbility| a.fields.get(k).is_some_and(|v| v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true"));
    let mut out = Vec::new();
    for a in subs {
        let is_tech = a.kind.eq_ignore_ascii_case("reduce_technology_price_ability");
        let (price, time) = match a.kind.to_ascii_lowercase().as_str() {
            "reduce_production_price_ability" | "reduce_technology_price_ability" => (num(a.fields.get("price_reduction_percentage")), 0.0),
            "reduce_production_time_ability" => (0.0, num(a.fields.get("time_reduction_percentage"))),
            _ => continue,
        };
        let cats = list(a.fields.get("applicable_unit_categories"));
        let m = ProductionMod {
            name: a.name.clone(),
            price,
            time,
            tech: is_tech,
            galaxy_wide: yes("galaxy_wide", a),
            all: cats.iter().any(|c| c == "all"),
            categories: cats.iter().map(|c| alamo_data::ShipCategory::from_mask(std::slice::from_ref(c))).filter(|c| *c != alamo_data::ShipCategory::Other).collect(),
            types: list(a.fields.get("applicable_unit_types")),
            excluded: list(a.fields.get("excluded_unit_types")),
            stacking: a.fields.get("stacking_category").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
        };
        if m.price != 0.0 || m.time != 0.0 {
            out.push(m);
        }
    }
    out
}

fn unit_from_def(def: &SpaceUnitDef, combat: CombatStats, objects: &alamo_data::GameObjects) -> UnitType {
    let role = match def.name.strip_prefix("Research_").or_else(|| def.name.strip_prefix("RESEARCH_")) {
        Some(target) if def.hardpoints.is_empty() && def.space_model.is_none() => UnitRole::Research(target.to_string()),
        _ => UnitRole::Ship,
    };
    let yes = |k: &str| def.fields.get(k).is_some_and(|v| alamo_data::xml::parse_bool(v));
    UnitType {
        name: def.name.clone(),
        text_id: def.text_id.clone(),
        role,
        category: def.category,
        category_mask: category_bits(&def.category_mask),
        affiliation: def.affiliation.iter().map(|a| key(a)).collect(),
        combat,
        hyperspace_speed: def.hyperspace_speed,
        cost: def.build_cost,
        build_time: def.build_time,
        tech: def.tech_level,
        gui_row: def.fields.get("gui_row").and_then(|v| v.trim().parse().ok()).unwrap_or(1),
        population: def.population_value,
        space_population: def.space_population.unwrap_or(def.population_value),
        req_starbase: def.required_star_base_level,
        req_structures: Vec::new(),
        req_planets: Vec::new(),
        locked: def.build_initially_locked,
        limit_current: def.build_limit_current,
        limit_lifetime: def.build_limit_lifetime,
        hero: yes("is_named_hero") || yes("show_hero_head"),
        prod_mods: production_mods(&def.sub_abilities),
        planet_destroyer: def.sub_abilities.iter().any(|a| a.kind.eq_ignore_ascii_case("planet_destruction_ability")),
        galactic_stealth: def.sub_abilities.iter().any(|a| a.kind.eq_ignore_ascii_case("galactic_stealth_ability")),
        system_spy: def.sub_abilities.iter().any(|a| a.kind.eq_ignore_ascii_case("system_spy_ability")),
        spy: crate::agents::spy_ability(&def.sub_abilities, &def.fields),
        siphon: None,
        req_ground_base: def.fields.get("required_ground_base_level").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
        upkeep: def.upkeep(),
        income_pct: def.income_pct(),
        gamble: def.sub_abilities.iter().find(|a| a.kind.eq_ignore_ascii_case("planet_income_gambling_ability")).map(|a| {
            let num = |k: &str| a.fields.get(k).and_then(|v| v.trim().parse::<f32>().ok());
            IncomeGamble {
                interval: num("evaluation_interval_in_secs").unwrap_or(30.0).max(1.0),
                min: num("min_income_modifier").unwrap_or(0.0),
                max: num("max_income_modifier").unwrap_or(0.0),
                chance: num("chance_to_win").unwrap_or(0.5).clamp(0.0, 1.0),
            }
        }),
        attrition_credits: def.attrition_credits(objects),
        bonuses: def.sub_abilities.iter().filter_map(battle_sim::BonusDef::from_sub_ability).collect(),
        ai_power: def.ai_combat_power,
        buildable: def.build_cost > 0.0 && yes("build_tab_space_units") && !def.affiliation.is_empty(),
        carries: def.starting_spawned_units.clone(),
        members: Vec::new(),
    }
}

impl World {
    pub(crate) fn build_catalogs(&mut self, data: &GameData) {
        let damage = self.damage.clone();
        let mut sb = StatsBuilder::new(data, &damage);

        // Structures first: units and structures reference them.
        let struct_kinds = ["specialstructure", "spacestructure", "techbuilding", "secondarystructure"];
        let mut struct_raw: Vec<StructRaw> = Vec::new();
        for o in data.objects.iter().filter(|o| struct_kinds.contains(&o.kind.to_ascii_lowercase().as_str())) {
            let Some(def) = data.objects.structure(&o.name) else { continue };
            let combat = (!def.hardpoints.is_empty()).then(|| {
                sb.stats(
                    def.hull,
                    def.shield_points,
                    def.fields.get("shield_refresh_rate").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                    def.armor_type.as_deref(),
                    def.fields.get("shield_armor_type").map(String::as_str),
                    &def.hardpoints,
                    None,
                )
            });
            let combat = combat.filter(|c| !c.weapons.is_empty() || c.hull > 0.0);
            let yes = |k: &str| def.fields.get(k).is_some_and(|v| alamo_data::xml::parse_bool(v));
            let special_combat = (combat.is_none() && yes("is_special_weapon_in_space")).then(|| sb.special_weapon(&def.fields)).flatten();
            let orbital = def.base_position.as_deref().is_some_and(|b| b.eq_ignore_ascii_case("orbital"))
                || yes("build_tab_space_station")
                || def.source_file.to_ascii_lowercase().contains("/space/");
            let id = self.structures.len() as StructureTypeId;
            self.structure_index.insert(key(&def.name), id);
            let reach = !def.hardpoints.is_empty() || yes("is_special_weapon_in_space");
            struct_raw.push(StructRaw {
                groups: parse_requirement(def.fields.get("required_special_structures").map_or("", String::as_str)),
                planets: name_list(def.fields.get("required_planets").map_or("", String::as_str)),
                // Space-side weapons: hardpoints, or a ground-to-space special weapon (hypervelocity gun, nuke launcher ...).
                space_reach: reach,
                unlocks: def.unlocks.clone(),
                forms: ["place_other_type_at_every_specific_marker_position", "tactical_additional_structure_type"]
                    .iter()
                    .filter_map(|k| def.fields.get(*k))
                    .filter_map(|v| name_list(v).into_iter().next())
                    .collect(),
            });
            self.structures.push(StructureType {
                name: def.name.clone(),
                text_id: def.text_id.clone(),
                affiliation: def.affiliation.iter().map(|a| key(a)).collect(),
                cost: def.build_cost,
                build_time: def.build_time,
                tech: def.tech_level,
                gui_row: def.fields.get("gui_row").and_then(|v| v.trim().parse().ok()).unwrap_or(1),
                req_starbase: def.required_star_base_level,
                req_structures: Vec::new(),
                req_planets: Vec::new(),
                galactic: orbital,
                space_reach: reach,
                battle_form: None,
                special_combat,
                ancestors: Vec::new(),
                income_abs: def.income_modifiers.iter().map(|m| m.absolute).sum(),
                income_pct: def.income_modifiers.iter().map(|m| m.percentage).sum(),
                income_pct_mining: def.income_modifiers.iter().map(|m| m.percentage * m.mining_colony_multiplier.unwrap_or(0.0)).sum(),
                income_abs_mining: def.income_modifiers.iter().map(|m| m.absolute * m.mining_colony_multiplier.unwrap_or(0.0)).sum(),
                gamble: def.sub_abilities.iter().find(|a| a.kind.eq_ignore_ascii_case("planet_income_gambling_ability")).map(|a| {
                    let num = |k: &str| a.fields.get(k).and_then(|v| v.trim().parse::<f32>().ok());
                    IncomeGamble { interval: num("evaluation_interval_in_secs").unwrap_or(30.0).max(1.0), min: num("min_income_modifier").unwrap_or(0.0), max: num("max_income_modifier").unwrap_or(0.0), chance: num("chance_to_win").unwrap_or(0.5).clamp(0.0, 1.0) }
                }),
                attrition_credits: def.attrition_credits(&data.objects),
                population_capacity: def.population_capacity,
                orbital,
                max_per_planet: def.max_instances_per_planet.unwrap_or(1).max(1),
                limit_current: def.fields.get("build_limit_current_per_player").and_then(|v| v.trim().parse().ok()),
                limit_lifetime: def.fields.get("build_limit_lifetime_per_player").and_then(|v| v.trim().parse().ok()),
                locked: def.build_initially_locked,
                buildable: def.build_cost > 0.0
                    && !def.affiliation.is_empty()
                    && (yes("build_tab_space_station") || yes("build_tab_special_structures")),
                combat,
                prod_mods: production_mods(&def.sub_abilities),
                spy: crate::agents::spy_ability(&def.sub_abilities, &def.fields),
                spawner: None,
                family: Vec::new(),
            });
        }
        for i in 0..self.structures.len() {
            let name = self.structures[i].name.clone();
            if let Some(r) = data.objects.resolve(&name) {
                self.structures[i].ancestors = r.chain.iter().skip(1).filter_map(|o| self.structure_index.get(&key(&o.name)).copied()).collect();
            }
        }
        // An orbital structure without weapons of its own that is replaced by an armed one in the battle (Golan platforms, the Uplink Command)
        // fights with the numbers of that one.
        for (i, raw) in struct_raw.iter().enumerate() {
            let st = &self.structures[i];
            if !st.orbital || st.combat.is_some() || st.space_reach {
                continue;
            }
            let hull = |f: StructureTypeId| self.structures[f as usize].combat.as_ref().map_or(0.0, |c| c.hull);
            let form = raw
                .forms
                .iter()
                .filter_map(|n| self.structure_index.get(&key(n)).copied())
                .filter(|&f| {
                    let t = &self.structures[f as usize];
                    f as usize != i && t.orbital && (t.combat.is_some() || t.space_reach)
                })
                .max_by(|&a, &b| hull(a).total_cmp(&hull(b)));
            if let Some(f) = form {
                let (combat, reach) = (self.structures[f as usize].combat.clone(), self.structures[f as usize].space_reach);
                let st = &mut self.structures[i];
                st.battle_form = Some(f);
                st.combat = combat;
                st.space_reach = reach;
            }
        }
        for (i, raw) in struct_raw.iter().enumerate() {
            let (groups, ok) = self.resolve_requirement(&raw.groups);
            let (planets, planets_ok) = self.resolve_planets(&raw.planets);
            let st = &mut self.structures[i];
            st.req_structures = groups;
            st.req_planets = planets;
            st.buildable &= ok && planets_ok;
        }

        // Units: ships first, then squadrons (which sum their members).
        let defs = data.objects.space_units();
        let mut squadrons = Vec::new();
        for def in defs {
            if def.is_squadron() {
                squadrons.push(def);
                continue;
            }
            let combat = sb.stats(
                def.hull,
                def.shield_points,
                def.shield_refresh_rate,
                def.armor_type.as_deref(),
                def.shield_armor_type.as_deref(),
                &def.hardpoints,
                def.direct_weapon.as_ref(),
            );
            let id = self.units.len() as UnitTypeId;
            self.unit_index.insert(key(&def.name), id);
            let mut u = unit_from_def(&def, combat, &data.objects);
            self.resolve_unit_requirements(&mut u, &def);
            self.units.push(u);
        }
        for def in squadrons {
            let mut combat = CombatStats::default();
            let mut speed = f32::MAX;
            let mut ok = !def.squadron_units.is_empty();
            let mut category = def.category;
            for m in &def.squadron_units {
                let Some(&mid) = self.unit_index.get(&key(m)) else {
                    ok = false;
                    continue;
                };
                let member = &self.units[mid as usize];
                combat.absorb(&member.combat);
                if member.hyperspace_speed > 0.0 {
                    speed = speed.min(member.hyperspace_speed);
                }
                if category == ShipCategory::Other {
                    category = member.category;
                }
            }
            let id = self.units.len() as UnitTypeId;
            self.unit_index.insert(key(&def.name), id);
            let mut u = unit_from_def(&def, combat, &data.objects);
            u.category = category;
            u.hyperspace_speed = if speed == f32::MAX { 0.5 } else { speed };
            u.buildable &= ok;
            if u.income_pct == 0.0 {
                u.income_pct = def.squadron_units.iter().filter_map(|m| self.unit_index.get(&key(m))).map(|&mid| self.units[mid as usize].income_pct).sum();
            }
            if u.gamble.is_none() {
                u.gamble = def.squadron_units.iter().filter_map(|m| self.unit_index.get(&key(m))).find_map(|&mid| self.units[mid as usize].gamble);
            }
            if u.attrition_credits == 0 {
                u.attrition_credits = def.squadron_units.iter().filter_map(|m| self.unit_index.get(&key(m))).map(|&mid| self.units[mid as usize].attrition_credits).sum();
            }
            if u.bonuses.is_empty() {
                u.bonuses = def.squadron_units.iter().filter_map(|m| self.unit_index.get(&key(m))).flat_map(|&mid| self.units[mid as usize].bonuses.clone()).collect();
            }
            u.members = def.squadron_units.iter().filter_map(|m| self.unit_index.get(&key(m)).copied()).collect();
            self.resolve_unit_requirements(&mut u, &def);
            self.units.push(u);
        }
        self.add_vessel_upgrades(data);
        self.add_dummy_purchases(data);
        self.add_agents(data);
        self.classify_ground_structures(&struct_raw);

        // Star bases.
        let sbs = data.objects.star_bases();
        let mut next_names = Vec::new();
        for def in &sbs {
            let combat = sb.stats(
                def.hull,
                def.shield_points,
                def.shield_refresh_rate,
                def.armor_type.as_deref(),
                def.shield_armor_type.as_deref(),
                &def.hardpoints,
                None,
            );
            let garrison = def
                .starting_spawned_units
                .iter()
                .filter_map(|(n, c)| self.unit_index.get(&key(n)).map(|&id| (id, *c)))
                .collect();
            let id = self.starbases.len() as StarbaseTypeId;
            if self.starbase_index.contains_key(&key(&def.name)) {
                next_names.push(None);
                continue;
            }
            self.starbase_index.insert(key(&def.name), id);
            next_names.push(def.next_level.clone());
            self.starbases.push(StarbaseType {
                name: def.name.clone(),
                affiliation: def.affiliation.iter().map(|a| key(a)).collect(),
                level: def.base_level,
                next: None,
                cost: def.build_cost,
                build_time: def.build_time,
                tech: def.tech_level,
                population_capacity: def.population_capacity,
                income_abs: def.income_modifiers.iter().map(|m| m.absolute).sum(),
                income_pct: def.income_modifiers.iter().map(|m| m.percentage).sum(),
                attrition_credits: def.attrition_credits(&data.objects),
                combat,
                garrison,
                prod_mods: production_mods(&def.sub_abilities),
            });
        }
        // `next_names` may be shorter than starbases when duplicates were skipped; align by name.
        for def in &sbs {
            if let (Some(&id), Some(next)) = (self.starbase_index.get(&key(&def.name)), def.next_level.as_ref())
                && let Some(&nid) = self.starbase_index.get(&key(next))
                && self.starbases[id as usize].next.is_none()
            {
                self.starbases[id as usize].next = Some(nid);
            }
        }
        let _ = next_names;
        // Minor factions define their star base levels as separate unlinked objects
        // (`Minor_Starbase_X`, `Minor_Starbase_X_2`, ...): chain them by name stem and level.
        for i in 0..self.starbases.len() {
            if self.starbases[i].next.is_some() || self.starbases[i].cost <= 0.0 {
                continue;
            }
            let stem = base_stem(&self.starbases[i].name);
            let lvl = self.starbases[i].level + 1;
            let aff = self.starbases[i].affiliation.clone();
            let found = (0..self.starbases.len()).find(|&j| {
                j != i && self.starbases[j].level == lvl && self.starbases[j].affiliation == aff && base_stem(&self.starbases[j].name) == stem
            });
            self.starbases[i].next = found.map(|j| j as StarbaseTypeId);
        }
        // How many buildable units each structure unlocks. A dummy purchase spawns no ship, so it does not change that count.
        // Counted before structure swaps, so a Golan I does not score higher for unlocking its own upgrade item
        // (the galaxy AI does not queue those items).
        self.structure_unlocks = vec![0; self.structures.len()];
        for u in &self.units {
            if u.buildable && !matches!(u.role, UnitRole::Dummy) {
                for &r in u.req_structures.iter().flatten() {
                    self.structure_unlocks[r as usize] += 1;
                }
            }
        }
        self.add_structure_upgrades(data);

        // Per-faction buildable lists.
        for f in &self.factions {
            let fname = key(&f.name);
            let units: Vec<UnitTypeId> = (0..self.units.len() as UnitTypeId)
                .filter(|&u| {
                    let t = &self.units[u as usize];
                    t.buildable && t.tech < 90 && t.affiliation.contains(&fname)
                })
                .collect();
            let structs: Vec<StructureTypeId> = (0..self.structures.len() as StructureTypeId)
                .filter(|&s| {
                    let t = &self.structures[s as usize];
                    t.buildable && t.tech < 90 && t.affiliation.contains(&fname)
                })
                .collect();
            let first = self
                .starbases
                .iter()
                .position(|b| {
                    b.level == 1 && b.cost > 0.0 && is_plain_starbase(&b.name) && b.affiliation.contains(&fname)
                })
                .map(|i| i as StarbaseTypeId);
            self.buildable_units.push(units);
            self.buildable_structures.push(structs);
            self.first_starbase.push(first);
        }

        // Innate planet production modifiers (shipyards: Kuat, Rendili, Rothana, etc.)
        for p in &mut self.planets {
            if let Some(def) = data.planet(&p.name) {
                p.prod_mods = production_mods(&def.sub_abilities);
            }
        }
    }

    /// Resolves `Required_Special_Structures` groups; the flag is false if a group has no structure that exists at all.
    pub(crate) fn resolve_requirement(&self, groups: &[Vec<String>]) -> (Vec<Vec<StructureTypeId>>, bool) {
        let mut out = Vec::new();
        let mut ok = true;
        for g in groups {
            let ids: Vec<StructureTypeId> = g.iter().filter_map(|n| self.structure_index.get(&key(n)).copied()).collect();
            if ids.is_empty() {
                ok = false;
            } else {
                out.push(ids);
            }
        }
        (out, ok)
    }

    /// Resolves `Required_Planets`: any one listed planet suffices, so only a list without a single planet of this galaxy blocks the item.
    pub(crate) fn resolve_planets(&self, names: &[String]) -> (Vec<PlanetId>, bool) {
        let ids: Vec<PlanetId> = names.iter().filter_map(|n| self.planet_index.get(&key(n)).copied()).collect();
        let ok = names.is_empty() || !ids.is_empty();
        (ids, ok)
    }

    fn resolve_unit_requirements(&self, u: &mut UnitType, def: &SpaceUnitDef) {
        let (groups, ok) = self.resolve_requirement(&parse_requirement(def.fields.get("required_special_structures").map_or("", String::as_str)));
        let (planets, planets_ok) = self.resolve_planets(&name_list(def.fields.get("required_planets").map_or("", String::as_str)));
        u.req_structures = groups;
        u.req_planets = planets;
        u.buildable &= ok && planets_ok;
    }

    /// Space-tab purchases with no hull and no script (`Rebel_Shield_Upgrade`). [`alamo_data::GameObjects::space_units`] drops them
    /// because they have no `Space_Model_Name`. Vessel upgrades, structure sells and encyclopedia buttons carry a `Lua_Script`, so they are not here.
    /// Price, time, tech, affiliation and requirements stay the object's. Completing one spawns no ship.
    fn add_dummy_purchases(&mut self, data: &GameData) {
        let mut names: Vec<&str> = data.objects.iter().map(|o| o.name.as_str()).collect();
        names.sort();
        for name in names {
            if self.unit_index.contains_key(&key(name)) {
                continue;
            }
            let Some(def) = data.objects.space_unit(name) else { continue };
            let tab = def.fields.get("build_tab_space_units").is_some_and(|v| alamo_data::xml::parse_bool(v));
            let dummy_ship = def.fields.get("behavior").is_some_and(|v| v.split(|c: char| c.is_whitespace() || c == ',').any(|t| t.eq_ignore_ascii_case("DUMMY_STARSHIP")));
            let script = def.fields.get("lua_script").map(|v| v.trim()).unwrap_or("");
            if def.is_squadron() || def.space_model.is_some() || !tab || !dummy_ship || def.build_cost <= 0.0 || !script.is_empty() || def.affiliation.is_empty() {
                continue;
            }
            let mut u = unit_from_def(&def, CombatStats::default(), &data.objects);
            u.role = UnitRole::Dummy;
            self.resolve_unit_requirements(&mut u, &def);
            self.unit_index.insert(key(&def.name), self.units.len() as UnitTypeId);
            self.units.push(u);
        }
    }

    /// Ship and hero upgrade items. They have no space model (`SpaceUnits` drops them), and the target ship is named only by the Remake script
    /// `Scripts/GameObject/Upgrade_Units.lua` (`Upgrade_Unit(planet, player, from, to)`), mirrored here: item, ship that must be in orbit, ship that replaces it.
    fn add_vessel_upgrades(&mut self, data: &GameData) {
        const UPGRADES: &[(&str, &str, &str)] = &[
            ("Vindicator_Upgrade", "Vindicator_Cruiser", "Bizzaro_Cult_Vindicator_Cruiser"),
            ("Gladiator_Upgrade", "Gladiator_Star_Destroyer", "Gladiator_Star_Destroyer_Loadout2"),
            ("Ton_Falk_Upgrade", "Ton_Falk_Carrier", "Ton_Falk_Carrier_Loadout2"),
            ("Executrix_Upgrade", "Havelon", "Executrix_Star_Destroyer"),
            ("Black_Squadron_Upgrade", "Black_Squadron_Team", "Devastator_Star_Destroyer"),
            ("Piett_Procursator_Upgrade", "Piett_Procursator", "Accuser_Star_Destroyer"),
            ("Piett_Accuser_Upgrade", "Accuser_Star_Destroyer", "Piett_Executor_P1"),
            ("Imperial_Piett_Upgrade", "Accuser_Star_Destroyer", "Imperial_Piett_Executor_P1"),
            ("Strikefast_Upgrade", "Strikefast", "Admonitor_Star_Destroyer"),
            ("Admonitor_Upgrade", "Admonitor_Star_Destroyer", "Admonitor_Star_Destroyer_II"),
            ("Quasar_Upgrade", "Quasar_Carrier_R", "Quasar_Carrier_Loadout2"),
            ("Liberator_Upgrade", "Liberator_Carrier_R", "Liberator_Carrier_Loadout2"),
            ("Ackbar_Upgrade", "Ackbar_Wingless", "Home_One"),
            ("Tyber_Zann_Upgrade", "Tyber_Zann_Javelin", "Tyber_Zann_The_Merciless"),
            ("Jared_Sykes_Upgrade", "Sykes_Venator", "End_of_Days"),
            ("Jabba_Upgrade", "Star_Jewel", "Jabba_Destroyer"),
        ];
        // The same script repairs a super ship by one phase: `REPAIR_PHASE_<n>_<ship>` turns phase n into phase n - 1.
        let script = data.upgrade_scripts.iter().find(|(k, _)| k.eq_ignore_ascii_case("UPGRADE_UNITS")).map(|(_, s)| s.as_str()).unwrap_or("");
        let repairs: Vec<(String, String, String)> = crate::structure_upgrades::parse_unit_swaps(script)
            .into_iter()
            .filter(|s| s.actor.to_ascii_uppercase().starts_with("REPAIR_PHASE_"))
            .map(|s| (s.actor, s.from, s.to))
            .collect();
        let all = UPGRADES.iter().map(|&(i, f, t)| (i, f, t)).chain(repairs.iter().map(|(i, f, t)| (i.as_str(), f.as_str(), t.as_str())));
        for (item, from, to) in all {
            let (Some(&f), Some(&t)) = (self.unit_index.get(&key(from)), self.unit_index.get(&key(to))) else { continue };
            if self.unit_index.contains_key(&key(item)) {
                continue;
            }
            let Some(def) = data.objects.space_unit(item) else { continue };
            let mut u = unit_from_def(&def, CombatStats::default(), &data.objects);
            u.role = UnitRole::Upgrade { from: f, to: t };
            u.category = ShipCategory::Other;
            u.hero = false;
            self.resolve_unit_requirements(&mut u, &def);
            self.unit_index.insert(key(&def.name), self.units.len() as UnitTypeId);
            self.units.push(u);
        }
    }

    /// Structure level swaps. `Upgrade_*.lua` names the object the script runs on and the two structures; the price and the requirements stay on
    /// that object when it is a space unit, or on the `GroundCompany` whose `Company_Units` is the ground dummy. Appended after every other unit
    /// so existing ids do not move.
    fn add_structure_upgrades(&mut self, data: &GameData) {
        let mut keys: Vec<&String> = data.upgrade_scripts.keys().collect();
        keys.sort();
        let swaps = crate::structure_upgrades::parse_swaps(keys.iter().filter_map(|k| data.upgrade_scripts.get(*k)));
        let spawners = crate::structure_upgrades::parse_spawners(keys.iter().filter_map(|k| data.upgrade_scripts.get(*k)));

        let mut adj: HashMap<StructureTypeId, Vec<StructureTypeId>> = HashMap::new();
        for swap in &swaps {
            if let (Some(&f), Some(&t)) = (self.structure_index.get(&key(&swap.from)), self.structure_index.get(&key(&swap.to))) {
                adj.entry(f).or_default().push(t);
                adj.entry(t).or_default().push(f);
            }
        }
        for sp in &spawners {
            let Some(&sp_id) = self.structure_index.get(&key(&sp.spawner)) else { continue };
            let Some(&base_id) = self.structure_index.get(&key(&sp.base)) else { continue };
            adj.entry(sp_id).or_default().push(base_id);
            adj.entry(base_id).or_default().push(sp_id);
            for h in &sp.higher {
                if let Some(&h_id) = self.structure_index.get(&key(h)) {
                    adj.entry(sp_id).or_default().push(h_id);
                    adj.entry(h_id).or_default().push(sp_id);
                    adj.entry(base_id).or_default().push(h_id);
                    adj.entry(h_id).or_default().push(base_id);
                }
            }
        }

        let mut visited = HashSet::new();
        for &node in adj.keys() {
            if visited.contains(&node) {
                continue;
            }
            let mut comp = Vec::new();
            let mut q = std::collections::VecDeque::new();
            q.push_back(node);
            visited.insert(node);
            while let Some(curr) = q.pop_front() {
                comp.push(curr);
                if let Some(neighbors) = adj.get(&curr) {
                    for &next in neighbors {
                        if visited.insert(next) {
                            q.push_back(next);
                        }
                    }
                }
            }
            comp.sort_unstable();
            for &id in &comp {
                self.structures[id as usize].family = comp.clone();
            }
        }

        for sp in &spawners {
            let Some(&sp_id) = self.structure_index.get(&key(&sp.spawner)) else { continue };
            let Some(&base_id) = self.structure_index.get(&key(&sp.base)) else { continue };
            let higher: Vec<StructureTypeId> = self.structures[sp_id as usize]
                .family
                .iter()
                .copied()
                .filter(|&id| id != sp_id && id != base_id)
                .collect();
            self.structures[sp_id as usize].spawner = Some(SpawnerRule {
                base: base_id,
                higher,
            });
        }
        let mut companies: HashMap<String, Vec<String>> = HashMap::new();
        for obj in data.objects.of_kind("groundcompany") {
            let Some(r) = data.objects.resolve(&obj.name) else { continue };
            for unit in r.list("company_units") {
                companies.entry(key(&unit)).or_default().push(obj.name.clone());
            }
        }
        for swap in swaps {
            let (Some(&from), Some(&to)) = (self.structure_index.get(&key(&swap.from)), self.structure_index.get(&key(&swap.to))) else { continue };
            if let Some(def) = data.objects.space_unit(&swap.actor) {
                let mut u = unit_from_def(&def, CombatStats::default(), &data.objects);
                self.resolve_unit_requirements(&mut u, &def);
                let max = def.fields.get("build_max_instances_per_planet").and_then(|v| v.trim().parse().ok());
                self.push_structure_swap(u, from, to, swap.downgrade, max);
                continue;
            }
            let Some(names) = companies.get(&key(&swap.actor)) else { continue };
            for name in names {
                let Some(r) = data.objects.resolve(name) else { continue };
                let cost = r.num::<f32>("build_cost_credits").unwrap_or(0.0);
                let affiliation: Vec<String> = r.list("affiliation").into_iter().map(|a| key(&a)).collect();
                let tab = ["build_tab_space_units", "build_tab_land_units", "build_tab_special_structures", "build_tab_space_station"]
                    .iter()
                    .any(|t| r.flag(t).unwrap_or(false));
                let (groups, groups_ok) = self.resolve_requirement(&parse_requirement(r.get("required_special_structures").unwrap_or("")));
                let (planets, planets_ok) = self.resolve_planets(&name_list(r.get("required_planets").unwrap_or("")));
                let buildable = cost > 0.0 && !affiliation.is_empty() && tab && groups_ok && planets_ok;
                let max = r.num::<i32>("build_max_instances_per_planet");
                self.push_structure_swap(
                    UnitType {
                        name: r.name().to_string(),
                        text_id: r.string("text_id"),
                        role: UnitRole::Ship,
                        category: ShipCategory::from_mask(&r.list("categorymask")),
                        category_mask: category_bits(&r.list("categorymask")),
                        affiliation,
                        combat: CombatStats::default(),
                        hyperspace_speed: 0.0,
                        cost,
                        build_time: r.num::<f32>("build_time_seconds").unwrap_or(0.0),
                        tech: r.num::<i32>("tech_level").unwrap_or(0),
                        gui_row: r.num::<i32>("gui_row").unwrap_or(1),
                        population: 0,
                        space_population: 0,
                        req_starbase: r.num::<i32>("required_star_base_level").unwrap_or(0),
                        req_structures: groups,
                        req_planets: planets,
                        locked: r.flag("build_initially_locked").unwrap_or(false),
                        limit_current: r.num("build_limit_current_per_player"),
                        limit_lifetime: r.num("build_limit_lifetime_per_player"),
                        hero: false,
                        prod_mods: Vec::new(),
                        planet_destroyer: false,
                        galactic_stealth: false,
                        system_spy: false,
                        upkeep: 0.0,
                        income_pct: 0.0,
                        gamble: None,
                        attrition_credits: 0,
                        spy: None,
                        siphon: None,
                        bonuses: Vec::new(),
                        req_ground_base: r.num::<i32>("required_ground_base_level").unwrap_or(0),
                        ai_power: 0.0,
                        buildable,
                        carries: Vec::new(),
                        members: Vec::new(),
                    },
                    from,
                    to,
                    swap.downgrade,
                    max,
                );
            }
        }
    }

    fn push_structure_swap(&mut self, mut u: UnitType, from: StructureTypeId, to: StructureTypeId, downgrade: bool, max_per_planet: Option<i32>) {
        if self.unit_index.contains_key(&key(&u.name)) {
            return;
        }
        if !u.req_structures.iter().any(|g| g.contains(&from)) {
            u.req_structures.push(vec![from]);
        }
        u.role = UnitRole::StructureSwap { from, to, downgrade, max_per_planet };
        self.unit_index.insert(key(&u.name), self.units.len() as UnitTypeId);
        self.units.push(u);
    }

    /// Decides which ground structures stay in the build menus (see `StructureType::galactic`).
    fn classify_ground_structures(&mut self, raw: &[StructRaw]) {
        let n = self.structures.len();
        let reaches_ships = |w: &World, m: &ProductionMod| m.all || !m.categories.is_empty() || m.types.iter().any(|t| w.unit_index.contains_key(t));
        let mut keep: Vec<bool> = (0..n)
            .map(|i| {
                let st = &self.structures[i];
                st.orbital
                    || raw[i].space_reach
                    || st.income_abs > 0.0
                    || st.income_pct > 0.0
                    || st.income_abs_mining > 0.0
                    || st.income_pct_mining > 0.0
                    || st.gamble.is_some()
                    || st.population_capacity > 0
                    || st.prod_mods.iter().any(|m| reaches_ships(self, m))
                    || raw[i].unlocks.iter().any(|u| self.unit_index.contains_key(&key(u)))
            })
            .collect();
        // Prerequisites of a kept structure stay (outposts and base cores lead to mines, taxation agencies ...).
        loop {
            let mut changed = false;
            for i in 0..n {
                if !keep[i] {
                    continue;
                }
                for &r in self.structures[i].req_structures.iter().flatten() {
                    if !keep[r as usize] {
                        keep[r as usize] = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for (st, k) in self.structures.iter_mut().zip(keep) {
            st.galactic = k;
            st.buildable &= k;
        }
    }

    pub(crate) fn build_tech_upgrades(&mut self, data: &GameData) {
        for fi in 0..self.factions.len() {
            let fname = key(&self.factions[fi].name);
            let mut chain = self.mod_tech_chain(data, &fname, self.factions[fi].start_tech);
            // Head of the chain: affiliated tech upgrade without a previous level.
            let head = data.objects.of_kind("upgradeobject").find_map(|o| {
                let r = data.objects.resolve(&o.name)?;
                let inc = r.flag("tactical_build_increments_tech_level")?;
                let aff = r.list("affiliation");
                (inc && aff.iter().any(|a| key(a) == fname) && r.get("previous_upgrade_level_type").is_none_or(str::is_empty))
                    .then(|| o.name.clone())
            });
            let mut cur = if chain.is_empty() { head } else { None };
            let mut guard = 0;
            while let Some(name) = cur.take() {
                guard += 1;
                if guard > 10 {
                    break;
                }
                let Some(r) = data.objects.resolve(&name) else { break };
                chain.push(TechUpgrade {
                    name: name.clone(),
                    cost: r.num("tactical_build_cost_campaign").unwrap_or(1000.0),
                    time: r.num("tactical_build_time_seconds").unwrap_or(60.0),
                    req_structures: Vec::new(),
                    req_starbase: 1,
                });
                cur = r.string("next_upgrade_level_type");
            }
            if chain.is_empty() {
                // Factions without upgrade objects (minors) buy generic levels.
                let (start, max) = (self.factions[fi].start_tech, self.factions[fi].max_tech);
                for lvl in start..max {
                    chain.push(TechUpgrade { name: format!("Generic_Tech_{}", lvl + 1), cost: 3000.0 * (lvl + 1) as f32, time: 60.0, req_structures: Vec::new(), req_starbase: 1 });
                }
            }
            self.factions[fi].tech_upgrades = chain;
        }
    }

    /// The Remake way to raise a tech level: upgrade objects with `Build_Advances_Tech_Level` (`E_Tech-2` ... `E_Tech-5`, `CIS_Tech-1` ...), variants of
    /// `Dummy_<F>` which require the faction's `<F>_Research_Institute` and a star base. Price, time and the requirement are the object's; empty when the
    /// data has none for this faction (vanilla chain then).
    fn mod_tech_chain(&self, data: &GameData, faction: &str, start: i32) -> Vec<TechUpgrade> {
        let found: Vec<(i32, String)> = data
            .objects
            .of_kind("upgradeobject")
            .filter_map(|o| {
                let r = data.objects.resolve(&o.name)?;
                let level: i32 = r.num("build_advances_tech_level")?;
                r.list("affiliation").iter().any(|a| key(a) == faction).then(|| (level, o.name.clone()))
            })
            .collect();
        let mut chain = Vec::new();
        for level in start + 1.. {
            let Some((_, name)) = found.iter().find(|(l, _)| *l == level) else { break };
            let Some(r) = data.objects.resolve(name) else { break };
            let (groups, _) = self.resolve_requirement(&parse_requirement(&r.string("required_special_structures").unwrap_or_default()));
            chain.push(TechUpgrade {
                name: name.clone(),
                cost: r.num("build_cost_credits").unwrap_or(1000.0),
                time: r.num("build_time_seconds").unwrap_or(60.0),
                req_structures: groups,
                req_starbase: r.num("required_star_base_level").unwrap_or(1).max(1),
            });
        }
        chain
    }

    pub(crate) fn build_victory(&mut self, campaign: &Campaign) {
        let lookup = |names: &[String], w: &World| -> Vec<PlanetId> {
            names.iter().filter_map(|n| w.planet_index.get(&key(n)).copied()).collect()
        };
        let has = |list: &[String], what: &str| list.iter().any(|c| c.eq_ignore_ascii_case(what));
        let v = VictoryRules {
            active_player: self.config.faction_override.as_deref()
                .or(campaign.starting_active_player.as_deref())
                .and_then(|p| self.faction_index.get(&key(p)).copied()),
            human_named_planets: if has(&campaign.human_victory_conditions, "Galactic_Control_Named_Planets") {
                lookup(&campaign.human_victory_planets, self)
            } else {
                Vec::new()
            },
            human_all_planets: has(&campaign.human_victory_conditions, "Galactic_All_Planets_Controlled"),
            ai_all_planets: has(&campaign.ai_victory_conditions, "Galactic_All_Planets_Controlled"),
            ai_named_planets: if has(&campaign.ai_victory_conditions, "Galactic_Control_Named_Planets") {
                lookup(&campaign.ai_victory_planets, self)
            } else {
                Vec::new()
            },
        };
        self.victory = v;
    }

    /// Faction id by case-insensitive name.
    pub fn faction_id(&self, name: &str) -> Option<FactionId> {
        self.faction_index.get(&key(name)).copied()
    }

    /// Planet id by case-insensitive name.
    pub fn planet_id(&self, name: &str) -> Option<PlanetId> {
        self.planet_index.get(&key(name)).copied()
    }

    /// Unit type id by case-insensitive name.
    pub fn unit_id(&self, name: &str) -> Option<UnitTypeId> {
        self.unit_index.get(&key(name)).copied()
    }

    /// Structure type id by case-insensitive name.
    pub fn structure_id(&self, name: &str) -> Option<StructureTypeId> {
        self.structure_index.get(&key(name)).copied()
    }

    /// True if the two factions are at war.
    pub fn hostile(&self, a: FactionId, b: FactionId) -> bool {
        a != b && self.factions[a as usize].enemies.contains(&b)
    }

    /// Euclidean map distance between two planets.
    pub fn distance(&self, a: PlanetId, b: PlanetId) -> f32 {
        let (pa, pb) = (self.planets[a as usize].position, self.planets[b as usize].position);
        ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2)).sqrt()
    }
}

/// Star base name without a trailing `_<level>` suffix.
fn base_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    match lower.rsplit_once('_') {
        Some((stem, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => stem.to_string(),
        _ => lower,
    }
}

/// Excludes templates, skirmish and tutorial variants and shield gates when picking a faction's base.
fn is_plain_starbase(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    !["skirmish", "template", "default", "tutorial", "gate"].iter().any(|bad| n.contains(bad))
}
