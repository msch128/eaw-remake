//! Units that object scripts bring into a battle (`spawns.rs`): hand-made definitions here, the Remake's scripts further down.

use std::sync::Arc;

use battle_sim::*;

fn ship(name: &str, class: TargetClass, radius: f32) -> ShipDef {
    ShipDef { name: name.into(), class, hull: 1000.0, radius, max_speed: 100.0, turn_rate: 1.0, accel: 50.0, ..ShipDef::default() }
}

fn gun() -> HpDef {
    HpDef {
        name: "gun".into(),
        kind: HpKind::Weapon,
        projectile: Some(0),
        range: 3000.0,
        arc_yaw: std::f32::consts::PI,
        arc_pitch: std::f32::consts::FRAC_PI_2,
        half_yaw: std::f32::consts::PI,
        half_pitch: std::f32::consts::FRAC_PI_2,
        recharge_min: 0.5,
        recharge_max: 0.5,
        pulses: 1,
        forward: Vec3::new(1.0, 0.0, 0.0),
        left: Vec3::new(0.0, 1.0, 0.0),
        normal: Vec3::new(0.0, 0.0, 1.0),
        ..HpDef::default()
    }
}

/// Escort script of the shape the Remake's hero scripts have: two escorts at the hero, 90 frames of hyperspace, 1 + 0..0.9 s apart.
fn escort_script(faction: &str, place: SpawnPlace, escort: ShipId) -> Arc<SpawnScriptDef> {
    Arc::new(SpawnScriptDef {
        script: "Test_Escort".into(),
        choices: vec![vec![escort, escort]],
        place,
        faction: faction.into(),
        delay: 0.0,
        hyperspace_in: 3.0,
        gap: SpawnGap { base: 1.0, lo: 0, hi: 9, div: 10.0 },
    })
}

fn units(cat: &Catalog, name: &str, list: &[&str]) -> SideSetup {
    SideSetup { name: name.into(), units: list.iter().map(|n| UnitSetup { ship: cat.ship_id(n).unwrap(), count: 1, tag: 7 }).collect(), structures: vec![], ai: false }
}

fn config(seed: u64) -> BattleConfig {
    BattleConfig { fog_of_war: false, seed, ..BattleConfig::default() }
}

/// `(battle time, event)` of every `Event::Spawned` within `secs`.
fn spawned(b: &mut Battle, secs: f32) -> Vec<(f32, u8, Vec<u32>, u32, bool)> {
    let mut out = Vec::new();
    while b.time() < secs && !b.is_over() {
        let t = b.time() + b.config().dt;
        for e in b.step() {
            if let Event::Spawned { side, ships, by, hyperspace } = e {
                out.push((t, *side, ships.clone(), *by, *hyperspace));
            }
        }
    }
    out
}

fn escort_catalog(faction: &str) -> Arc<Catalog> {
    let mut hero = ship("Hero", TargetClass::Capital, 300.0);
    hero.spawn_script = Some(escort_script(faction, SpawnPlace::ObjectsOfType("hero".into()), 1));
    Arc::new(Catalog::from_defs(vec![hero, ship("Escort", TargetClass::Frigate, 130.0), ship("Foe", TargetClass::Frigate, 130.0)], vec![ProjDef::default()]))
}

#[test]
fn hero_script_brings_its_escorts_in_one_by_one_after_the_hyperspace_delay() {
    let cat = escort_catalog("Rebel");
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Rebel", &["Hero"]), units(&cat, "Empire", &["Foe"])], config: config(3) });
    let got = spawned(&mut b, 8.0);
    assert_eq!(got.len(), 2, "{got:?}");
    // The script starts on the first step (1/30 s); each escort arrives 90 frames after its spawn, the second 1.0 to 1.9 s after the first.
    assert!((got[0].0 - (3.0 + 1.0 / 30.0)).abs() < 0.04, "first at {}", got[0].0);
    let gap = got[1].0 - got[0].0;
    assert!((0.99..=1.94).contains(&gap), "gap {gap}");
    let hero = b.ships.iter().find(|s| s.def == 0).unwrap();
    for (_, side, ships, by, hyperspace) in &got {
        assert_eq!((*side, *by, *hyperspace, ships.len()), (0, hero.id, true, 1));
        let e = &b.ships[ships[0] as usize];
        assert_eq!(e.def, 1);
        // Beside the hero, not inside it.
        let d = e.pos.dist(hero.pos);
        assert!((300.0..1500.0).contains(&d), "escort {d} from the hero");
    }
    assert!(b.ships[got[0].2[0] as usize].pos.dist(b.ships[got[1].2[0] as usize].pos) > 150.0, "escorts side by side");
}

#[test]
fn script_units_are_battle_only() {
    let cat = escort_catalog("Rebel");
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Rebel", &["Hero"]), units(&cat, "Empire", &["Foe"])], config: config(3) });
    spawned(&mut b, 6.0);
    assert_eq!(b.ships.iter().filter(|s| s.def == 1).count(), 2);
    // No results group: the galaxy gets nothing back for them.
    assert!(b.result().groups.iter().all(|g| g.ship != 1), "{:?}", b.result().groups);
}

#[test]
fn script_units_join_the_side_of_their_faction() {
    let cat = escort_catalog("Empire");
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Rebel", &["Hero"]), units(&cat, "Empire", &["Foe"])], config: config(3) });
    assert!(spawned(&mut b, 6.0).iter().all(|e| e.1 == 1));
    // A faction that is not in the battle: the units fight for the script's owner.
    let cat = escort_catalog("Pirates");
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Rebel", &["Hero"]), units(&cat, "Empire", &["Foe"])], config: config(3) });
    let got = spawned(&mut b, 6.0);
    assert!(!got.is_empty() && got.iter().all(|e| e.1 == 0));
}

#[test]
fn the_roll_picks_one_list_from_the_seed() {
    let mut hero = ship("Hero", TargetClass::Capital, 300.0);
    let mut script = (*escort_script("Rebel", SpawnPlace::ObjectsOfType("hero".into()), 1)).clone();
    script.choices = vec![vec![1], vec![2, 2], vec![]];
    hero.spawn_script = Some(Arc::new(script));
    let cat = Arc::new(Catalog::from_defs(vec![hero, ship("A", TargetClass::Frigate, 130.0), ship("B", TargetClass::Corvette, 60.0), ship("Foe", TargetClass::Frigate, 130.0)], vec![ProjDef::default()]));
    let run = |seed: u64| {
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Rebel", &["Hero"]), units(&cat, "Empire", &["Foe"])], config: config(seed) });
        spawned(&mut b, 8.0).iter().map(|e| b.ships[e.2[0] as usize].def).collect::<Vec<_>>()
    };
    let outcomes: Vec<Vec<ShipId>> = (1..40).map(run).collect();
    assert_eq!(run(5), run(5), "same seed, same roll");
    for o in &outcomes {
        assert!(o.is_empty() || *o == vec![1] || *o == vec![2, 2], "{o:?}");
    }
    for want in [vec![], vec![1], vec![2, 2]] {
        assert!(outcomes.contains(&want), "outcome {want:?} never rolled");
    }
}

#[test]
fn a_single_target_healer_skips_its_excluded_types_when_it_picks() {
    // A wearing-out healer of an excluded type is the most damaged ship near it; the heal must still go to the friend.
    let corvette_heal = HealDef { range: 1000.0, amount: 75.0, interval: 2.0, single: true, classes: TargetClass::Corvette.bit(), excluded: vec!["healer".into()], ..HealDef::default() };
    let healer = ShipDef { heals: vec![corvette_heal], ..ship("Healer", TargetClass::Corvette, 50.0) };
    let cat = Arc::new(Catalog::from_defs(vec![healer, ship("Friend", TargetClass::Corvette, 60.0), ship("Foe", TargetClass::Frigate, 130.0)], vec![ProjDef::default()]));
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "A", &["Healer", "Friend"]), units(&cat, "B", &["Foe"])], config: config(1) });
    let (h, f) = (b.ships.iter().position(|s| s.def == 0).unwrap(), b.ships.iter().position(|s| s.def == 1).unwrap());
    b.ships[h].hull = 100.0;
    b.ships[f].hull = 500.0;
    b.step();
    assert_eq!((b.ships[h].hull, b.ships[f].hull), (100.0, 575.0));
}

#[test]
fn a_hardpoint_breaking_off_runs_its_prop_script_at_the_map_markers() {
    let mut target = ship("Target", TargetClass::Capital, 300.0);
    let mut hp = HpDef { name: "turret".into(), kind: HpKind::Other, health: 50.0, destroyable: true, targetable: true, ..HpDef::default() };
    let mut s = (*escort_script("Rebel", SpawnPlace::DefenderPositions, 1)).clone();
    s.choices = vec![vec![1, 1, 1]];
    hp.breakoff_script = Some(Arc::new(s));
    // A second, sturdy hardpoint keeps the target alive.
    let hull = HpDef { name: "hull".into(), kind: HpKind::Other, health: 1.0e6, destroyable: true, ..HpDef::default() };
    target.hardpoints = vec![hp, hull];
    target.max_speed = 0.0;
    let mut shooter = ship("Shooter", TargetClass::Frigate, 130.0);
    shooter.hardpoints = vec![gun()];
    let proj = ProjDef { name: "laser".into(), damage: 100.0, speed: 3000.0, max_distance: 5000.0, shield_damage: true, hull_damage: true, ..ProjDef::default() };
    let cat = Arc::new(Catalog::from_defs(vec![target, ship("Reinforcement", TargetClass::Frigate, 130.0), shooter], vec![proj]));
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "Empire", &["Shooter"]), units(&cat, "Rebel", &["Target"])], config: config(2) });
    // Put the shooter next to its target.
    let t = b.ships.iter().find(|s| s.def == 0).unwrap().pos;
    let sh = b.ships.iter().position(|s| s.def == 2).unwrap();
    b.ships[sh].pos = t + Vec3::new(-1200.0, 0.0, 0.0);
    assert!(spawned(&mut b, 2.0).is_empty(), "nothing before the hardpoint breaks");
    let got = spawned(&mut b, 20.0);
    assert_eq!(got.len(), 3, "{got:?}");
    // Without a map the defender positions are side 1's start place.
    let start = b.ships.iter().find(|s| s.def == 0).unwrap().pos;
    for e in &got {
        assert_eq!(e.1, 1);
        assert!(b.ships[e.2[0] as usize].pos.dist(start) < 3000.0);
    }
}

// The Remake's scripts; skipped when the game is not installed.

fn load() -> Option<(alamo_data::GameData, alamo_fs::GameFs)> {
    let install = alamo_fs::GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = alamo_fs::GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = alamo_data::load_galaxy(&fs).0;
    Some((data, fs))
}

fn names(cat: &Catalog, ids: &[ShipId]) -> Vec<String> {
    ids.iter().map(|&i| cat.ship(i).name.clone()).collect()
}

#[test]
fn hero_escort_scripts_come_from_the_lua() {
    let Some((data, fs)) = load() else { return };
    let heroes = ["Predominant_Star_Destroyer", "TF-24_Requiem", "Voca_Blade", "Savrip", "VSD_Protector", "Voca_Blade_Warlord"];
    let cat = Catalog::build(&data, Some(&fs), &heroes);
    let script = |hero: &str| cat.ship(cat.ship_id(hero).unwrap()).spawn_script.clone();
    // `Reinforcements_<hero>.lua`: one list, rolled with GameRandom(1, 1), spawned at the hero for the faction named in the call.
    let want: [(&str, &str, &[&str], &str); 5] = [
        ("Predominant_Star_Destroyer", "Reinforcements_Zaarin", &["Vindicator_Cruiser_Kas", "Vindicator_Cruiser_Kas"], "Empire"),
        ("TF-24_Requiem", "Reinforcements_TF24", &["TF-24_Relentless", "TF-24_Ravenous"], "Empire"),
        ("Voca_Blade", "Reinforcements_Voca", &["Voca_Destruction", "Voca_Fist", "Secutor_Inferno", "Voca_Lancer"], "Empire"),
        ("Savrip", "Reinforcements_Noira", &["Renegades_Blood"], "Underworld"),
        ("VSD_Protector", "Reinforcements_Harkov", &["Neb_Akaga", "Neb_Kagi", "Neb_Siryu", "Neb_Horyu"], "Warlords"),
    ];
    for (hero, name, units, faction) in want {
        let s = script(hero).unwrap_or_else(|| panic!("{hero} has no spawn script"));
        assert_eq!(s.script, name);
        assert_eq!(s.choices.len(), 1, "{hero}");
        assert_eq!(names(&cat, &s.choices[0]), units, "{hero}");
        assert_eq!(s.place, SpawnPlace::ObjectsOfType(hero.to_ascii_lowercase()));
        assert_eq!(s.faction, faction);
        assert_eq!((s.delay, s.hyperspace_in), (0.0, 3.0), "{hero}: Cinematic_Hyperspace_In(90)");
        assert_eq!(s.gap, SpawnGap { base: 1.0, lo: 0, hi: 9, div: 10.0 }, "{hero}: Sleep(1 + GameRandom(0,9) / 10)");
    }
    // `Reinforcements_Voca_Warlord` is commented out on the Warlord Voca.
    assert!(script("Voca_Blade_Warlord").is_none());
}

#[test]
fn subjugator_ion_turrets_break_off_into_reinforcement_scripts() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Subjugator_Zann_P3"]);
    let def = cat.ship(cat.ship_id("Subjugator_Zann_P3").unwrap());
    let hp = |n: &str| def.hardpoints.iter().find(|h| h.name == n).unwrap().breakoff_script.clone().unwrap();
    // `Hardpoint_Breakoff_MIT_01` / `_02` carry `Reinforcement_Subjugator-Attack` / `-Defend`: GameRandom(1,3) picks one of three lists.
    for (n, script, place) in [("HP_SBJZ_P3_ICT_01", "Reinforcement_Subjugator-Attack", SpawnPlace::AttackerEntries), ("HP_SBJZ_P3_ICT_02", "Reinforcement_Subjugator-Defend", SpawnPlace::DefenderPositions)] {
        let s = hp(n);
        assert_eq!((s.script.as_str(), &s.place, s.faction.as_str()), (script, &place, "Underworld"));
        assert_eq!(s.choices.iter().map(Vec::len).collect::<Vec<_>>(), vec![19, 22, 15], "{n}");
        assert_eq!(names(&cat, &s.choices[0][17..]), ["Sabaoth_Destroyer", "Sabaoth_Destroyer"]);
        assert_eq!(names(&cat, &s.choices[2][13..]), ["Krayt_Class_Destroyer", "Recusant_Destroyer_II"]);
    }
    assert_eq!(def.hardpoints.iter().filter(|h| h.breakoff_script.is_some()).count(), 2);
}

#[test]
fn zaarin_arrives_with_two_vindicators_beside_him() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Predominant_Star_Destroyer", "Nebulon_B_Frigate"]));
    let side = |name: &str, unit: &str| SideSetup { name: name.into(), units: vec![UnitSetup { ship: cat.ship_id(unit).unwrap(), count: 1, tag: 1 }], structures: vec![], ai: true };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("Empire", "Predominant_Star_Destroyer"), side("Rebel", "Nebulon_B_Frigate")], config: BattleConfig { seed: 9, ..BattleConfig::default() } });
    let got = spawned(&mut b, 8.0);
    let kas = cat.ship_id("Vindicator_Cruiser_Kas").unwrap();
    assert_eq!(got.iter().map(|e| (e.1, b.ships[e.2[0] as usize].def)).collect::<Vec<_>>(), vec![(0, kas), (0, kas)]);
    assert!((got[0].0 - 3.033).abs() < 0.04 && (0.99..=1.94).contains(&(got[1].0 - got[0].0)), "{:?}", got.iter().map(|e| e.0).collect::<Vec<_>>());
    let zaarin = b.ships.iter().find(|s| s.def == cat.ship_id("Predominant_Star_Destroyer").unwrap()).unwrap();
    for e in &got {
        assert!(b.ships[e.2[0] as usize].pos.dist(zaarin.pos) < 2500.0);
    }
    // The escorts fight for the Empire but are not part of the fleet the galaxy gets back.
    assert!(b.result().groups.iter().all(|g| g.ship != kas));
    // Same seed, same battle.
    let mut again = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("Empire", "Predominant_Star_Destroyer"), side("Rebel", "Nebulon_B_Frigate")], config: BattleConfig { seed: 9, ..BattleConfig::default() } });
    spawned(&mut again, 8.0);
    assert_eq!(again.state_hash(), b.state_hash());
    // Losing one costs its `Destruction_Survivors` death penalty (`Zaarin_Vindi_Penalty_Dummy`, 9000 credits), as in the Remake.
    assert_eq!(b.result().attrition_credits[0], 0);
    b.ships[got[0].2[0] as usize].alive = false;
    assert_eq!(b.result().attrition_credits[0], -9000);
}

#[test]
fn a_broken_subjugator_turret_brings_reinforcements_to_the_attacker_entries() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Subjugator_Zann_P3", "Star_Destroyer"]));
    let arena = Arena::load(&data, &fs, "_Space_Planet_Coruscant_02.ted").expect("map");
    let entries = arena.attacker_entries.clone();
    let side = |name: &str, unit: &str| SideSetup { name: name.into(), units: vec![UnitSetup { ship: cat.ship_id(unit).unwrap(), count: 1, tag: 1 }], structures: vec![], ai: false };
    let config = BattleConfig { seed: 4, arena: Some(Arc::new(arena)), fog_of_war: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("Empire", "Star_Destroyer"), side("Underworld", "Subjugator_Zann_P3")], config });
    let sub = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    let def = cat.ship(b.ships[sub as usize].def);
    let ict = def.hardpoints.iter().position(|h| h.name == "HP_SBJZ_P3_ICT_01").unwrap();
    b.ships[sub as usize].hps[ict].health = 1.0;
    // An Imperial broadside at the turret.
    let isd = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    let at = b.ships[sub as usize].pos + Vec3::new(0.0, -2500.0, 0.0);
    b.ships[isd as usize].pos = at;
    b.command(0, Command::AttackHardpoint { ships: vec![isd], target: sub, hardpoint: ict as u16 }).unwrap();
    let mut broke = None;
    let mut got = Vec::new();
    while b.time() < 60.0 && !b.is_over() {
        let t = b.time() + b.config().dt;
        for e in b.step() {
            match e {
                Event::HardpointDestroyed { ship, hardpoint, .. } if *ship == sub && *hardpoint as usize == ict => broke = Some(t),
                Event::Spawned { side, ships, .. } => got.push((t, *side, ships.clone())),
                _ => {}
            }
        }
    }
    let broke = broke.expect("the turret broke off");
    // One of the three lists of `Reinforcement_Subjugator-Attack` (19, 22 or 15 units), for the Underworld side.
    assert!([19, 22, 15].contains(&got.len()), "{} units", got.len());
    assert!(got.iter().all(|e| e.1 == 1));
    assert!((got[0].0 - broke - 3.0).abs() < 0.05, "first unit {} s after the break", got[0].0 - broke);
    // They cycle through the three `Attacker Entry Position` markers.
    let nearest = |p: Vec3| (0..entries.len()).min_by(|&a, &c| entries[a].0.dist(p).total_cmp(&entries[c].0.dist(p))).unwrap();
    let markers: Vec<usize> = got.iter().take(3).map(|e| nearest(b.ships[e.2[0] as usize].pos)).collect();
    assert_eq!(markers, vec![0, 1, 2]);
}

#[test]
fn station_defence_fleet_scripts_are_switched_off_in_the_data() {
    use alamo_fs::FileSource;
    let Some((data, fs)) = load() else { return };
    // `Buildings/Space/GC/Minors/Starbase.xml` and `Pirates/Starbase.xml` comment out every `Lua_Script` of the star bases.
    let minors = ["CIS", "Republic", "CSA", "Hapan", "Mandalorian", "Warlords", "Cartels", "Sith"];
    let mut bases: Vec<String> = minors.iter().flat_map(|m| [format!("Minor_Starbase_{m}"), format!("Minor_Starbase_{m}_2"), format!("Minor_Starbase_{m}_3")]).collect();
    bases.extend(["Pirate_Starbase_Lv1".to_string(), "Pirate_Starbase_Lv2".to_string()]);
    let refs: Vec<&str> = bases.iter().map(String::as_str).collect();
    let cat = Catalog::build(&data, Some(&fs), &refs);
    for b in &bases {
        let id = cat.ship_id(b).unwrap_or_else(|| panic!("{b}"));
        assert!(cat.ship(id).spawn_script.is_none(), "{b} has a spawn script");
    }
    // The scripts themselves read fine: switched back on, a base would roll one of its lists for the defender markers.
    let script = |name: &str| parse_spawn_script(&String::from_utf8_lossy(&fs.read(&format!("Scripts/GameObject/{name}.lua")).unwrap())).unwrap();
    let rolls = [
        ("Cartels", [3, 3, 2]),
        ("CIS", [4, 4, 4]),
        ("CSA", [3, 5, 4]),
        ("Hapan", [2, 3, 3]),
        ("Mandalorians", [3, 3, 4]),
        ("Republic", [4, 4, 4]),
        ("Sith", [2, 3, 5]),
        ("Warlords", [5, 5, 5]),
    ];
    for (faction, lists) in rolls {
        for (level, n) in ["", "_Lvl2", "_Lvl3"].iter().zip(lists) {
            let s = script(&format!("Reinforcements_Starbase_{faction}{level}"));
            assert_eq!((s.choices.len(), s.marker.as_str(), s.faction.as_str()), (n, "Defending Forces Position", faction), "{faction}{level}");
            assert!(s.choices.iter().all(|c| !c.is_empty()), "{faction}{level}: {:?}", s.choices);
        }
    }
    let cis = script("Reinforcements_Starbase_CIS");
    assert_eq!(cis.choices[1], ["Gozanti_Squadron_CIS", "Recusant_Destroyer_CIS"]);
    let pirates = script("Reinforcements_Pirate_Base_Lv2");
    assert_eq!(pirates.choices.iter().map(|c| c.join(",")).collect::<Vec<_>>(), ["Dreadnaught_Cruiser", "Victory_Destroyer", "Providence_Destroyer", "Nebulon_B_Frigate", "Corellian_Destroyer", "Acclamator"]);
}

// Helper objects placed by abilities.

fn ability<'a>(cat: &'a Catalog, unit: &str, kind: &str) -> (u16, &'a AbilityDef) {
    let d = cat.ship(cat.ship_id(unit).unwrap_or_else(|| panic!("{unit}")));
    let i = d.abilities.iter().position(|a| a.kind.eq_ignore_ascii_case(kind)).unwrap_or_else(|| panic!("{unit} {kind}"));
    (i as u16, &d.abilities[i])
}

#[test]
fn repair_droid_pings_place_healers_instead_of_revealing() {
    let Some((data, fs)) = load() else { return };
    let units = ["Nebulon_B_Frigate", "Gladiator_Star_Destroyer", "MTC_Repair", "Havelon", "Rebel_One", "Broadside_Cruiser"];
    let cat = Catalog::build(&data, Some(&fs), &units);
    let droids = cat.ship_id("Repair_Droids").expect("the droids are in the catalog");
    // `FOW_REVEAL_PING` with `Spawned_Object_Type` Repair_Droids: `Effective_Radius` is how far it may be placed, `Recharge_Seconds` as given.
    for (unit, reach, recharge) in [("Nebulon_B_Frigate", 1000.0, 120.0), ("Gladiator_Star_Destroyer", 1000.0, 120.0), ("MTC_Repair", 1000.0, 120.0), ("Havelon", 1000.0, 120.0), ("Rebel_One", 1500.0, 60.0)] {
        let (_, a) = ability(&cat, unit, "FOW_REVEAL_PING");
        let h = a.helper.as_ref().unwrap_or_else(|| panic!("{unit}"));
        assert_eq!((h.ship, h.max_range, h.delay, h.own_dps, a.recharge, a.reveal_radius), (droids, reach, 0.0, 0.0, recharge, 0.0), "{unit}");
    }
    // The Broadside's ping spawns a projectile: it stays a reveal ping.
    assert!(ability(&cat, "Broadside_Cruiser", "FOW_REVEAL_PING").1.helper.is_none());
    // `Repair_Droids`: 60 hit points, `Is_Valid_Target` False, not collidable, `ObjectScript_TakeDamage` takes 1 per second, and its
    // `Force_Healing_Ability` mends 75 every 2 s, one ship within 1000 (corvette to capital, never the droids).
    let d = cat.ship(droids);
    assert_eq!((d.hull, d.untargetable, d.no_collide, d.decay, d.population), (60.0, true, true, 1.0, 0));
    let h = &d.heals[0];
    assert_eq!((h.range, h.amount, h.interval, h.single), (1000.0, 75.0, 2.0, true));
    assert_eq!(h.classes, [TargetClass::Corvette, TargetClass::Frigate, TargetClass::Capital, TargetClass::Transport].iter().fold(0, |m, c| m | c.bit()));
    assert_eq!(h.excluded, ["repair_droids", "repair_droids_fighters"]);
}

#[test]
fn buzz_droid_abilities_place_fighter_repair_drones_and_jamming_pods() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["U-Wing_Squadron", "Gozanti_Squadron", "TIE_Interdictor_Squadron", "TIE_Scimitar_Squadron", "Eclipse_Star_Dreadnought_P4"]);
    let drones = cat.ship_id("Repair_Droids_Fighters").unwrap();
    let pods = cat.ship_id("Missile_Jamming_Pods_Empire").unwrap();
    // `Buzz_Droids_Ability`: `Activation_Max_Range` 500, `Activation_Time_In_Seconds` 1, `Own_Damage_Per_Second` 1, no enemy damage.
    for (unit, object, recharge) in [("U-Wing_LC", drones, 120.0), ("Gozanti", drones, 120.0), ("TIE_Interdictor", pods, 450.0), ("TIE_Scimitar", pods, 450.0)] {
        let (_, a) = ability(&cat, unit, "BUZZ_DROIDS");
        let h = a.helper.as_ref().unwrap_or_else(|| panic!("{unit}"));
        assert_eq!((h.ship, h.max_range, h.delay, h.own_dps, a.recharge), (object, 500.0, 1.0, 1.0, recharge), "{unit}");
        assert!(a.zone.is_none());
    }
    // The Eclipse's lightning storm does damage: it stays a damage zone.
    let (_, storm) = ability(&cat, "Eclipse_Star_Dreadnought_P4", "BUZZ_DROIDS");
    assert!(storm.helper.is_none() && storm.zone.is_some());
    // Fighter drones: 60 hit points, 5 every second to every fighter, bomber, gunship and transport within 500; no script of their own.
    let d = cat.ship(drones);
    assert_eq!((d.hull, d.decay, d.untargetable), (60.0, 0.0, true));
    assert_eq!((d.heals[0].range, d.heals[0].amount, d.heals[0].interval, d.heals[0].single), (500.0, 5.0, 1.0, false));
    // Jamming pods: 60 hit points, `Passive_Missile_Shield_Radius` 500, not a target but they can be hit.
    let p = cat.ship(pods);
    assert_eq!((p.hull, p.passive_missile_radius, p.untargetable, p.no_collide, p.max_speed), (60.0, 500.0, true, false, 0.0));
}

fn helper_battle(cat: &Arc<Catalog>, own: &str, foe: &str, ai: bool) -> Battle {
    let side = |name: &str, unit: &str, ai: bool| SideSetup { name: name.into(), units: vec![UnitSetup { ship: cat.ship_id(unit).unwrap(), count: 1, tag: 1 }], structures: vec![], ai };
    Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("Rebel", own, ai), side("Empire", foe, false)], config: BattleConfig { seed: 6, fog_of_war: false, ..BattleConfig::default() } })
}

#[test]
fn repair_droids_mend_a_damaged_frigate_and_wear_out_after_60_s() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Nebulon_B_Frigate", "Star_Destroyer"]));
    let mut b = helper_battle(&cat, "Nebulon_B_Frigate", "Star_Destroyer", false);
    let neb = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    let def = cat.ship(b.ships[neb as usize].def);
    // Half health on every hardpoint (its hull is their sum).
    for (i, h) in def.hardpoints.iter().enumerate().filter(|(_, h)| h.destroyable) {
        b.ships[neb as usize].hps[i].health = h.health * 0.5;
    }
    b.ships[neb as usize].hull = def.hull * 0.5;
    b.step();
    let start = b.ships[neb as usize].hull;
    let (k, _) = ability(&cat, "Nebulon_B_Frigate", "FOW_REVEAL_PING");
    let at = b.ships[neb as usize].pos + Vec3::new(400.0, 300.0, 0.0);
    b.command(0, Command::UseAbilityAt { ship: neb, ability: k, pos: at }).unwrap();
    let got = spawned(&mut b, 1.0);
    assert_eq!(got.len(), 1);
    let (t0, side, ships, by, hyperspace) = got[0].clone();
    assert_eq!((side, by, hyperspace), (0, neb, false));
    let droid = ships[0];
    let d = b.ships[droid as usize].pos;
    assert!((d.x - at.x).hypot(d.y - at.y) < 1.0, "placed where aimed: {d:?} {at:?}");
    assert_eq!(b.ships[droid as usize].def, cat.ship_id("Repair_Droids").unwrap());
    // 75 hull points every 2 s: pulses at 0, 2, 4, 6 and 8 s.
    while b.time() < t0 + 9.0 {
        b.step();
    }
    let healed = b.ships[neb as usize].hull - start;
    assert!((healed - 375.0).abs() < 1.0, "healed {healed}");
    // 1 hit point per second off 60.
    assert!((b.ships[droid as usize].hull - (60.0 - 9.0)).abs() < 0.2, "droid at {}", b.ships[droid as usize].hull);
    let mut gone = None;
    while b.time() < t0 + 62.0 {
        let t = b.time() + b.config().dt;
        if b.step().iter().any(|e| matches!(e, Event::ShipDestroyed { ship, .. } if *ship == droid)) {
            gone = Some(t);
        }
    }
    let gone = gone.expect("the droids wore out");
    assert!((gone - t0 - 60.0).abs() < 0.1, "lasted {} s", gone - t0);
    // Not in the results: the droids belong to the battle.
    assert!(b.result().groups.iter().all(|g| g.ship != cat.ship_id("Repair_Droids").unwrap()));
}

#[test]
fn nothing_aims_at_or_hits_repair_droids() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Nebulon_B_Frigate", "Star_Destroyer"]));
    let mut b = helper_battle(&cat, "Nebulon_B_Frigate", "Star_Destroyer", false);
    let neb = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    let isd = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    // The Imperial ship sits close, aims at the frigate, and the droids lie between them.
    let p = b.ships[neb as usize].pos;
    b.ships[isd as usize].pos = p + Vec3::new(2500.0, 0.0, 0.0);
    let (k, _) = ability(&cat, "Nebulon_B_Frigate", "FOW_REVEAL_PING");
    b.command(0, Command::UseAbilityAt { ship: neb, ability: k, pos: p + Vec3::new(600.0, 0.0, 0.0) }).unwrap();
    let droid = spawned(&mut b, 0.5)[0].2[0];
    assert_eq!(b.command(1, Command::Attack { ships: vec![isd], target: droid }), Err(CommandError::InvalidShip));
    let (mut hits, mut aimed, mut frigate_hits) = (0, 0, 0);
    while b.time() < 15.0 && !b.is_over() {
        for e in b.step() {
            if let Event::Hit { ship, .. } = e {
                hits += usize::from(*ship == droid);
                frigate_hits += usize::from(*ship == neb);
            }
        }
        aimed += usize::from(b.ships.iter().any(|s| s.side == 1 && s.target == droid));
    }
    assert_eq!((hits, aimed), (0, 0));
    assert!(frigate_hits > 0, "the frigate was under fire");
}

#[test]
fn the_ai_leaves_repair_droid_pings_to_the_player() {
    // No Remake AI script uses `FOW_REVEAL_PING`, and the ping has no `Supports_Autofire`.
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Nebulon_B_Frigate", "Star_Destroyer"]));
    let mut b = helper_battle(&cat, "Nebulon_B_Frigate", "Star_Destroyer", true);
    let neb = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    let def = cat.ship(b.ships[neb as usize].def);
    for (i, h) in def.hardpoints.iter().enumerate().filter(|(_, h)| h.destroyable) {
        b.ships[neb as usize].hps[i].health = h.health * 0.5;
    }
    b.ships[neb as usize].hull = def.hull * 0.5;
    assert!(spawned(&mut b, 10.0).is_empty());
}

#[test]
fn an_ai_squadron_drops_one_jamming_pod_at_its_target_in_reach() {
    // `Default_Target_In_Range` (`PGEvents.lua`): `Try_Ability(unit, "BUZZ_DROIDS", target)`.
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["TIE_Interdictor_Squadron", "X-Wing_Squadron"]));
    let side = |name: &str, unit: &str, ai: bool| SideSetup { name: name.into(), units: vec![UnitSetup { ship: cat.ship_id(unit).unwrap(), count: 1, tag: 1 }], structures: vec![], ai };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("Empire", "TIE_Interdictor_Squadron", true), side("Rebel", "X-Wing_Squadron", false)], config: BattleConfig { seed: 3, fog_of_war: false, ..BattleConfig::default() } });
    let fighters = b.ships.iter().filter(|s| s.side == 0).count();
    // The X-wings wait 1500 ahead of the Interdictors.
    let shift = b.ships.iter().find(|s| s.side == 0).unwrap().pos + Vec3::new(1500.0, 0.0, 0.0) - b.ships.iter().find(|s| s.side == 1).unwrap().pos;
    for s in b.ships.iter_mut().filter(|s| s.side == 1) {
        s.pos += shift;
    }
    let got = spawned(&mut b, 20.0);
    assert!(fighters > 1);
    assert_eq!(got.len(), 1, "one pod for the squadron: {got:?}");
    let pod = &b.ships[got[0].2[0] as usize];
    assert_eq!(pod.def, cat.ship_id("Missile_Jamming_Pods_Empire").unwrap());
    let by = &b.ships[got[0].3 as usize];
    assert!(pod.pos.dist(by.pos) < 1500.0, "near the caster");
}

#[test]
fn a_jamming_pod_appears_after_a_second_within_reach_and_shields_against_missiles() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["TIE_Interdictor_Squadron", "Nebulon_B_Frigate"]));
    let side = |name: &str, unit: &str| SideSetup { name: name.into(), units: vec![UnitSetup { ship: cat.ship_id(unit).unwrap(), count: 1, tag: 1 }], structures: vec![], ai: false };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("Empire", "TIE_Interdictor_Squadron"), side("Rebel", "Nebulon_B_Frigate")], config: BattleConfig { seed: 2, fog_of_war: false, ..BattleConfig::default() } });
    let tie = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    let (k, _) = ability(&cat, "TIE_Interdictor", "BUZZ_DROIDS");
    let from = b.ships[tie as usize].pos;
    b.command(0, Command::UseAbilityAt { ship: tie, ability: k, pos: from + Vec3::new(3000.0, 0.0, 0.0) }).unwrap();
    let t0 = b.time();
    let got = spawned(&mut b, 3.0);
    assert_eq!(got.len(), 1);
    assert!((got[0].0 - t0 - 1.0).abs() < 0.05, "after {} s", got[0].0 - t0);
    let pod = got[0].2[0];
    assert!((b.ships[pod as usize].pos.dist(from) - 500.0).abs() < 1.0, "pulled back to the 500 reach");
    assert!((b.ships[pod as usize].decay - 1.0).abs() < 1e-6);
}

#[test]
fn a_passive_missile_shield_destroys_enemy_missiles_only() {
    let missile = ProjDef { name: "missile".into(), category: "missile".into(), damage: 50.0, speed: 600.0, max_distance: 6000.0, shield_damage: true, hull_damage: true, ..ProjDef::default() };
    let mut launcher = ship("Launcher", TargetClass::Frigate, 130.0);
    launcher.hardpoints = vec![HpDef { range: 4000.0, ..gun() }];
    launcher.max_speed = 0.0;
    let mut target = ship("Target", TargetClass::Frigate, 130.0);
    target.max_speed = 0.0;
    let run = |radius: f32| {
        let mut t = target.clone();
        t.passive_missile_radius = radius;
        let cat = Arc::new(Catalog::from_defs(vec![launcher.clone(), t], vec![missile.clone()]));
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![units(&cat, "A", &["Launcher"]), units(&cat, "B", &["Target"])], config: config(1) });
        let p = b.ships[1].pos;
        b.ships[0].pos = p + Vec3::new(-2000.0, 0.0, 0.0);
        let (mut hits, mut stopped) = (0, 0);
        while b.time() < 10.0 {
            for e in b.step() {
                match e {
                    Event::Hit { ship: 1, .. } => hits += 1,
                    Event::ProjectileIntercepted { .. } => stopped += 1,
                    _ => {}
                }
            }
        }
        (hits, stopped)
    };
    let (hits, stopped) = run(0.0);
    assert!(hits > 0 && stopped == 0, "{hits} {stopped}");
    let (hits, stopped) = run(500.0);
    assert!(hits == 0 && stopped > 0, "{hits} {stopped}");
}

#[test]
fn every_spawned_type_has_its_model_for_the_view() {
    use alamo_fs::FileSource;
    let Some((data, fs)) = load() else { return };
    let spawners = ["Predominant_Star_Destroyer", "TF-24_Requiem", "Voca_Blade", "Savrip", "VSD_Protector", "Subjugator_Zann_P3", "Nebulon_B_Frigate", "U-Wing_Squadron", "TIE_Interdictor_Squadron"];
    let cat = Catalog::build(&data, Some(&fs), &spawners);
    let mut types: Vec<ShipId> = spawners.iter().flat_map(|n| cat.ship(cat.ship_id(n).unwrap()).script_spawns()).collect();
    // Squadrons are drawn as their craft.
    types = types.into_iter().flat_map(|t| if cat.ship(t).is_squadron { cat.ship(t).members.iter().map(|m| m.0).collect() } else { vec![t] }).collect();
    types.sort_unstable();
    types.dedup();
    assert!(types.len() >= 29, "{} types", types.len());
    for t in types {
        let d = cat.ship(t);
        let model = d.model.as_deref().unwrap_or_else(|| panic!("{} has no model", d.name));
        assert!(fs.exists(&format!("Art/Models/{model}")), "{}: {model} missing", d.name);
    }
}
