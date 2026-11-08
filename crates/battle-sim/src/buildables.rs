//! The tactical build bar of star bases and stations: the `Tactical_Buildable_Objects_Campaign` entries of a structure (per faction) can
//! be bought during a campaign battle with the side's galactic credits (`Tactical_Build_Cost_Campaign`), one after the other in the
//! side's production queue (`Tactical_Build_Time_Seconds` each). `Tactical_Build_Prerequisites` must be built first and
//! `Build_Limit_Current_For_All_Allies` caps how often an item may be bought. Purchases are `Command`s, so a battle replays the same; the
//! credits spent leave the battle with its result (`BattleResult::attrition_credits`).

use std::collections::HashMap;

use alamo_data::GameData;

use crate::battle::*;

/// Index of a buildable in [`Catalog::buildables`].
pub type BuildId = u32;

/// `Enable_Ability` of an upgrade: switches a disabled ability (`Initially_Enabled` No) of the listed unit types on.
#[derive(Debug, Clone, Default)]
pub struct EnableDef {
    /// `Ability_Name`.
    pub ability: String,
    /// `Applicable_Unit_Types`.
    pub unit_types: Vec<String>,
}

/// An item of a tactical build bar (`UpgradeObject` and the like).
#[derive(Debug, Clone, Default)]
pub struct BuildableDef {
    /// Object name.
    pub name: String,
    /// `Text_ID`.
    pub text_id: Option<String>,
    /// `Icon_Name`.
    pub icon: Option<String>,
    /// `Tactical_Build_Cost_Campaign` in credits (0 where the data has none).
    pub cost: i64,
    /// `Tactical_Build_Time_Seconds`.
    pub time: f32,
    /// `Tactical_Build_Prerequisites`: items this side must have finished in this battle first.
    pub prerequisites: Vec<BuildId>,
    /// `Previous_Upgrade_Level_Type` of an item with `Destroy_Previous_Upgrade_Level`: finishing this one removes that one.
    pub replaces: Option<BuildId>,
    /// `Build_Limit_Current_For_All_Allies` (or `_Per_Player`): how often a side may own it at once (0 = no limit).
    pub limit: u32,
    /// `Combat_Bonus_Ability` sub-abilities (`Space_Automatic`): bonuses for the side's ships while the item stays built.
    pub bonuses: Vec<crate::BonusDef>,
    /// `Enable_Ability` sub-abilities.
    pub enables: Vec<EnableDef>,
    /// `Battlefield_Modifier_Ability` `Stealth_Duration_Multiplier` (1 = none): timed stealth abilities of the side last this much longer.
    pub stealth_duration: f32,
    /// `Lua_Script`.
    pub script: Option<String>,
    /// What `script` spawns when the item is finished, if it is a spawn script (purchased reinforcements; see `spawns.rs`).
    pub spawn: Option<std::sync::Arc<crate::SpawnScriptDef>>,
    /// Auras its debuff dummies leave for the enemy factions (mercenary structures; see [`dummy_debuffs`]).
    pub debuffs: Vec<DebuffDef>,
    /// Seconds from finishing the item to the debuffs (the dummies' countdown).
    pub debuff_delay: f32,
    /// The map marker its script places the dummies on: without one on the map the script spawns nothing.
    pub debuff_marker: Option<String>,
}

/// `Tactical_Buildable_Objects_Campaign` of object `name`: `(faction, item)` pairs, faction lower case. The tag is a list in which a
/// faction name starts the section of that faction's items (`Empire, A, B, Rebel, C`).
pub(crate) fn buildable_lists(data: &GameData, name: &str) -> Vec<(String, String)> {
    let Some(r) = data.objects.resolve(name) else { return Vec::new() };
    let mut out = Vec::new();
    for tag in r.get_all("tactical_buildable_objects_campaign") {
        let mut faction: Option<String> = None;
        for tok in alamo_data::xml::split_list(tag) {
            if data.faction(&tok).is_some() {
                faction = Some(tok.to_ascii_lowercase());
            } else if let Some(f) = &faction
                && !out.iter().any(|(g, n): &(String, String)| g == f && n.eq_ignore_ascii_case(&tok))
            {
                out.push((f.clone(), tok));
            }
        }
    }
    out
}

/// Builds the definitions of `items` (object names), in that order; references between them (prerequisites, previous levels) are
/// resolved by name, so every item they name must be in the list (callers pass the closure).
pub fn buildable_defs(data: &GameData, items: &[String]) -> Vec<BuildableDef> {
    let index: HashMap<String, BuildId> = items.iter().enumerate().map(|(i, n)| (n.to_ascii_lowercase(), i as BuildId)).collect();
    let id = |n: &str| index.get(&n.trim().to_ascii_lowercase()).copied();
    items
        .iter()
        .map(|name| {
            let Some(r) = data.objects.resolve(name) else { return BuildableDef { name: name.clone(), ..BuildableDef::default() } };
            let num = |t: &str| r.num::<f32>(t);
            let subs = data.objects.sub_abilities(name);
            let replaces = r.flag("destroy_previous_upgrade_level").unwrap_or(false).then(|| r.get("previous_upgrade_level_type").and_then(id)).flatten();
            BuildableDef {
                name: r.name().to_string(),
                text_id: r.string("text_id"),
                icon: r.string("icon_name"),
                cost: num("tactical_build_cost_campaign").unwrap_or(0.0).max(0.0).round() as i64,
                time: num("tactical_build_time_seconds").unwrap_or(0.0).max(0.0),
                prerequisites: r.list("tactical_build_prerequisites").iter().filter_map(|p| id(p)).collect(),
                replaces,
                limit: num("build_limit_current_for_all_allies").or_else(|| num("build_limit_current_per_player")).unwrap_or(0.0).max(0.0) as u32,
                bonuses: subs.iter().filter_map(crate::BonusDef::from_sub_ability).collect(),
                enables: subs
                    .iter()
                    .filter(|a| a.kind.eq_ignore_ascii_case("Enable_Ability"))
                    .filter_map(|a| {
                        let ability = a.fields.get("ability_name")?.trim().to_string();
                        let unit_types = a.fields.get("applicable_unit_types").map(|v| alamo_data::xml::split_list(v)).unwrap_or_default();
                        Some(EnableDef { ability, unit_types })
                    })
                    .collect(),
                stealth_duration: subs
                    .iter()
                    .filter(|a| a.kind.eq_ignore_ascii_case("Battlefield_Modifier_Ability"))
                    .filter_map(|a| a.fields.get("stealth_duration_multiplier").and_then(|v| alamo_data::xml::parse_num::<f32>(v)))
                    .fold(1.0, f32::max),
                script: r.string("lua_script").filter(|s| !s.trim().is_empty()),
                spawn: None,
                debuffs: Vec::new(),
                debuff_delay: 0.0,
                debuff_marker: None,
            }
        })
        .collect()
}

/// The aura a debuff dummy leaves behind (see [`dummy_debuffs`]): `bonus` acts on the ships of the side whose faction is `faction`.
#[derive(Debug, Clone, Default)]
pub struct DebuffDef {
    /// Lower-case faction name (the `Affiliation` of the aura object).
    pub faction: String,
    /// Its `Combat_Bonus_Ability`.
    pub bonus: crate::BonusDef,
}

/// A mercenary debuff dummy (`Destruction_Space_Dummy` variants such as `Empire_Enemies_-7%_Damage`): it destroys itself after its
/// `SELF_DESTRUCT` `Expiration_Seconds` and leaves its `Destruction_Survivors`, one aura object per enemy faction. Returns that delay and
/// the auras; `None` if `name` is no such dummy.
pub(crate) fn dummy_debuffs(data: &GameData, name: &str) -> Option<(f32, Vec<DebuffDef>)> {
    let unit = data.objects.space_unit(name)?;
    let countdown = unit.abilities.iter().find(|a| a.kind.eq_ignore_ascii_case("SELF_DESTRUCT"))?.expiration_seconds.unwrap_or(0.0).max(0.0);
    let survivors = unit.destruction_survivors.as_deref()?;
    let toks = alamo_data::xml::split_list(survivors);
    let mut out = Vec::new();
    for survivor in toks.iter().filter(|t| alamo_data::xml::parse_num::<f32>(t).is_none()) {
        let Some(r) = data.objects.resolve(survivor) else { continue };
        let bonuses: Vec<crate::BonusDef> = data.objects.sub_abilities(survivor).iter().filter_map(crate::BonusDef::from_sub_ability).collect();
        for faction in r.list("affiliation") {
            out.extend(bonuses.iter().map(|b| DebuffDef { faction: faction.to_ascii_lowercase(), bonus: b.clone() }));
        }
    }
    (!out.is_empty()).then_some((countdown, out))
}

/// Every item reachable from `roots` through prerequisites and previous levels, roots first.
pub(crate) fn buildable_closure(data: &GameData, roots: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut queue: Vec<String> = roots.into_iter().collect();
    while let Some(n) = queue.pop() {
        if out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
            continue;
        }
        if let Some(r) = data.objects.resolve(&n) {
            queue.extend(r.list("tactical_build_prerequisites"));
            queue.extend(r.string("previous_upgrade_level_type"));
        }
        out.push(n);
    }
    out.sort_by_key(|n| n.to_ascii_lowercase());
    out
}

/// One button of a side's build bar (see [`Battle::build_options`]).
#[derive(Debug, Clone, PartialEq)]
pub struct BuildOption {
    pub item: BuildId,
    /// May it be bought now (why not).
    pub ready: Result<(), CommandError>,
    /// Build progress 0..1 while it is in the queue.
    pub progress: Option<f32>,
}

/// A purchase in a side's production queue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BuildJob {
    pub side: u8,
    pub item: BuildId,
    /// Seconds of building left (only the first job of a side progresses).
    pub left: f32,
}

impl Battle {
    /// Credits `side` has left to spend on tactical builds.
    pub fn credits(&self, side: u8) -> i64 {
        self.credits.get(side as usize).copied().unwrap_or(0)
    }

    /// Items the living structures of `side` offer to its faction, in catalog order.
    pub fn buildable(&self, side: u8) -> Vec<BuildId> {
        let Some(faction) = self.sides.get(side as usize).map(|s| s.name.to_ascii_lowercase()) else { return Vec::new() };
        let mut out: Vec<BuildId> = self
            .ships
            .iter()
            .filter(|s| s.side == side && s.alive && !s.retreated)
            .flat_map(|s| self.catalog.ship(s.def).buildable.iter().filter(|(f, _)| *f == faction).map(|&(_, b)| b))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Items `side` has finished in this battle (an item replaced by a later level is gone).
    pub fn built(&self, side: u8) -> &[BuildId] {
        self.built.get(side as usize).map_or(&[], Vec::as_slice)
    }

    /// The debuff auras that act on `side` now (mercenary structures of its enemies).
    pub fn debuffs_on(&self, side: u8) -> Vec<&crate::BonusDef> {
        let Some(faction) = self.sides.get(side as usize).map(|s| s.name.to_ascii_lowercase()) else { return Vec::new() };
        self.debuffs
            .iter()
            .filter(|&&(at, _)| at <= self.time)
            .flat_map(|&(_, b)| self.catalog.buildables[b as usize].debuffs.iter())
            .filter(|d| d.faction == faction)
            .map(|d| &d.bonus)
            .collect()
    }

    /// True if a built item of `side` enables ability `ability` of unit type `unit` (`Enable_Ability`).
    pub(crate) fn build_enables(&self, side: u8, unit: &str, ability: &str) -> bool {
        self.built(side).iter().flat_map(|&b| self.catalog.buildables[b as usize].enables.iter()).any(|e| e.ability.eq_ignore_ascii_case(ability) && e.unit_types.iter().any(|t| t.eq_ignore_ascii_case(unit)))
    }

    /// Stealth duration multiplier from the items `side` has built (the strongest one).
    pub(crate) fn build_stealth_duration(&self, side: u8) -> f32 {
        self.built(side).iter().map(|&b| self.catalog.buildables[b as usize].stealth_duration).fold(1.0, f32::max)
    }

    /// The production queue of `side`: items and seconds left, the one being built first.
    pub fn build_queue(&self, side: u8) -> Vec<(BuildId, f32)> {
        self.build_jobs.iter().filter(|j| j.side == side).map(|j| (j.item, j.left)).collect()
    }

    /// What the build bar of `side` shows, in catalog order: every offered item that is not built (or replaced by a later level) and whose
    /// prerequisites are built or queued, so of an upgrade line only the next level. `ready` is [`Battle::build_check`]; `progress` is
    /// 0..1 for the first job of the item in the queue.
    pub fn build_options(&self, side: u8) -> Vec<BuildOption> {
        let built = self.built(side);
        let queued = |p: BuildId| self.build_jobs.iter().any(|j| j.side == side && j.item == p);
        self.buildable(side)
            .into_iter()
            .filter(|&item| {
                let def = &self.catalog.buildables[item as usize];
                let superseded = built.iter().any(|&b| self.catalog.buildables[b as usize].replaces == Some(item));
                !built.contains(&item) && !superseded && def.prerequisites.iter().all(|&p| built.contains(&p) || queued(p))
            })
            .map(|item| {
                let time = self.catalog.buildables[item as usize].time;
                let progress = self.build_jobs.iter().find(|j| j.side == side && j.item == item).map(|j| if time > 0.0 { (1.0 - j.left / time).clamp(0.0, 1.0) } else { 0.0 });
                BuildOption { item, ready: self.build_check(side, item), progress }
            })
            .collect()
    }

    /// Whether `side` may queue `item` now (the reason when not).
    pub fn build_check(&self, side: u8, item: BuildId) -> Result<(), CommandError> {
        let def = self.catalog.buildables.get(item as usize).ok_or(CommandError::NotBuildable)?;
        if !self.buildable(side).contains(&item) {
            return Err(CommandError::NotBuildable);
        }
        let owned = self.built(side).iter().filter(|&&b| b == item).count() + self.build_jobs.iter().filter(|j| j.side == side && j.item == item).count();
        // An item a later level replaced still counts as had: it cannot be bought again.
        let superseded = self.built(side).iter().any(|&b| self.catalog.buildables[b as usize].replaces == Some(item));
        if (def.limit > 0 && owned >= def.limit as usize) || superseded {
            return Err(CommandError::BuildLimit);
        }
        let queued = |p: BuildId| self.build_jobs.iter().any(|j| j.side == side && j.item == p);
        if def.prerequisites.iter().any(|&p| !self.built(side).contains(&p) && !queued(p)) {
            return Err(CommandError::MissingPrerequisite);
        }
        if self.credits(side) < def.cost {
            return Err(CommandError::NotEnoughCredits);
        }
        Ok(())
    }

    /// `Command::TacticalBuild`: pays and queues the item.
    pub(crate) fn queue_build(&mut self, side: u8, item: BuildId) -> Result<(), CommandError> {
        self.build_check(side, item)?;
        let def = &self.catalog.buildables[item as usize];
        self.credits[side as usize] -= def.cost;
        self.spent[side as usize] += def.cost;
        self.build_jobs.push(BuildJob { side, item, left: def.time });
        if self.config.record_events {
            self.cmd_events.push(Event::BuildQueued { side, item });
        }
        Ok(())
    }

    /// `Command::CancelTacticalBuild`: the last queued job of the item leaves the queue and its credits come back.
    pub(crate) fn cancel_build(&mut self, side: u8, item: BuildId) -> Result<(), CommandError> {
        let k = self.build_jobs.iter().rposition(|j| j.side == side && j.item == item).ok_or(CommandError::NotBuildable)?;
        self.build_jobs.remove(k);
        self.refund(side, item);
        if self.config.record_events {
            self.cmd_events.push(Event::BuildCancelled { side, item });
        }
        Ok(())
    }

    fn refund(&mut self, side: u8, item: BuildId) {
        let cost = self.catalog.buildables[item as usize].cost;
        self.credits[side as usize] += cost;
        self.spent[side as usize] -= cost;
    }

    /// Builds the first job of every side's queue. A side that has lost every structure offering an item loses its queue and gets the
    /// credits back (estimate: the data does not say what happens to a destroyed station's queue).
    pub(crate) fn run_builds(&mut self, dt: f32) {
        if self.build_jobs.is_empty() {
            return;
        }
        for side in 0..self.sides.len() as u8 {
            let offered = self.buildable(side);
            let gone: Vec<usize> = (0..self.build_jobs.len()).filter(|&k| self.build_jobs[k].side == side && !offered.contains(&self.build_jobs[k].item)).collect();
            for &k in gone.iter().rev() {
                let item = self.build_jobs.remove(k).item;
                self.refund(side, item);
                if self.config.record_events {
                    self.events.push(Event::BuildCancelled { side, item });
                }
            }
            let Some(k) = self.build_jobs.iter().position(|j| j.side == side) else { continue };
            self.build_jobs[k].left -= dt;
            if self.build_jobs[k].left > 1e-4 {
                continue;
            }
            let item = self.build_jobs.remove(k).item;
            self.finish_build(side, item);
        }
    }

    fn finish_build(&mut self, side: u8, item: BuildId) {
        let replaces = self.catalog.buildables[item as usize].replaces;
        let built = &mut self.built[side as usize];
        if let Some(old) = replaces {
            built.retain(|&b| b != old);
        }
        built.push(item);
        if let Some(script) = self.catalog.buildables[item as usize].spawn.clone() {
            // The script runs on the structure that offers the item (`Find_Player` decides the side of the units).
            let faction = self.sides[side as usize].name.to_ascii_lowercase();
            let owner = self.ships.iter().find(|s| s.side == side && s.alive && !s.retreated && self.catalog.ship(s.def).buildable.iter().any(|(f, b)| *f == faction && *b == item));
            if let Some(owner) = owner.map(|s| s.id) {
                self.run_spawn_script(owner, &script, 0x8000 | item as u64);
            }
        }
        let def = &self.catalog.buildables[item as usize];
        if !def.debuffs.is_empty() {
            let marker_ok = def.debuff_marker.as_ref().is_none_or(|m| self.config.arena.as_ref().is_some_and(|a| a.markers.iter().any(|k| k.name.eq_ignore_ascii_case(m))));
            if marker_ok {
                self.debuffs.push((self.time + def.debuff_delay, item));
            }
        }
        if self.config.record_events {
            self.events.push(Event::BuildFinished { side, item });
        }
    }
}
