//! Names behind the ids of a saved state, and moving a state from the catalog it was saved with to the catalog of this build.
//!
//! Planets, factions, unit types, structures and star bases are plain indices into the [`World`], and the world is rebuilt from the game data
//! on every start. The indices therefore move whenever the catalog grows or is ordered differently (a build that reads more of the data, a mod
//! update). A save keeps the [`NameTables`] it was written with; a load builds an [`IdMap`] from them and rewrites every id in the snapshot
//! ([`crate::Sim::remap_snapshot`]). Names the save does not know are simply new; a name the save uses and the game data lacks is an error that
//! lists it.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::agents::{ActiveSiphon, SpyReport};
use crate::sim::SimSnapshot;
use crate::state::*;
use crate::world::*;

/// The names behind the ids of a state, each list in id order.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NameTables {
    /// Planets ([`PlanetId`]).
    pub planets: Vec<String>,
    /// Factions ([`FactionId`]).
    pub factions: Vec<String>,
    /// Unit types ([`UnitTypeId`]).
    pub units: Vec<String>,
    /// Structures ([`StructureTypeId`]).
    pub structures: Vec<String>,
    /// Star base levels ([`StarbaseTypeId`]).
    pub starbases: Vec<String>,
}

impl World {
    /// The names behind every id of this world.
    pub fn name_tables(&self) -> NameTables {
        NameTables {
            planets: self.planets.iter().map(|p| p.name.clone()).collect(),
            factions: self.factions.iter().map(|f| f.name.clone()).collect(),
            units: self.units.iter().map(|u| u.name.clone()).collect(),
            structures: self.structures.iter().map(|s| s.name.clone()).collect(),
            starbases: self.starbases.iter().map(|s| s.name.clone()).collect(),
        }
    }
}

/// One id space of an [`IdMap`].
#[derive(Debug, Clone)]
struct Table {
    /// What the ids name, for messages.
    kind: &'static str,
    /// Saved id to id now; `None` when the game data no longer has the name.
    to: Vec<Option<u32>>,
    /// Saved names, for messages.
    names: Vec<String>,
    /// The saved table is the table of this world.
    same: bool,
}

impl Table {
    /// Matches by name without regard to case, as the world's own name lookups do. Of several entries with one name the n-th stays the n-th.
    fn new(kind: &'static str, saved: &[String], now: &[String]) -> Table {
        let mut slots: HashMap<String, VecDeque<u32>> = HashMap::new();
        for (i, name) in now.iter().enumerate() {
            slots.entry(name.to_ascii_lowercase()).or_default().push_back(i as u32);
        }
        let to = saved.iter().map(|name| slots.get_mut(&name.to_ascii_lowercase()).and_then(VecDeque::pop_front)).collect();
        Table { kind, to, names: saved.to_vec(), same: saved == now }
    }

    fn get(&self, id: u32) -> Option<u32> {
        self.to.get(id as usize).copied().flatten()
    }
}

/// Translation of the ids of a saved state into the ids of a world.
#[derive(Debug, Clone)]
pub struct IdMap {
    planets: Table,
    factions: Table,
    units: Table,
    structures: Table,
    starbases: Table,
}

impl IdMap {
    /// The map from the tables a state was saved with to the tables of the world it is loaded into.
    pub fn new(saved: &NameTables, now: &NameTables) -> IdMap {
        IdMap {
            planets: Table::new("planet", &saved.planets, &now.planets),
            factions: Table::new("faction", &saved.factions, &now.factions),
            units: Table::new("unit", &saved.units, &now.units),
            structures: Table::new("structure", &saved.structures, &now.structures),
            starbases: Table::new("star base", &saved.starbases, &now.starbases),
        }
    }

    /// Nothing moved: the state can be used as it is.
    pub fn is_identity(&self) -> bool {
        self.same_places() && self.units.same && self.structures.same && self.starbases.same
    }

    /// Planets and factions did not move (only the catalogs did, if anything).
    pub fn same_places(&self) -> bool {
        self.planets.same && self.factions.same
    }

    /// Id of a saved planet in the world; `None` when the world does not have it.
    pub fn planet(&self, id: PlanetId) -> Option<PlanetId> {
        self.planets.get(id)
    }

    /// Id of a saved faction.
    pub fn faction(&self, id: FactionId) -> Option<FactionId> {
        self.factions.get(id)
    }

    /// Id of a saved unit type.
    pub fn unit(&self, id: UnitTypeId) -> Option<UnitTypeId> {
        self.units.get(id)
    }

    /// Id of a saved structure.
    pub fn structure(&self, id: StructureTypeId) -> Option<StructureTypeId> {
        self.structures.get(id)
    }

    /// Id of a saved star base level.
    pub fn starbase(&self, id: StarbaseTypeId) -> Option<StarbaseTypeId> {
        self.starbases.get(id)
    }
}

/// Rewrites one snapshot and collects what does not fit instead of stopping at the first problem, so the message names everything.
struct Remap<'a> {
    map: &'a IdMap,
    /// The lane index for a fleet travelling between two planets (ids of the world): the saved index and the two ends go in.
    lane: &'a dyn Fn(u32, PlanetId, PlanetId) -> Result<u32, String>,
    /// `kind name` of everything the state uses and the world lacks.
    missing: BTreeSet<String>,
    /// Other reasons the state does not fit.
    problems: Vec<String>,
}

impl Remap<'_> {
    fn look(&mut self, table: &Table, id: u32) -> u32 {
        match table.to.get(id as usize) {
            Some(Some(now)) => *now,
            Some(None) => {
                self.missing.insert(format!("{} {}", table.kind, table.names[id as usize]));
                0
            }
            None => {
                self.missing.insert(format!("{} #{id} (not in the name table of the save)", table.kind));
                0
            }
        }
    }

    fn planet(&mut self, id: PlanetId) -> PlanetId {
        let map = self.map;
        self.look(&map.planets, id)
    }

    fn faction(&mut self, id: FactionId) -> FactionId {
        let map = self.map;
        self.look(&map.factions, id)
    }

    fn unit(&mut self, id: UnitTypeId) -> UnitTypeId {
        let map = self.map;
        self.look(&map.units, id)
    }

    fn structure(&mut self, id: StructureTypeId) -> StructureTypeId {
        let map = self.map;
        self.look(&map.structures, id)
    }

    fn starbase(&mut self, id: StarbaseTypeId) -> StarbaseTypeId {
        let map = self.map;
        self.look(&map.starbases, id)
    }

    // Every struct is taken apart field by field without `..`: a field added later does not compile here until someone decides whether it
    // holds an id.

    fn item(&mut self, item: ProductionItem) -> ProductionItem {
        let ProductionItem { kind, remaining, total, cost } = item;
        let kind = match kind {
            ProductionKind::Unit(u) => ProductionKind::Unit(self.unit(u)),
            ProductionKind::Structure(s) => ProductionKind::Structure(self.structure(s)),
            ProductionKind::Starbase(b) => ProductionKind::Starbase(self.starbase(b)),
            // An index into the faction's tech chain, which is read from the data in file order.
            ProductionKind::Tech(level) => ProductionKind::Tech(level),
        };
        ProductionItem { kind, remaining, total, cost }
    }

    fn planet_state(&mut self, p: PlanetState) -> PlanetState {
        let PlanetState { owner, starbase, structures, queues, capture, base_health, destroyed, gamble, gamble_at, siphon, hero_gamble, hero_gamble_at, last_space_battle, suspended_structures, battle_settled: _ } = p;
        PlanetState {
            owner: owner.map(|f| self.faction(f)),
            starbase: starbase.map(|b| self.starbase(b)),
            structures: structures.into_iter().map(|s| self.structure(s)).collect(),
            queues: queues.map(|q| q.into_iter().map(|i| self.item(i)).collect()),
            capture: capture.map(|(f, t)| (self.faction(f), t)),
            base_health,
            destroyed,
            gamble,
            gamble_at,
            siphon: siphon.map(|s| {
                let ActiveSiphon { by, agent, share, cycles_left, cycle_end } = s;
                ActiveSiphon { by: self.faction(by), agent: self.unit(agent), share, cycles_left, cycle_end }
            }),
            hero_gamble,
            hero_gamble_at,
            last_space_battle,
            suspended_structures: suspended_structures.into_iter().map(|s| self.structure(s)).collect(),
            // A fingerprint over ids cannot be translated: the next battle here is judged anew.
            battle_settled: None,
        }
    }

    fn faction_state(&mut self, f: FactionState) -> FactionState {
        // `unlocked`, `story_locked` and `powers` hold names already.
        let FactionState {
            credits,
            tech,
            unlocked,
            story_locked,
            powers,
            built_lifetime,
            built_structures_lifetime,
            eliminated,
            income_per_week,
            population_used,
            population_cap,
            planets_owned,
            difficulty,
            superweapon_ready_at,
            stats,
        } = f;
        let mut built = BTreeMap::new();
        for (u, n) in built_lifetime {
            *built.entry(self.unit(u)).or_default() += n;
        }
        let mut built_structs = BTreeMap::new();
        for (s, n) in built_structures_lifetime {
            *built_structs.entry(self.structure(s)).or_default() += n;
        }
        FactionState {
            credits,
            tech,
            unlocked,
            story_locked,
            powers,
            built_lifetime: built,
            built_structures_lifetime: built_structs,
            eliminated,
            income_per_week,
            population_used,
            population_cap,
            planets_owned,
            difficulty,
            superweapon_ready_at,
            stats,
        }
    }

    fn location(&mut self, location: FleetLocation) -> FleetLocation {
        match location {
            FleetLocation::AtPlanet(p) => FleetLocation::AtPlanet(self.planet(p)),
            FleetLocation::Moving { from, to, route, progress, rest } => {
                let (from, to) = (self.planet(from), self.planet(to));
                let route = (self.lane)(route, from, to).unwrap_or_else(|why| {
                    self.problems.push(why);
                    0
                });
                FleetLocation::Moving { from, to, route, progress, rest: rest.into_iter().map(|p| self.planet(p)).collect() }
            }
        }
    }

    fn fleet(&mut self, fleet: Fleet) -> Fleet {
        let Fleet { id, owner, units, location, previous_planet, slot } = fleet;
        let mut mapped = BTreeMap::new();
        for (u, n) in units {
            *mapped.entry(self.unit(u)).or_default() += n;
        }
        Fleet { id, owner: self.faction(owner), units: mapped, location: self.location(location), previous_planet: self.planet(previous_planet), slot }
    }

    fn galaxy(&mut self, g: Galaxy, fresh: &Galaxy) -> Galaxy {
        let Galaxy {
            time,
            planets,
            factions,
            fleets,
            battles,
            winner,
            allied,
            ships,
            damage,
            rewarded,
            escaping,
            hyperspace,
            dest_slots,
            merged_into,
            cheated,
            ai_paused,
            spy_reports,
            next_ship_id,
            next_fleet_id,
            next_battle_id,
            rng,
            week,
            ai_timer,
            tick_accum,
        } = g;
        for (kind, held, named) in [("planets", planets.len(), self.map.planets.to.len()), ("factions", factions.len(), self.map.factions.to.len())] {
            if held != named {
                self.problems.push(format!("the save holds {held} {kind} and names {named}"));
            }
        }
        // Planets and factions the save does not know keep the state of a new match.
        let mut placed_planets = fresh.planets.clone();
        for (i, p) in planets.into_iter().enumerate() {
            let (to, p) = (self.planet(i as PlanetId), self.planet_state(p));
            if let Some(slot) = placed_planets.get_mut(to as usize) {
                *slot = p;
            }
        }
        let mut placed_factions = fresh.factions.clone();
        for (i, f) in factions.into_iter().enumerate() {
            let (to, f) = (self.faction(i as FactionId), self.faction_state(f));
            if let Some(slot) = placed_factions.get_mut(to as usize) {
                *slot = f;
            }
        }
        Galaxy {
            time,
            planets: placed_planets,
            factions: placed_factions,
            fleets: fleets.into_iter().map(|(id, f)| (id, self.fleet(f))).collect(),
            battles: battles
                .into_iter()
                .map(|b| {
                    let PendingBattle { id, planet, fleets, factions, defender, line_up } = b;
                    PendingBattle { id, planet: self.planet(planet), fleets, factions: factions.into_iter().map(|f| self.faction(f)).collect(), defender: defender.map(|f| self.faction(f)), line_up }
                })
                .collect(),
            winner: winner.map(|f| self.faction(f)),
            // Pairs are kept sorted.
            allied: allied
                .into_iter()
                .map(|(a, b)| {
                    let (a, b) = (self.faction(a), self.faction(b));
                    (a.min(b), a.max(b))
                })
                .collect(),
            ships: ships.into_iter().map(|(fleet, list)| (fleet, list.into_iter().map(|(ship, u)| (ship, self.unit(u))).collect())).collect(),
            damage,
            rewarded: rewarded.into_iter().map(|(p, f)| (self.planet(p), self.faction(f))).collect(),
            escaping,
            hyperspace: hyperspace.into_iter().map(|(fleet, (ship, u))| (fleet, (ship, self.unit(u)))).collect(),
            dest_slots: dest_slots.into_iter().map(|(fleet, (p, slot))| (fleet, (self.planet(p), slot))).collect(),
            merged_into,
            cheated,
            ai_paused,
            spy_reports: spy_reports
                .into_iter()
                .map(|r| {
                    let SpyReport { faction, planet, until, see } = r;
                    SpyReport { faction: self.faction(faction), planet: self.planet(planet), until, see }
                })
                .collect(),
            next_ship_id,
            next_fleet_id,
            next_battle_id,
            rng,
            week,
            ai_timer,
            tick_accum,
        }
    }

    fn snapshot(&mut self, snap: SimSnapshot, fresh: &Galaxy) -> SimSnapshot {
        let SimSnapshot { galaxy, ai, passive_types } = snap;
        SimSnapshot {
            galaxy: self.galaxy(galaxy, fresh),
            ai: ai.into_iter().map(|f| self.faction(f)).collect(),
            passive_types: passive_types.into_iter().map(|(p, types)| (self.planet(p), types.into_iter().map(|u| self.unit(u)).collect())).collect(),
        }
    }

    /// The reason the state does not fit, if any: every missing name (the first dozen spelled out) and every other problem.
    fn verdict(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.missing.is_empty() {
            const SHOWN: usize = 12;
            let mut list: Vec<&str> = self.missing.iter().take(SHOWN).map(String::as_str).collect();
            let more = self.missing.len().saturating_sub(SHOWN);
            let tail = format!("and {more} more");
            if more > 0 {
                list.push(&tail);
            }
            parts.push(format!("the save uses {} this game data does not have: {}", if self.missing.len() == 1 { "an object".to_string() } else { format!("{} objects", self.missing.len()) }, list.join(", ")));
        }
        parts.extend(self.problems.iter().cloned());
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

impl crate::Sim {
    /// Rewrites a snapshot taken with other name tables into the ids of this sim's world, ready for [`crate::Sim::restore`]. Call it on the
    /// freshly built sim: planets and factions the snapshot does not know keep their state of a new match. Fails with a message that names
    /// what the snapshot uses and the world lacks.
    pub fn remap_snapshot(&self, snap: SimSnapshot, map: &IdMap) -> Result<SimSnapshot, String> {
        let world = &self.world;
        // Lanes have no name in a save: the saved index is kept while it still joins the two planets, else the lane between them is looked up.
        let lane = |saved: u32, from: PlanetId, to: PlanetId| {
            let joins = |r: &RouteInfo| (r.a == from && r.b == to) || (r.a == to && r.b == from);
            if world.routes.get(saved as usize).is_some_and(joins) {
                return Ok(saved);
            }
            let name = |p: PlanetId| world.planets.get(p as usize).map_or("?", |i| i.name.as_str());
            world.adjacency.get(from as usize).and_then(|_| world.route_between(from, to)).ok_or_else(|| format!("a fleet travels from {} to {}, which no lane joins any more", name(from), name(to)))
        };
        let mut remap = Remap { map, lane: &lane, missing: BTreeSet::new(), problems: Vec::new() };
        let snap = remap.snapshot(snap, &self.galaxy);
        match remap.verdict() {
            Some(why) => Err(why),
            None => Ok(snap),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn tables(units: &[&str]) -> NameTables {
        NameTables { planets: names(&["Alpha", "Beta"]), factions: names(&["Rebel", "Empire"]), units: names(units), structures: names(&["Mine"]), starbases: names(&["Base_1"]) }
    }

    #[test]
    fn equal_tables_are_the_identity() {
        let t = tables(&["A", "B"]);
        let map = IdMap::new(&t, &t);
        assert!(map.is_identity());
        assert_eq!((map.unit(0), map.unit(1), map.unit(2)), (Some(0), Some(1), None));
    }

    /// The case that broke every save: the catalog of the loading build has more unit types than the one that saved.
    #[test]
    fn a_unit_added_in_front_moves_the_saved_ids_up() {
        let map = IdMap::new(&tables(&["A", "B"]), &tables(&["New", "A", "B", "Newer"]));
        assert!(!map.is_identity());
        assert!(map.same_places(), "planets and factions did not move");
        assert_eq!((map.unit(0), map.unit(1)), (Some(1), Some(2)));
    }

    #[test]
    fn a_reordered_table_is_followed_by_name() {
        let map = IdMap::new(&tables(&["A", "B", "C"]), &tables(&["c", "A", "B"]));
        assert_eq!((map.unit(0), map.unit(1), map.unit(2)), (Some(1), Some(2), Some(0)), "names match without regard to case");
    }

    #[test]
    fn a_name_the_world_lost_maps_to_nothing() {
        let map = IdMap::new(&tables(&["A", "Gone", "B"]), &tables(&["A", "B"]));
        assert_eq!((map.unit(0), map.unit(1), map.unit(2)), (Some(0), None, Some(1)));
    }

    /// The data has objects that share a name (the later one overrides the earlier in name lookups, both keep an id).
    #[test]
    fn equal_names_keep_their_order() {
        let map = IdMap::new(&tables(&["A", "Twin", "Twin"]), &tables(&["Twin", "New", "Twin", "A"]));
        assert_eq!((map.unit(0), map.unit(1), map.unit(2)), (Some(3), Some(0), Some(2)));
    }

    /// Every saved lane index is still good.
    fn lanes_kept(saved: u32, _: PlanetId, _: PlanetId) -> Result<u32, String> {
        Ok(saved)
    }

    fn remap(map: &IdMap) -> Remap<'_> {
        Remap { map, lane: &lanes_kept, missing: BTreeSet::new(), problems: Vec::new() }
    }

    /// A galaxy with every kind of id in it, as a build with the unit table `["A", "B"]` would save it.
    fn full_galaxy() -> Galaxy {
        let mut g = Galaxy::sized(2, vec![FactionState::default(); 2], 1);
        g.planets[1] = PlanetState {
            owner: Some(1),
            starbase: Some(0),
            structures: vec![0],
            queues: [
                [ProductionItem { kind: ProductionKind::Unit(1), remaining: 3.0, total: 9.0, cost: 50.0 }].into(),
                [ProductionItem { kind: ProductionKind::Structure(0), remaining: 1.0, total: 2.0, cost: 5.0 }].into(),
                [ProductionItem { kind: ProductionKind::Starbase(0), remaining: 1.0, total: 2.0, cost: 5.0 }, ProductionItem { kind: ProductionKind::Tech(2), remaining: 1.0, total: 2.0, cost: 5.0 }].into(),
            ],
            capture: Some((0, 2.5)),
            siphon: Some(ActiveSiphon { by: 0, agent: 1, share: 0.1, cycles_left: 2, cycle_end: 90.0 }),
            ..PlanetState::default()
        };
        g.factions[1].built_lifetime = BTreeMap::from([(0, 4), (1, 1)]);
        g.fleets.insert(5, Fleet { id: 5, owner: 1, units: BTreeMap::from([(0, 2), (1, 1)]), location: FleetLocation::AtPlanet(1), previous_planet: 0, slot: 1 });
        g.sync_ships();
        g.hyperspace.insert(5, g.ships[&5][2]);
        g.battles.push(PendingBattle { id: 1, planet: 1, fleets: vec![5], factions: vec![0, 1], defender: Some(1), line_up: 7 });
        g.winner = Some(1);
        g.allied.insert((0, 1));
        g.rewarded.insert((1, 1));
        g.dest_slots.insert(5, (1, 2));
        g.spy_reports.push(SpyReport { faction: 0, planet: 1, until: 70.0, see: Default::default() });
        g
    }

    #[test]
    fn every_unit_id_of_a_state_follows_its_name() {
        // Saved with ["A", "B"]; now "B" is id 0 and "A" is id 3.
        let map = IdMap::new(&tables(&["A", "B"]), &tables(&["B", "New", "Newer", "A"]));
        let before = SimSnapshot { galaxy: full_galaxy(), ai: vec![1], passive_types: BTreeMap::from([(1, vec![0, 1])]) };
        let mut r = remap(&map);
        let after = r.snapshot(before.clone(), &Galaxy::sized(2, vec![FactionState::default(); 2], 1));
        assert_eq!(r.verdict(), None);
        // The same renumbering by hand, field by field; everything else must be untouched.
        let mut expect = before;
        let g = &mut expect.galaxy;
        g.fleets.get_mut(&5).unwrap().units = BTreeMap::from([(3, 2), (0, 1)]);
        for (ship, to) in g.ships.get_mut(&5).unwrap().iter_mut().zip([3, 3, 0]) {
            ship.1 = to;
        }
        g.hyperspace.get_mut(&5).unwrap().1 = 0;
        g.planets[1].queues[0][0].kind = ProductionKind::Unit(0);
        g.planets[1].siphon.as_mut().unwrap().agent = 0;
        g.factions[1].built_lifetime = BTreeMap::from([(3, 4), (0, 1)]);
        expect.passive_types = BTreeMap::from([(1, vec![3, 0])]);
        assert_eq!(after, expect);
        assert_eq!(after.galaxy.planets[1].queues[2][1].kind, ProductionKind::Tech(2), "a tech level is not a catalog id");
    }

    #[test]
    fn planets_and_factions_follow_their_names_and_new_ones_keep_the_fresh_state() {
        let saved = tables(&["A", "B"]);
        let now = NameTables { planets: names(&["Beta", "Gamma", "Alpha"]), factions: names(&["Empire", "Rebel", "Pirates"]), ..saved.clone() };
        let map = IdMap::new(&saved, &now);
        assert!(!map.same_places());
        let mut fresh = Galaxy::sized(3, vec![FactionState::default(); 3], 1);
        fresh.planets[1].owner = Some(2);
        fresh.factions[2].credits = 777.0;
        let mut r = remap(&map);
        let after = r.snapshot(SimSnapshot { galaxy: full_galaxy(), ai: vec![1], passive_types: BTreeMap::from([(1, vec![0])]) }, &fresh);
        // The fleet sits in orbit, so no lane is needed.
        assert_eq!(r.verdict(), None);
        let g = &after.galaxy;
        // Beta was planet 1 and is planet 0; the Empire was faction 1 and is faction 0.
        assert_eq!(g.planets[0].owner, Some(0));
        assert_eq!(g.planets[0].capture, Some((1, 2.5)));
        assert_eq!(g.planets[0].siphon.unwrap().by, 1);
        assert_eq!(g.planets[2], PlanetState::default(), "Alpha was empty and is planet 2 now");
        assert_eq!((g.planets[1].owner, g.factions[2].credits), (Some(2), 777.0), "Gamma and the Pirates are new");
        assert_eq!(g.factions[0].built_lifetime.len(), 2, "the Empire's state moved with it");
        let f = &g.fleets[&5];
        assert_eq!((f.owner, &f.location, f.previous_planet), (0, &FleetLocation::AtPlanet(0), 2));
        assert_eq!(g.battles[0], PendingBattle { id: 1, planet: 0, fleets: vec![5], factions: vec![1, 0], defender: Some(0), line_up: 7 });
        assert_eq!(g.winner, Some(0));
        assert!(g.allied.contains(&(0, 1)), "pairs stay sorted");
        assert!(g.rewarded.contains(&(0, 0)));
        assert_eq!(g.dest_slots[&5], (0, 2));
        assert_eq!((g.spy_reports[0].faction, g.spy_reports[0].planet), (1, 0));
        assert_eq!(after.ai, vec![0]);
        assert_eq!(after.passive_types.keys().copied().collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn what_the_world_lacks_is_named() {
        let saved = NameTables { structures: names(&["Mine"]), ..tables(&["A", "Old_Ship"]) };
        let now = NameTables { structures: names(&["Other"]), ..tables(&["A"]) };
        let map = IdMap::new(&saved, &now);
        let mut r = remap(&map);
        r.snapshot(SimSnapshot { galaxy: full_galaxy(), ai: Vec::new(), passive_types: BTreeMap::new() }, &Galaxy::sized(2, vec![FactionState::default(); 2], 1));
        let why = r.verdict().expect("refused");
        assert_eq!(why, "the save uses 2 objects this game data does not have: structure Mine, unit Old_Ship");
    }

    /// A unit type the catalog lost does no harm while the state does not use it.
    #[test]
    fn a_lost_name_nobody_uses_is_no_error() {
        let map = IdMap::new(&tables(&["A", "B", "Unused"]), &tables(&["A", "B"]));
        let mut r = remap(&map);
        r.snapshot(SimSnapshot { galaxy: full_galaxy(), ai: Vec::new(), passive_types: BTreeMap::new() }, &Galaxy::sized(2, vec![FactionState::default(); 2], 1));
        assert_eq!(r.verdict(), None);
    }

    #[test]
    fn an_id_outside_the_saved_table_is_refused() {
        let map = IdMap::new(&tables(&["A"]), &tables(&["A"]));
        let mut r = remap(&map);
        r.snapshot(SimSnapshot { galaxy: full_galaxy(), ai: Vec::new(), passive_types: BTreeMap::new() }, &Galaxy::sized(2, vec![FactionState::default(); 2], 1));
        assert!(r.verdict().unwrap().contains("unit #1 (not in the name table of the save)"));
    }

    #[test]
    fn a_travelling_fleet_gets_the_lane_of_this_world_or_is_refused() {
        let saved = tables(&["A", "B"]);
        let map = IdMap::new(&saved, &NameTables { planets: names(&["Beta", "Alpha"]), ..saved.clone() });
        let mut g = full_galaxy();
        g.fleets.get_mut(&5).unwrap().location = FleetLocation::Moving { from: 0, to: 1, route: 4, progress: 1.0, rest: vec![0] };
        // The lane lookup sees the planets of the world: Alpha is 1 and Beta 0 there.
        let lane = |saved: u32, from: PlanetId, to: PlanetId| if (from, to) == (1, 0) { Ok(saved + 10) } else { Err(format!("no lane from {from} to {to}")) };
        let mut r = Remap { map: &map, lane: &lane, missing: BTreeSet::new(), problems: Vec::new() };
        let after = r.snapshot(SimSnapshot { galaxy: g.clone(), ai: Vec::new(), passive_types: BTreeMap::new() }, &Galaxy::sized(2, vec![FactionState::default(); 2], 1));
        assert_eq!(r.verdict(), None);
        assert_eq!(after.galaxy.fleets[&5].location, FleetLocation::Moving { from: 1, to: 0, route: 14, progress: 1.0, rest: vec![1] });

        let gone = |_: u32, _: PlanetId, _: PlanetId| Err("no lane joins them any more".to_string());
        let mut r = Remap { map: &map, lane: &gone, missing: BTreeSet::new(), problems: Vec::new() };
        r.snapshot(SimSnapshot { galaxy: g, ai: Vec::new(), passive_types: BTreeMap::new() }, &Galaxy::sized(2, vec![FactionState::default(); 2], 1));
        assert_eq!(r.verdict().as_deref(), Some("no lane joins them any more"));
    }
}
