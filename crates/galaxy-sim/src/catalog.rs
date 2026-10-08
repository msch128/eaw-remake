//! Builds the unit, structure and star base catalogs from the object definitions.

use std::collections::HashMap;

use alamo_data::{Campaign, GameData, HardpointDef, ProjectileDef, ShipCategory, SpaceUnitDef};

use crate::world::*;

fn key(s: &str) -> String {
    s.trim().to_ascii_lowercase()
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
    let mut out = Vec::new();
    for a in subs {
        let (price, time) = match a.kind.to_ascii_lowercase().as_str() {
            "reduce_production_price_ability" => (num(a.fields.get("price_reduction_percentage")), 0.0),
            "reduce_production_time_ability" => (0.0, num(a.fields.get("time_reduction_percentage"))),
            _ => continue,
        };
        let cats = list(a.fields.get("applicable_unit_categories"));
        let m = ProductionMod {
            name: a.name.clone(),
            price,
            time,
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

fn unit_from_def(def: &SpaceUnitDef, combat: CombatStats) -> UnitType {
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
        affiliation: def.affiliation.iter().map(|a| key(a)).collect(),
        combat,
        hyperspace_speed: def.hyperspace_speed,
        cost: def.build_cost,
        build_time: def.build_time,
        tech: def.tech_level,
        population: def.population_value,
        req_starbase: def.required_star_base_level,
        req_structures: Vec::new(),
        req_planets: Vec::new(),
        locked: def.build_initially_locked,
        limit_current: def.build_limit_current,
        limit_lifetime: def.build_limit_lifetime,
        hero: yes("is_named_hero"),
        prod_mods: production_mods(&def.sub_abilities),
        planet_destroyer: def.sub_abilities.iter().any(|a| a.kind.eq_ignore_ascii_case("planet_destruction_ability")),
        upkeep: def.upkeep(),
        ai_power: def.ai_combat_power,
        buildable: def.build_cost > 0.0 && yes("build_tab_space_units") && !def.affiliation.is_empty(),
        carries: def.starting_spawned_units.clone(),
    }
}

impl World {
    pub(crate) fn build_catalogs(&mut self, data: &GameData) {
        let damage = self.damage.clone();
        let mut sb = StatsBuilder::new(data, &damage);

        // Structures first: units and structures reference them.
        let struct_kinds = ["specialstructure", "spacestructure", "techbuilding", "secondarystructure"];
        let mut struct_reqs: Vec<Vec<String>> = Vec::new();
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
            let orbital = def.base_position.as_deref().is_some_and(|b| b.eq_ignore_ascii_case("orbital"))
                || yes("build_tab_space_station")
                || def.source_file.to_ascii_lowercase().contains("/space/");
            let id = self.structures.len() as StructureTypeId;
            self.structure_index.insert(key(&def.name), id);
            struct_reqs.push(def.required_special_structures.clone());
            self.structures.push(StructureType {
                name: def.name.clone(),
                text_id: def.text_id.clone(),
                affiliation: def.affiliation.iter().map(|a| key(a)).collect(),
                cost: def.build_cost,
                build_time: def.build_time,
                tech: def.tech_level,
                req_starbase: def.required_star_base_level,
                req_structures: Vec::new(),
                ancestors: Vec::new(),
                income_abs: def.income_modifiers.iter().map(|m| m.absolute).sum(),
                income_pct: def.income_modifiers.iter().map(|m| m.percentage).sum(),
                income_pct_mining: def.income_modifiers.iter().map(|m| m.percentage * m.mining_colony_multiplier.unwrap_or(0.0)).sum(),
                income_abs_mining: def.income_modifiers.iter().map(|m| m.absolute * m.mining_colony_multiplier.unwrap_or(0.0)).sum(),
                gamble: def.sub_abilities.iter().find(|a| a.kind.eq_ignore_ascii_case("planet_income_gambling_ability")).map(|a| {
                    let num = |k: &str| a.fields.get(k).and_then(|v| v.trim().parse::<f32>().ok());
                    IncomeGamble { interval: num("evaluation_interval_in_secs").unwrap_or(30.0).max(1.0), min: num("min_income_modifier").unwrap_or(0.0), max: num("max_income_modifier").unwrap_or(0.0), chance: num("chance_to_win").unwrap_or(0.5).clamp(0.0, 1.0) }
                }),
                population_capacity: def.population_capacity,
                orbital,
                max_per_planet: def.max_instances_per_planet.unwrap_or(1).max(1),
                locked: def.build_initially_locked,
                buildable: def.build_cost > 0.0
                    && !def.affiliation.is_empty()
                    && (yes("build_tab_space_station") || yes("build_tab_special_structures")),
                combat,
                prod_mods: production_mods(&def.sub_abilities),
            });
        }
        for i in 0..self.structures.len() {
            let name = self.structures[i].name.clone();
            if let Some(r) = data.objects.resolve(&name) {
                self.structures[i].ancestors = r.chain.iter().skip(1).filter_map(|o| self.structure_index.get(&key(&o.name)).copied()).collect();
            }
        }
        for (i, reqs) in struct_reqs.iter().enumerate() {
            let mut ids = Vec::new();
            let mut missing = false;
            for r in reqs {
                match self.structure_index.get(&key(r)) {
                    Some(&id) => ids.push(id),
                    None => missing = true,
                }
            }
            self.structures[i].req_structures = ids;
            if missing {
                self.structures[i].buildable = false;
            }
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
            let mut u = unit_from_def(&def, combat);
            u.req_structures = def.required_special_structures.iter().filter_map(|s| self.structure_index.get(&key(s)).copied()).collect();
            if u.req_structures.len() != def.required_special_structures.len() {
                u.buildable = false;
            }
            u.req_planets = def.required_planets.iter().filter_map(|p| self.planet_index.get(&key(p)).copied()).collect();
            if u.req_planets.len() != def.required_planets.len() {
                // Restricted to a planet that is not in this galaxy.
                u.buildable = false;
            }
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
            let mut u = unit_from_def(&def, combat);
            u.category = category;
            u.hyperspace_speed = if speed == f32::MAX { 0.5 } else { speed };
            u.buildable &= ok;
            u.req_structures = def.required_special_structures.iter().filter_map(|s| self.structure_index.get(&key(s)).copied()).collect();
            if u.req_structures.len() != def.required_special_structures.len() {
                u.buildable = false;
            }
            u.req_planets = def.required_planets.iter().filter_map(|p| self.planet_index.get(&key(p)).copied()).collect();
            if u.req_planets.len() != def.required_planets.len() {
                u.buildable = false;
            }
            self.units.push(u);
        }

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
                combat,
                garrison,
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
        // How many buildable units each structure unlocks.
        self.structure_unlocks = vec![0; self.structures.len()];
        for u in &self.units {
            if u.buildable {
                for &r in &u.req_structures {
                    self.structure_unlocks[r as usize] += 1;
                }
            }
        }

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
    }

    pub(crate) fn build_tech_upgrades(&mut self, data: &GameData) {
        for fi in 0..self.factions.len() {
            let fname = key(&self.factions[fi].name);
            let mut chain = Vec::new();
            // Head of the chain: affiliated tech upgrade without a previous level.
            let head = data.objects.of_kind("upgradeobject").find_map(|o| {
                let r = data.objects.resolve(&o.name)?;
                let inc = r.flag("tactical_build_increments_tech_level")?;
                let aff = r.list("affiliation");
                (inc && aff.iter().any(|a| key(a) == fname) && r.get("previous_upgrade_level_type").is_none_or(str::is_empty))
                    .then(|| o.name.clone())
            });
            let mut cur = head;
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
                });
                cur = r.string("next_upgrade_level_type");
            }
            if chain.is_empty() {
                // Factions without upgrade objects (minors) buy generic levels.
                let (start, max) = (self.factions[fi].start_tech, self.factions[fi].max_tech);
                for lvl in start..max {
                    chain.push(TechUpgrade { name: format!("Generic_Tech_{}", lvl + 1), cost: 3000.0 * (lvl + 1) as f32, time: 60.0 });
                }
            }
            self.factions[fi].tech_upgrades = chain;
        }
    }

    pub(crate) fn build_victory(&mut self, campaign: &Campaign) {
        let lookup = |names: &[String], w: &World| -> Vec<PlanetId> {
            names.iter().filter_map(|n| w.planet_index.get(&key(n)).copied()).collect()
        };
        let has = |list: &[String], what: &str| list.iter().any(|c| c.eq_ignore_ascii_case(what));
        let v = VictoryRules {
            active_player: campaign.starting_active_player.as_ref().and_then(|p| self.faction_index.get(&key(p)).copied()),
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
