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

/// What a finished purchase does.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum BuildEffect {
    /// Nothing the battle models.
    #[default]
    None,
}

/// An item of a tactical build bar (`UpgradeObject` and the like).
#[derive(Debug, Clone, Default, PartialEq)]
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
    /// What it does when finished.
    pub effect: BuildEffect,
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
pub(crate) fn buildable_defs(data: &GameData, items: &[String]) -> Vec<BuildableDef> {
    let index: HashMap<String, BuildId> = items.iter().enumerate().map(|(i, n)| (n.to_ascii_lowercase(), i as BuildId)).collect();
    let id = |n: &str| index.get(&n.trim().to_ascii_lowercase()).copied();
    items
        .iter()
        .map(|name| {
            let Some(r) = data.objects.resolve(name) else { return BuildableDef { name: name.clone(), ..BuildableDef::default() } };
            let num = |t: &str| r.num::<f32>(t);
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
                effect: BuildEffect::None,
            }
        })
        .collect()
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

    /// The production queue of `side`: items and seconds left, the one being built first.
    pub fn build_queue(&self, side: u8) -> Vec<(BuildId, f32)> {
        self.build_jobs.iter().filter(|j| j.side == side).map(|j| (j.item, j.left)).collect()
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
        let def = self.catalog.buildables[item as usize].clone();
        let built = &mut self.built[side as usize];
        if let Some(old) = def.replaces {
            built.retain(|&b| b != old);
        }
        built.push(item);
        match &def.effect {
            BuildEffect::None => {}
        }
        if self.config.record_events {
            self.events.push(Event::BuildFinished { side, item });
        }
    }
}
