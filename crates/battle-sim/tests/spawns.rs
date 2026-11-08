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
