//! [`ScriptHost`] implementation on top of galaxy-sim's public API.
//!
//! # Object model
//!
//! galaxy-sim has no object identities below fleets, so scripts see *virtual objects*:
//!
//! | kind | id | represents |
//! |---|---|---|
//! | planet | `1<<56 \| planet` | a planet (also dummy locations) |
//! | ship group | `2<<56 \| fleet<<24 \| unit_type` | all ships of one type in one fleet |
//! | structure | `3<<56 \| planet<<24 \| structure_type` | a built structure |
//! | star base | `4<<56 \| planet` | the star base of a planet |
//!
//! Players are factions (id = faction id); the extra *Neutral* player (id = number of factions) owns
//! unowned planets.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use alamo_data::{GameData, GameObjects};
use alamo_lua::{
    HostValue, ObjectFilter, ObjectId, ObjectInfo, PlanEvent, PlayerId, PlayerInfo, ScriptHost, SpawnAt, StoryReward,
    TaskForceSpec,
};
use galaxy_sim::{FleetLocation, Sim, UnitRole, World};

/// Something the app may want to show or play, produced by scripts.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptEvent {
    /// `Game_Message` text.
    Message(String),
    /// A story dialog (advisor / transmission): title, lines and optional sound event, text resolved.
    Dialog {
        /// Dialog file (`Dialog_Intro`).
        dialog: String,
        /// Chapter index.
        chapter: i32,
        /// Title text.
        title: String,
        /// Text lines.
        lines: Vec<String>,
        /// `SFX` event named by the dialog, if any.
        sfx: Option<String>,
        /// `SPEECH` event named by the dialog, if any.
        speech: Option<String>,
        /// Story event that raised it.
        event: String,
        /// Faction whose plot raised it.
        faction: Option<String>,
    },
    /// A hero or unique unit appeared at a planet.
    UnitSpawned {
        /// Unit type.
        unit: String,
        /// Planet name.
        planet: String,
        /// Faction name.
        faction: String,
    },
    /// An object became buildable / was locked for a faction (story research and unlock rewards).
    BuildableChanged {
        /// Object name.
        object: String,
        /// Faction name.
        faction: String,
        /// New state.
        buildable: bool,
    },
    /// The unit-filter scripts hide or show a unit in the build menu (UI only, not a game rule).
    BuildMenuFilter {
        /// Object name.
        object: String,
        /// True when hidden.
        hidden: bool,
    },
    /// A planet changed hands through a story reward.
    PlanetChanged {
        /// Planet name.
        planet: String,
        /// New owner.
        faction: String,
    },
    /// A story reward that has no simulation effect yet.
    UnhandledReward(StoryReward),
}

/// Counters of how scripts affected the simulation.
#[derive(Debug, Clone, Default)]
pub struct HostStats {
    /// Units added to the sim by scripts.
    pub spawned: u32,
    /// Spawn requests that reached the host.
    pub spawn_requests: u32,
    /// How spawn requests ended: ship, structure, structure_duplicate, no_owner_faction, not_simulated.
    pub spawn_outcomes: BTreeMap<&'static str, u32>,
    /// Structures added to planets by scripts (duplicates on a planet are merged).
    pub structures_added: u32,
    /// Spawn requests for types the sim cannot represent (ground units, markers, props), by type.
    pub spawn_ignored: BTreeMap<String, u32>,
    /// Story rewards applied, by kind.
    pub rewards_applied: BTreeMap<String, u32>,
    /// Story rewards without effect, by kind.
    pub rewards_ignored: BTreeMap<String, u32>,
    /// Credits given by scripts.
    pub credits_given: f64,
}

/// Reads a story dialog text file by name.
pub type DialogReader = Rc<dyn Fn(&str) -> Option<String>>;

/// State shared between the host (called from Lua) and the stepper.
pub struct Inner {
    /// The simulation.
    pub sim: Sim,
    /// Immutable world.
    pub world: Arc<World>,
    /// Faction controlled by the human.
    pub human: u32,
    /// Allied pairs created by scripts.
    pub allies: HashSet<(u32, u32)>,
    /// Game object definitions (types, combat ratings).
    pub objects: GameObjects,
    /// Localized texts.
    pub data: Arc<GameData>,
    /// Events for the app.
    pub events: Vec<ScriptEvent>,
    /// Counters.
    pub stats: HostStats,
    /// Dialog texts: `(dialog file, chapter)` -> (title, lines, sfx, speech).
    pub dialogs: DialogReader,
    /// The Remake AI, if enabled.
    pub ai: Option<galaxy_ai::AiEngine>,
    /// DebugMessage output of scripts (first 3000 lines).
    pub script_log: Vec<String>,
    type_cache: std::collections::HashMap<(String, String), Option<f32>>,
    /// Sim events caused by script calls (captures, tech); `ScriptedSim::step` reports them with the next step.
    pub pending_events: Vec<galaxy_sim::Event>,
}

impl Inner {
    /// Creates the shared state.
    pub fn new(
        sim: Sim,
        human: u32,
        objects: GameObjects,
        data: Arc<GameData>,
        dialogs: DialogReader,
    ) -> Self {
        let world = sim.world().clone();
        Self {
            sim,
            world,
            human,
            allies: HashSet::new(),
            objects,
            data,
            events: Vec::new(),
            stats: HostStats::default(),
            dialogs,
            ai: None,
            script_log: Vec::new(),
            type_cache: std::collections::HashMap::new(),
            pending_events: Vec::new(),
        }
    }

    fn neutral(&self) -> u32 {
        self.world.factions.len() as u32
    }

    fn planet_name(&self, p: u32) -> String {
        self.world.planets[p as usize].name.clone()
    }

    /// Resolves a planet name as scripts write it (`Coruscant`) to a planet id; the world names carry
    /// campaign suffixes (`Coruscant_WW`).
    pub fn find_planet(&self, name: &str) -> Option<u32> {
        let key = name.trim().to_ascii_lowercase();
        if let Some(&p) = self.world.planet_index.get(&key) {
            return Some(p);
        }
        for suffix in ["_ww", "_dummy_ww"] {
            if let Some(&p) = self.world.planet_index.get(&format!("{key}{suffix}")) {
                return Some(p);
            }
        }
        let prefix = format!("{key}_");
        self.world
            .planet_index
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, &p)| p)
            .min()
    }

    fn faction_by_name(&self, name: &str) -> Option<u32> {
        self.world
            .faction_index
            .get(&name.trim().to_ascii_lowercase())
            .copied()
    }

    fn add_ships(&mut self, owner: u32, planet: u32, unit: u32, count: u32) -> u64 {
        let fleet = self.sim.spawn_units(owner, planet, unit, count);
        self.stats.spawned += count;
        ship_id(fleet, unit)
    }

    fn unlock(&mut self, faction: u32, object: &str, on: bool) {
        self.sim.set_unlocked(faction, object, on);
        self.events.push(ScriptEvent::BuildableChanged {
            object: object.to_string(),
            faction: self.world.factions[faction as usize].name.clone(),
            buildable: on,
        });
    }

    fn remove_units(&mut self, faction: u32, unit: u32) {
        let fleets: Vec<u32> = self.sim.galaxy().fleets.values().filter(|f| f.owner == faction).map(|f| f.id).collect();
        for fleet in fleets {
            for (ship, u) in self.sim.ships_of(fleet) {
                if u == unit {
                    self.sim.despawn_ship(ship);
                }
            }
        }
    }

    fn apply_reward(&mut self, r: &StoryReward) -> bool {
        let p = |i: usize| r.params.get(i).map(String::as_str).unwrap_or("");
        let plot_faction = r.faction.as_deref().and_then(|f| self.faction_by_name(f));
        match r.kind.as_str() {
            "CREDITS" => {
                let (faction, amount) = match (self.faction_by_name(p(0)), p(0).parse::<f64>()) {
                    (Some(f), _) => (Some(f), p(1).parse::<f64>().unwrap_or(0.0)),
                    (None, Ok(a)) => (plot_faction, a),
                    _ => (None, 0.0),
                };
                if let Some(f) = faction {
                    self.sim.add_credits(f, amount);
                    self.stats.credits_given += amount;
                    return true;
                }
                false
            }
            "SET_TECH_LEVEL" => {
                let (Some(f), Ok(level)) = (
                    self.faction_by_name(p(0)).or(plot_faction),
                    p(1).parse::<i32>(),
                ) else {
                    return false;
                };
                let max = self.world.factions[f as usize].max_tech;
                let events = self.sim.set_tech_level(f, level.min(max));
                self.pending_events.extend(events);
                true
            }
            "SPAWN_HERO" | "UNIQUE_UNIT" => {
                let Some(f) = plot_faction else { return false };
                let lower = p(0).to_ascii_lowercase();
                // Unique units become buildable at their planet; heroes appear there directly.
                if r.kind == "UNIQUE_UNIT" {
                    self.unlock(f, p(0), true);
                }
                let planet = self
                    .find_planet(p(1))
                    .filter(|&pl| self.sim.galaxy().planets[pl as usize].owner == Some(f))
                    .or(self.world.factions[f as usize].home);
                let (Some(&unit), Some(pl)) = (self.world.unit_index.get(&lower), planet) else {
                    return r.kind == "UNIQUE_UNIT";
                };
                if r.kind == "SPAWN_HERO" {
                    self.add_ships(f, pl, unit, 1);
                    self.events.push(ScriptEvent::UnitSpawned {
                        unit: p(0).to_string(),
                        planet: self.planet_name(pl),
                        faction: self.world.factions[f as usize].name.clone(),
                    });
                }
                true
            }
            "BUILDABLE_UNIT" | "LOCK_UNIT" | "UNLOCK_UNIT" => {
                let Some(f) = plot_faction else { return false };
                let ui_filter = r.event.to_ascii_lowercase().ends_with("unit_event");
                let on = r.kind != "LOCK_UNIT";
                if ui_filter {
                    self.events.push(ScriptEvent::BuildMenuFilter {
                        object: p(0).to_string(),
                        hidden: !on,
                    });
                } else {
                    self.unlock(f, p(0), on);
                }
                true
            }
            // Bookkeeping of the story UI (the "will return" goal entry); nothing to do headless.
            "REMOVE_STORY_GOAL" | "STORY_GOAL_COMPLETED" => true,
            "REMOVE_UNIT" => {
                let (Some(f), Some(&u)) = (
                    plot_faction,
                    self.world.unit_index.get(&p(0).to_ascii_lowercase()),
                ) else {
                    return false;
                };
                self.remove_units(f, u);
                true
            }
            "PLANET_FACTION" => {
                let (Some(pl), Some(f)) = (self.find_planet(p(0)), self.faction_by_name(p(1)))
                else {
                    return false;
                };
                let events = self.sim.set_planet_owner_keeping_buildings(pl, Some(f));
                self.pending_events.extend(events);
                self.events.push(ScriptEvent::PlanetChanged {
                    planet: self.planet_name(pl),
                    faction: self.world.factions[f as usize].name.clone(),
                });
                true
            }
            // Objective / UI bookkeeping with no simulation meaning.
            "STORY_GOAL_COMPLETED"
            | "REMOVE_STORY_GOAL"
            | "OBJECTIVE_COMPLETE"
            | "OBJECTIVE_FAILED"
            | "ADD_OBJECTIVE"
            | "ENABLE_OBJECTIVE_DISPLAY"
            | "SET_SANDBOX_OBJECTIVES"
            | "FORCE_CLICK_GUI"
            | "NEW_POWER_FOR_ALL" => false,
            _ => false,
        }
    }
}

/// Virtual object id of a ship group.
pub fn ship_id(fleet: u32, unit: u32) -> ObjectId {
    (2u64 << 56) | (u64::from(fleet) << 24) | u64::from(unit)
}

fn planet_obj(p: u32) -> ObjectId {
    (1u64 << 56) | u64::from(p)
}

enum Obj {
    Planet(u32),
    Ships { fleet: u32, unit: u32 },
    Structure { planet: u32, structure: u32 },
    Starbase(u32),
}

fn decode(id: ObjectId) -> Option<Obj> {
    let low = id & 0x00FF_FFFF_FFFF_FFFF;
    Some(match id >> 56 {
        1 => Obj::Planet(low as u32),
        2 => Obj::Ships {
            fleet: (low >> 24) as u32,
            unit: (low & 0xFF_FFFF) as u32,
        },
        3 => Obj::Structure {
            planet: (low >> 24) as u32,
            structure: (low & 0xFF_FFFF) as u32,
        },
        4 => Obj::Starbase(low as u32),
        _ => return None,
    })
}

/// The [`ScriptHost`] given to the Lua runtime.
pub struct SimHost {
    /// Shared state.
    pub inner: Rc<RefCell<Inner>>,
}

impl SimHost {
    fn info(&self, id: ObjectId) -> Option<ObjectInfo> {
        let s = self.inner.borrow();
        let w = &s.world;
        let g = s.sim.galaxy();
        match decode(id)? {
            Obj::Planet(p) => {
                let pi = w.planets.get(p as usize)?;
                Some(ObjectInfo {
                    id,
                    type_name: pi.name.clone(),
                    name: pi.name.clone(),
                    owner: Some(g.planets[p as usize].owner.unwrap_or(s.neutral())),
                    planet: Some(id),
                    position: pi.position,
                    is_planet: true,
                    categories: vec!["PLANET".into()],
                    hull: 1.0,
                })
            }
            Obj::Ships { fleet, unit } => {
                let f = g.fleets.get(&fleet)?;
                f.units.get(&unit)?;
                let u = &w.units[unit as usize];
                let planet = f.planet().or(match f.location {
                    FleetLocation::Moving { from, .. } => Some(from),
                    FleetLocation::AtPlanet(p) => Some(p),
                })?;
                Some(ObjectInfo {
                    id,
                    type_name: u.name.clone(),
                    name: u.name.clone(),
                    owner: Some(f.owner),
                    planet: Some(planet_obj(planet)),
                    position: w.planets[planet as usize].position,
                    is_planet: false,
                    categories: vec![format!("{:?}", u.category).to_ascii_uppercase()],
                    hull: 1.0,
                })
            }
            Obj::Structure { planet, structure } => {
                let ps = g.planets.get(planet as usize)?;
                ps.structures.contains(&structure).then_some(())?;
                let st = &w.structures[structure as usize];
                Some(ObjectInfo {
                    id,
                    type_name: st.name.clone(),
                    name: st.name.clone(),
                    owner: ps.owner,
                    planet: Some(planet_obj(planet)),
                    position: w.planets[planet as usize].position,
                    is_planet: false,
                    categories: vec!["STRUCTURE".into()],
                    hull: 1.0,
                })
            }
            Obj::Starbase(planet) => {
                let ps = g.planets.get(planet as usize)?;
                let sb = ps.starbase?;
                Some(ObjectInfo {
                    id,
                    type_name: w.starbases[sb as usize].name.clone(),
                    name: w.starbases[sb as usize].name.clone(),
                    owner: ps.owner,
                    planet: Some(planet_obj(planet)),
                    position: w.planets[planet as usize].position,
                    is_planet: false,
                    categories: vec!["STARBASE".into()],
                    hull: 1.0,
                })
            }
        }
    }
}

impl ScriptHost for SimHost {
    fn players(&self) -> Vec<PlayerId> {
        let s = self.inner.borrow();
        (0..=s.neutral()).collect()
    }

    fn local_player(&self) -> Option<PlayerId> {
        Some(self.inner.borrow().human)
    }

    fn find_player(&self, name: &str) -> Option<PlayerId> {
        let s = self.inner.borrow();
        if name.eq_ignore_ascii_case("neutral") {
            return Some(s.neutral());
        }
        s.faction_by_name(name)
    }

    fn player(&self, id: PlayerId) -> Option<PlayerInfo> {
        let s = self.inner.borrow();
        if id == s.neutral() {
            return Some(PlayerInfo {
                name: "Neutral".into(),
                is_neutral: true,
                difficulty: "Normal".into(),
                ..PlayerInfo::default()
            });
        }
        let f = s.world.factions.get(id as usize)?;
        let st = &s.sim.galaxy().factions[id as usize];
        Some(PlayerInfo {
            name: f.name.clone(),
            is_human: id == s.human,
            credits: st.credits as f32,
            difficulty: "Normal".into(),
            tech_level: st.tech,
            is_neutral: false,
        })
    }

    fn give_money(&mut self, player: PlayerId, amount: f32) {
        let mut s = self.inner.borrow_mut();
        if (player as usize) < s.world.factions.len() {
            s.sim.add_credits(player, f64::from(amount));
        }
        s.stats.credits_given += f64::from(amount);
    }

    fn set_tech_level(&mut self, player: PlayerId, level: i32) {
        let mut s = self.inner.borrow_mut();
        if (player as usize) < s.world.factions.len() {
            let max = s.world.factions[player as usize].max_tech;
            let events = s.sim.set_tech_level(player, level.min(max));
            s.pending_events.extend(events);
        }
    }

    fn make_ally(&mut self, a: PlayerId, b: PlayerId) {
        let mut s = self.inner.borrow_mut();
        s.allies.insert((a.min(b), a.max(b)));
        if (a as usize) < s.world.factions.len() && (b as usize) < s.world.factions.len() {
            s.sim.set_alliance(a, b, true);
        }
    }

    fn is_enemy(&self, a: PlayerId, b: PlayerId) -> bool {
        let s = self.inner.borrow();
        if a == b || s.allies.contains(&(a.min(b), a.max(b))) {
            return false;
        }
        s.world
            .factions
            .get(a as usize)
            .is_some_and(|f| f.enemies.contains(&b))
    }

    fn object_type_exists(&self, name: &str) -> bool {
        let s = self.inner.borrow();
        s.objects.get(name).is_some() || s.world.unit_index.contains_key(&name.to_ascii_lowercase())
    }

    fn object(&self, id: ObjectId) -> Option<ObjectInfo> {
        self.info(id)
    }

    fn planet_by_name(&self, name: &str) -> Option<ObjectId> {
        self.inner.borrow().find_planet(name).map(planet_obj)
    }

    fn all_planets(&self) -> Vec<ObjectId> {
        let s = self.inner.borrow();
        (0..s.world.planets.len() as u32)
            .filter(|&p| s.world.real[p as usize])
            .map(planet_obj)
            .collect()
    }

    fn find_objects(&self, f: &ObjectFilter) -> Vec<ObjectId> {
        let s = self.inner.borrow();
        let w = &s.world;
        let g = s.sim.galaxy();
        let want_type = f.type_name.as_deref().map(str::to_ascii_lowercase);
        let want_cat = f.category.as_deref().map(str::to_ascii_uppercase);
        let planet_filter = f.planet.and_then(|p| match decode(p) {
            Some(Obj::Planet(i)) => Some(i),
            _ => None,
        });
        let mut out = Vec::new();
        let owner_ok = |o: Option<u32>| f.owner.is_none_or(|w| o == Some(w));
        // Planets.
        if planet_filter.is_none()
            && (want_cat.as_deref() == Some("PLANET") || want_type.is_some() || want_cat.is_none())
        {
            for (i, p) in w.planets.iter().enumerate() {
                let owner = g.planets[i].owner.unwrap_or(s.neutral());
                let type_ok = want_type
                    .as_ref()
                    .is_none_or(|t| p.name.eq_ignore_ascii_case(t));
                let cat_ok = want_cat.as_deref().is_none_or(|c| c == "PLANET");
                let all_ok = want_type.is_some() || want_cat.is_some();
                if all_ok && type_ok && cat_ok && owner_ok(Some(owner)) && w.real[i] {
                    out.push(planet_obj(i as u32));
                }
            }
        }
        // Ships.
        if want_cat.as_deref() != Some("PLANET") {
            for fl in g.fleets.values() {
                let at = fl.planet();
                if planet_filter.is_some() && at != planet_filter {
                    continue;
                }
                if !owner_ok(Some(fl.owner)) {
                    continue;
                }
                for &u in fl.units.keys() {
                    let ut = &w.units[u as usize];
                    let type_ok = want_type
                        .as_ref()
                        .is_none_or(|t| ut.name.eq_ignore_ascii_case(t));
                    let cat_ok = want_cat
                        .as_ref()
                        .is_none_or(|c| format!("{:?}", ut.category).to_ascii_uppercase() == *c);
                    if type_ok && cat_ok && matches!(ut.role, UnitRole::Ship) {
                        out.push(ship_id(fl.id, u));
                    }
                }
            }
        }
        // Structures.
        if (want_cat.is_none() || want_cat.as_deref() == Some("STRUCTURE"))
            && let Some(t) = &want_type
        {
            for (pi, ps) in g.planets.iter().enumerate() {
                if planet_filter.is_some_and(|p| p != pi as u32) || !owner_ok(ps.owner) {
                    continue;
                }
                for &st in &ps.structures {
                    if w.structures[st as usize].name.eq_ignore_ascii_case(t) {
                        out.push((3u64 << 56) | ((pi as u64) << 24) | u64::from(st));
                    }
                }
            }
        }
        out
    }

    fn spawn(&mut self, type_name: &str, at: SpawnAt, owner: PlayerId) -> Option<ObjectId> {
        let mut s = self.inner.borrow_mut();
        s.stats.spawn_requests += 1;
        let lower = type_name.to_ascii_lowercase();
        let planet = match at {
            SpawnAt::Planet(id) => match decode(id) {
                Some(Obj::Planet(p)) => p,
                _ => {
                    *s.stats
                        .spawn_ignored
                        .entry("(bad spawn location)".into())
                        .or_default() += 1;
                    return None;
                }
            },
            SpawnAt::Position(_) => {
                *s.stats
                    .spawn_ignored
                    .entry("(position spawn)".into())
                    .or_default() += 1;
                return None;
            }
        };
        if owner as usize >= s.world.factions.len() {
            *s.stats
                .spawn_ignored
                .entry(format!("{type_name} (neutral owner)"))
                .or_default() += 1;
            return None;
        }
        if let Some(&u) = s.world.unit_index.get(&lower)
            && matches!(s.world.units[u as usize].role, UnitRole::Ship)
        {
            *s.stats.spawn_outcomes.entry("ship").or_default() += 1;
            return Some(s.add_ships(owner, planet, u, 1));
        }
        if let Some(&st) = s.world.structure_index.get(&lower)
            && s.sim.galaxy().planets[planet as usize].owner == Some(owner)
        {
            let already = s.sim.galaxy().planets[planet as usize].structures.contains(&st);
            let new = !already && s.sim.add_structure(planet, st);
            if new {
                s.stats.structures_added += 1;
            }
            *s.stats
                .spawn_outcomes
                .entry(if new {
                    "structure"
                } else {
                    "structure_duplicate"
                })
                .or_default() += 1;
            return Some((3u64 << 56) | (u64::from(planet) << 24) | u64::from(st));
        }
        *s.stats
            .spawn_ignored
            .entry(type_name.to_string())
            .or_default() += 1;
        None
    }

    fn despawn(&mut self, id: ObjectId) {
        let mut s = self.inner.borrow_mut();
        match decode(id) {
            Some(Obj::Ships { fleet, unit }) => {
                // One ship group object stands for a single unit.
                if let Some((ship, _)) = s.sim.ships_of(fleet).into_iter().find(|(_, u)| *u == unit) {
                    s.sim.despawn_ship(ship);
                }
            }
            Some(Obj::Structure { planet, structure }) => {
                s.sim.remove_structure(planet, structure);
            }
            _ => {}
        }
    }

    fn change_owner(&mut self, id: ObjectId, owner: PlayerId) {
        let mut s = self.inner.borrow_mut();
        if owner as usize >= s.world.factions.len() {
            return;
        }
        match decode(id) {
            Some(Obj::Planet(p)) => {
                let events = s.sim.set_planet_owner_keeping_buildings(p, Some(owner));
                s.pending_events.extend(events);
            }
            Some(Obj::Ships { fleet, unit }) => {
                if let Some((ship, _)) = s.sim.ships_of(fleet).into_iter().find(|(_, u)| *u == unit) {
                    s.sim.change_ship_owner(ship, owner);
                }
            }
            _ => {}
        }
    }

    fn log(&mut self, script: &str, text: &str) {
        let mut s = self.inner.borrow_mut();
        if s.script_log.len() < 40000 {
            s.script_log.push(format!("[{script}] {text}"));
        }
    }

    fn message(&mut self, text: &str) {
        self.inner
            .borrow_mut()
            .events
            .push(ScriptEvent::Message(text.to_string()));
    }

    fn story_dialog(&mut self, dialog: &str, chapter: i32, event: &str, faction: Option<&str>) {
        let mut s = self.inner.borrow_mut();
        let Some(text) = (s.dialogs)(dialog) else {
            return;
        };
        let parsed = parse_dialog_chapter(&text, chapter);
        let resolve = |id: &str| {
            s.data
                .text(id)
                .map(str::to_string)
                .unwrap_or_else(|| id.to_string())
        };
        let title = resolve(&parsed.title);
        let lines: Vec<String> = parsed.lines.iter().map(|l| resolve(l)).collect();
        s.events.push(ScriptEvent::Dialog {
            dialog: dialog.to_string(),
            chapter,
            title,
            lines,
            sfx: parsed.sfx,
            speech: parsed.speech,
            event: event.to_string(),
            faction: faction.map(str::to_string),
        });
    }

    fn type_number(&self, type_name: &str, what: &str) -> Option<f32> {
        let key = (type_name.to_ascii_lowercase(), what.to_string());
        if let Some(v) = self.inner.borrow().type_cache.get(&key) {
            return *v;
        }
        let v = self.type_number_uncached(type_name, what);
        self.inner.borrow_mut().type_cache.insert(key, v);
        v
    }

    fn create_taskforce(&mut self, spec: &TaskForceSpec) -> Option<u64> {
        self.with_ai(|ai, sim| ai.create_taskforce(sim, spec))
    }

    fn taskforce_call(&mut self, tf: u64, method: &str, args: &[HostValue]) -> Option<HostValue> {
        self.with_ai(|ai, sim| ai.taskforce_call(sim, tf, method, args)).flatten()
    }

    fn block_status(&mut self, block: u64) -> (bool, Option<bool>) {
        self.with_ai(|ai, _| ai.block_status(block)).unwrap_or((true, Some(false)))
    }

    fn ai_call(&mut self, function: &str, args: &[HostValue]) -> Option<HostValue> {
        self.with_ai(|ai, sim| ai.ai_call(sim, function, args)).flatten()
    }

    fn take_plan_events(&mut self) -> Vec<PlanEvent> {
        self.with_ai(|ai, _| ai.take_plan_events()).unwrap_or_default()
    }

    fn evaluate_perception(&self, name: &str, player: PlayerId, target: Option<ObjectId>) -> Option<f32> {
        self.with_ai(|ai, sim| ai.evaluate_perception(sim, name, player, target)).flatten().map(|v| v as f32)
    }

    fn story_reward(&mut self, reward: &StoryReward) {
        let mut s = self.inner.borrow_mut();
        if s.apply_reward(reward) {
            *s.stats
                .rewards_applied
                .entry(reward.kind.clone())
                .or_default() += 1;
        } else {
            *s.stats
                .rewards_ignored
                .entry(reward.kind.clone())
                .or_default() += 1;
            s.events.push(ScriptEvent::UnhandledReward(reward.clone()));
        }
    }
}

impl SimHost {
    /// Runs `f` with the AI engine and a simulation adapter; `None` if the Remake AI is not enabled.
    fn with_ai<R>(
        &self,
        f: impl FnOnce(&mut galaxy_ai::AiEngine, &mut crate::ai::SimAdapter<'_>) -> R,
    ) -> Option<R> {
        let mut guard = self.inner.borrow_mut();
        let Inner { sim, world, human, allies, ai, pending_events, .. } = &mut *guard;
        let ai = ai.as_mut()?;
        let mut adapter = crate::ai::SimAdapter { sim, world: world.clone(), human: *human, allies, pending: pending_events };
        Some(f(ai, &mut adapter))
    }

    fn type_number_uncached(&self, type_name: &str, what: &str) -> Option<f32> {
        let s = self.inner.borrow();
        let tag = match what {
            "combat_rating" => "ai_combat_power",
            "build_cost" => "build_cost_credits",
            "score_cost" => "score_cost_credits",
            _ => return None,
        };
        if let Some(r) = s.objects.resolve(type_name)
            && let Some(v) = r.num::<f32>(tag)
        {
            return Some(v);
        }
        if what == "combat_rating" {
            // The engine derives the rating from unit stats; no XML tag carries it. Use `AI_Combat_Power`
            // where set, otherwise half the build cost (similar magnitude: fighters ~100, frigates ~500).
            // A rating of 0 would make the spawn scripts' budget loops run to their 999-iteration limit.
            let unit = s
                .world
                .unit_index
                .get(&type_name.to_ascii_lowercase())
                .map(|&u| &s.world.units[u as usize]);
            return Some(unit.map_or(50.0, |u| {
                if u.ai_power > 0.0 {
                    u.ai_power
                } else if u.cost > 0.0 {
                    u.cost * 0.5
                } else {
                    50.0
                }
            }));
        }
        None
    }
}

struct DialogChapter {
    title: String,
    lines: Vec<String>,
    sfx: Option<String>,
    speech: Option<String>,
}

/// Extracts one `[CHAPTER n]` of a story dialog file (`TITLE`, `TEXT`, `SFX`, `SPEECH` lines).
fn parse_dialog_chapter(text: &str, chapter: i32) -> DialogChapter {
    let mut out = DialogChapter {
        title: String::new(),
        lines: Vec::new(),
        sfx: None,
        speech: None,
    };
    let mut current = -1;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("[CHAPTER") {
            current = rest.trim_end_matches(']').trim().parse().unwrap_or(-1);
            continue;
        }
        if current != chapter {
            continue;
        }
        let (key, val) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let val = val.trim();
        match key {
            "TITLE" => out.title = val.to_string(),
            "TEXT" if !val.is_empty() => out.lines.push(val.to_string()),
            "SFX" => out.sfx = Some(val.to_string()),
            "SPEECH" => out.speech = Some(val.to_string()),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_chapters() {
        let t = "[CHAPTER 0]\nTITLE A\nTEXT T1\nSFX Boom\n[CHAPTER 1]\nTITLE B\nTEXT T2\nTEXT T3\nSPEECH Hi\n";
        let c = parse_dialog_chapter(t, 1);
        assert_eq!(
            (c.title.as_str(), c.lines.len(), c.speech.as_deref()),
            ("B", 2, Some("Hi"))
        );
        let c0 = parse_dialog_chapter(t, 0);
        assert_eq!((c0.sfx.as_deref(), c0.lines.len()), (Some("Boom"), 1));
    }

    #[test]
    fn id_roundtrip() {
        match decode(ship_id(7, 99)) {
            Some(Obj::Ships { fleet: 7, unit: 99 }) => {}
            _ => panic!(),
        }
    }
}
