//! A won battle is over once: the Empire's start fleet attacks the pirate planet Fresia in "From the Ground Up"
//! (real data, the scripted start with `Spawn_Starting_Forces_Web_Way.lua`; skipped without the game).

use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use battle_sim::{Arena, Catalog};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{BattleRepeats, Command, Event, FleetId, FleetLocation, GroupKind, PlanetId, SimConfig, TacticalRequest, UnitTypeId, run_headless};

struct Ctx {
    fs: Arc<GameFs>,
    data: Arc<GameData>,
}

fn load() -> Option<Ctx> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let data = Arc::new(load_galaxy(&*fs).0);
    Some(Ctx { fs, data })
}

/// The Empire's "From the Ground Up" game as the app starts it (the human plays the Empire, battles wait for the caller). The guard against
/// re-created battles (`SimConfig::battle_repeat_seconds`) is off: a won battle must be over because nothing hostile is left, not because the
/// guard holds the next one back.
fn ground_up_empire(ctx: &Ctx) -> ScriptedSim {
    let campaign = ctx.data.campaign("GC_Full_Web_Way_Imperial").expect("campaign").clone();
    let sim_config = SimConfig { auto_resolve_battles: false, victory_enabled: false, ground_up: true, seed: 42, battle_repeat_seconds: 0.0, ..SimConfig::default() };
    let fs: Arc<dyn FileSource> = ctx.fs.clone();
    ScriptedSim::new(fs, ctx.data.clone(), &campaign, sim_config, ScriptedConfig { human: Some("Empire".into()), ..ScriptedConfig::default() }).expect("scripted sim")
}

/// The Empire's start fleet flies from Coruscant to Fresia, where the pirates hold `pirate_units` in one fleet (the play tester's composition)
/// behind their star base. Returns the game at the pending battle: battle id, planet, Empire fleet, pirate fleet.
fn fresia_attack(ctx: &Ctx, pirate_units: &[(&str, u32)]) -> (ScriptedSim, u32, PlanetId, FleetId, FleetId) {
    let mut s = ground_up_empire(ctx);
    // The start scripts run in the first seconds (story start, starting forces).
    for _ in 0..3 {
        s.step(1.0);
    }
    let w = s.with_sim(|sim| sim.world().clone());
    let (empire, pirates) = (w.faction_id("Empire").unwrap(), w.faction_id("Pirates").unwrap());
    let (fresia, coruscant) = (w.planet_id("Fresia_WW").expect("Fresia"), w.planet_id("Coruscant_WW").expect("Coruscant"));
    let (attacker, defender) = s.with_sim_mut(|sim| {
        assert_eq!(sim.galaxy().planets[fresia as usize].owner, Some(pirates), "Fresia starts as a pirate planet");
        assert!(sim.galaxy().planets[fresia as usize].starbase.is_some(), "with a star base");
        assert_eq!(sim.galaxy().factions[empire as usize].tech, 1, "the Empire starts at tech 1");
        let units: BTreeMap<UnitTypeId, u32> = pirate_units.iter().map(|&(n, c)| (w.unit_id(n).unwrap_or_else(|| panic!("unit {n}")), c)).collect();
        let g = sim.galaxy_mut();
        let defender = g.fleets.values().find(|f| f.owner == pirates && f.location == FleetLocation::AtPlanet(fresia)).map(|f| f.id).expect("a pirate fleet at Fresia");
        g.fleets.get_mut(&defender).unwrap().units = units;
        g.sync_ships();
        let attacker = g.fleets.values().filter(|f| f.owner == empire && f.location == FleetLocation::AtPlanet(coruscant) && w.is_combat_fleet(f)).max_by_key(|f| f.unit_count()).map(|f| f.id).expect("the Empire's start fleet");
        sim.apply(Command::MoveFleet { faction: empire, fleet: attacker, to: fresia }).expect("move order");
        (attacker, defender)
    });
    let mut battle = None;
    for _ in 0..600 {
        let report = s.step(1.0);
        battle = report.sim_events.iter().find_map(|e| if let Event::BattlePending(b) = e { (b.planet == fresia).then_some(b.id) } else { None });
        if battle.is_some() {
            break;
        }
    }
    (s, battle.expect("a battle at Fresia"), fresia, attacker, defender)
}

fn catalog_for(ctx: &Ctx, req: &TacticalRequest) -> Arc<Catalog> {
    let names: Vec<&str> = req.sides.iter().flat_map(|s| s.groups.iter().chain(&s.reserve).map(|g| g.name.as_str())).collect();
    Arc::new(Catalog::build(&ctx.data, Some(&*ctx.fs), &names))
}

/// Units of fleets per type that `side` of the request holds on the field and in the reserve.
fn fleet_units(req: &TacticalRequest, side: usize) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for g in req.sides[side].groups.iter().chain(&req.sides[side].reserve).filter(|g| g.kind == GroupKind::Unit) {
        *out.entry(g.name.clone()).or_default() += g.count;
    }
    out
}

/// What the play tester met at Fresia: six units in the pirate fleet.
const PIRATES_AT_FRESIA: [(&str, u32); 5] = [("Interceptor_IV", 2), ("Z-95_Squadron", 1), ("IPV1_System_Patrol_Craft", 1), ("N1_Squadron", 1), ("Skipray_Squadron", 1)];

#[test]
fn every_unit_of_the_pirate_fleet_at_fresia_belongs_to_the_battle() {
    let Some(ctx) = load() else { return };
    let (s, battle, _, _, _) = fresia_attack(&ctx, &PIRATES_AT_FRESIA);
    let w = s.with_sim(|sim| sim.world().clone());
    let req = s.with_sim_mut(|sim| sim.tactical_setup(battle)).expect("request");
    let ps = req.sides.iter().position(|x| x.faction == w.faction_id("Pirates").unwrap()).unwrap();
    for (si, side) in req.sides.iter().enumerate() {
        let list = |l: &[galaxy_sim::TacticalGroup]| l.iter().map(|g| format!("{}x{}", g.count, g.name)).collect::<Vec<_>>().join(" ");
        println!("side {si} {}: field [{}] reserve [{}] cap {}", side.faction_name, list(&side.groups), list(&side.reserve), side.unit_cap);
    }
    let want: BTreeMap<String, u32> = PIRATES_AT_FRESIA.iter().map(|&(n, c)| (n.to_owned(), c)).collect();
    assert_eq!(fleet_units(&req, ps), want, "field plus reserve of the pirates is their whole fleet");
    assert!(req.sides[ps].groups.iter().any(|g| g.kind == GroupKind::Starbase), "and their star base");
}

#[test]
fn the_won_battle_at_fresia_is_over_once_and_the_planet_falls() {
    let Some(ctx) = load() else { return };
    let (mut s, battle, fresia, attacker, defender) = fresia_attack(&ctx, &PIRATES_AT_FRESIA);
    let w = s.with_sim(|sim| sim.world().clone());
    let (empire, pirates) = (w.faction_id("Empire").unwrap(), w.faction_id("Pirates").unwrap());
    let req = s.with_sim_mut(|sim| sim.tactical_setup(battle)).expect("request");
    let (es, ps) = (req.sides.iter().position(|x| x.faction == empire).unwrap(), req.sides.iter().position(|x| x.faction == pirates).unwrap());
    let catalog = catalog_for(&ctx, &req);
    let arena = req.map.as_ref().and_then(|m| Arena::load(&ctx.data, &*ctx.fs, m)).map(Arc::new);
    let result = run_headless(&req, &catalog, arena).expect("result");
    println!("Fresia: winner {:?} after {:.0} s, survived {:?}, reserve called {:?}", result.winner, result.duration, result.survived, result.reserve_called);
    assert_eq!(result.winner, Some(es), "the Empire's start fleet beats the pirates at Fresia");
    assert!(result.survived[ps].iter().all(|&n| n == 0), "nothing of the pirates is left: {:?}", result.survived[ps]);
    let events = s.with_sim_mut(|sim| sim.apply_tactical_result(battle, &result)).expect("applied");
    assert!(events.iter().any(|e| matches!(e, Event::BattleWon { faction, .. } if *faction == empire)));
    s.with_sim(|sim| {
        let g = sim.galaxy();
        assert!(!g.fleets.contains_key(&defender), "the pirate fleet is destroyed, all of it");
        assert!(g.fleets_at(fresia).all(|f| f.owner == empire), "only the winner is in orbit");
        assert!(g.planets[fresia as usize].starbase.is_none(), "the star base is gone");
        assert!(g.fleets.contains_key(&attacker));
    });
    // The galaxy goes on: no second battle at the same planet, and the planet falls to the winner alone in orbit (capture rule). The
    // invariant against re-created battles (window 10 s) sees nothing either.
    let mut watch = BattleRepeats::new(10.0);
    let mut again = 0;
    for _ in 0..30 {
        let events = s.step(1.0).sim_events;
        watch.check(s.with_sim(|sim| sim.galaxy().time), &events);
        again += events.iter().filter(|e| matches!(e, Event::BattlePending(b) if b.planet == fresia)).count();
    }
    assert_eq!((again, watch.total()), (0, 0), "a won battle is over once");
    assert_eq!(s.with_sim(|sim| sim.galaxy().planets[fresia as usize].owner), Some(empire), "Fresia falls");
}

#[test]
fn a_pre_battle_save_sets_the_battle_up_with_the_whole_pirate_fleet() {
    // Saves keep no tactical set-up and no unit limit: a game saved with the battle pending (as the pre-battle autosave is, whatever rule the
    // build that wrote it had) sets the battle up again under the current rule when it is loaded.
    let Some(ctx) = load() else { return };
    let (s, battle, _, _, _) = fresia_attack(&ctx, &PIRATES_AT_FRESIA);
    let path = std::env::temp_dir().join(format!("eaw_remake_fresia_pre_battle_{}.json", std::process::id()));
    s.save(&path).expect("save");
    let fs: Arc<dyn FileSource> = ctx.fs.clone();
    let loaded = ScriptedSim::load(&path, fs, ctx.data.clone()).expect("load");
    let _ = std::fs::remove_file(&path);
    let w = loaded.with_sim(|sim| sim.world().clone());
    let req = loaded.with_sim_mut(|sim| sim.tactical_setup(battle)).expect("the battle is still pending after loading");
    let ps = req.sides.iter().position(|x| x.faction == w.faction_id("Pirates").unwrap()).unwrap();
    let want: BTreeMap<String, u32> = PIRATES_AT_FRESIA.iter().map(|&(n, c)| (n.to_owned(), c)).collect();
    assert_eq!(fleet_units(&req, ps), want);
    assert!(req.sides.iter().all(|x| x.unit_cap == 600), "the cap of the current rule");
}
