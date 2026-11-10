//! The tactical build bar of star bases and stations (`buildables.rs`) on the real Remake data; skipped when the game is not installed.

use std::sync::Arc;

use alamo_fs::FileSource;
use battle_sim::*;

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

fn names(cat: &Catalog, ids: impl IntoIterator<Item = buildables::BuildId>) -> Vec<String> {
    ids.into_iter().map(|b| cat.buildable(b).name.clone()).collect()
}

/// A Rebel frigate and a defender `faction` with `base` and `units`; `credits` for the defender.
fn defended(cat: &Arc<Catalog>, faction: &str, base: &str, units: &[(&str, u32)], credits: i64) -> Battle {
    let ship = |n: &str| cat.ship_id(n).unwrap_or_else(|| panic!("{n}"));
    let attacker = SideSetup { name: "Rebel".into(), units: vec![UnitSetup { ship: ship("Nebulon_B_Frigate"), count: 1, tag: 1 }], structures: vec![], ai: false };
    let units = units.iter().map(|&(n, count)| UnitSetup { ship: ship(n), count, tag: 2 }).collect();
    let defender = SideSetup { name: faction.into(), units, structures: vec![ship(base)], ai: false };
    Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![attacker, defender], config: BattleConfig { side_credits: vec![0, credits], fog_of_war: false, ..BattleConfig::default() } })
}

#[test]
fn star_bases_offer_their_levels_tactical_builds_to_their_faction() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Empire_Star_Base_1", "Empire_Star_Base_3", "Rebel_Star_Base_2", "Rebel_Star_Base_3", "Underworld_Star_Base_3", "E_Mercenary_Outpost"]);
    let bar = |n: &str| cat.ship(cat.ship_id(n).unwrap()).buildable.iter().map(|(f, b)| format!("{f}:{}", cat.buildable(*b).name)).collect::<Vec<_>>();
    // `Buildings/Space/GC/Empire/Colonies.xml`.
    assert_eq!(bar("Empire_Star_Base_1"), ["empire:ES_Enhanced_Reactors_L1_Upgrade", "empire:ES_Reinforced_Armor_L1_Upgrade"]);
    assert_eq!(bar("Empire_Star_Base_3").len(), 7);
    assert_eq!(bar("Rebel_Star_Base_2"), ["rebel:RS_Enhanced_Shielding_L1_Upgrade", "rebel:RS_Improved_Weapons_L1_Upgrade", "rebel:RS_Reinforced_Armor_L1_Upgrade"]);
    // A mercenary outpost offers each main faction its own debuffs.
    let outpost = bar("E_Mercenary_Outpost");
    assert_eq!(outpost.len(), 6, "{outpost:?}");
    assert!(outpost.contains(&"rebel:Rebel_Enemies_-7%_Damage_Debuff".to_string()));
    // `Upgrades/GC/Space/Empire/Unit_Damage.xml`.
    let l2 = cat.buildable(cat.buildable_id("ES_Improved_Weapons_L2_Upgrade").unwrap());
    assert_eq!((l2.cost, l2.time, l2.limit), (160, 35.0, 1));
    assert_eq!(names(&cat, l2.prerequisites.clone()), ["ES_Improved_Weapons_L1_Upgrade"]);
    assert_eq!(l2.replaces, cat.buildable_id("ES_Improved_Weapons_L1_Upgrade"));
    let shield3 = cat.buildable(cat.buildable_id("RS_Enhanced_Shielding_L2_Upgrade").unwrap());
    assert_eq!((shield3.cost, shield3.time), (260, 35.0));
}

#[test]
fn a_purchase_pays_now_builds_in_order_and_the_credits_leave_with_the_result() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Empire_Star_Base_3", "Nebulon_B_Frigate"]));
    let mut b = defended(&cat, "Empire", "Empire_Star_Base_3", &[], 400);
    let id = |n: &str| cat.buildable_id(n).unwrap();
    let (w1, w2, d1) = (id("ES_Improved_Weapons_L1_Upgrade"), id("ES_Improved_Weapons_L2_Upgrade"), id("ES_Improved_Defenses_L1_Upgrade"));
    assert_eq!(b.buildable(1).len(), 7);
    assert!(b.buildable(0).is_empty(), "the attacker has no station");
    assert_eq!(b.command(1, Command::TacticalBuild { item: w2 }), Err(CommandError::MissingPrerequisite));
    b.command(1, Command::TacticalBuild { item: w1 }).unwrap();
    assert_eq!(b.command(1, Command::TacticalBuild { item: w1 }), Err(CommandError::BuildLimit));
    // The next level may be queued behind its prerequisite.
    b.command(1, Command::TacticalBuild { item: w2 }).unwrap();
    assert_eq!((b.credits(1), b.command(1, Command::TacticalBuild { item: d1 })), (80, Err(CommandError::NotEnoughCredits)));
    let mut done = Vec::new();
    while b.time() < 70.0 {
        let t = b.time() + b.config().dt;
        for e in b.step() {
            if let Event::BuildFinished { side, item } = e {
                done.push((t, *side, *item));
            }
        }
    }
    // 25 s for L1, then 35 s for L2; L2 replaces L1.
    assert_eq!(done.iter().map(|d| (d.1, d.2)).collect::<Vec<_>>(), [(1, w1), (1, w2)]);
    assert!((done[0].0 - 25.0).abs() < 0.05 && (done[1].0 - 60.0).abs() < 0.05, "{done:?}");
    assert_eq!(b.built(1), [w2]);
    assert_eq!(b.command(1, Command::TacticalBuild { item: w1 }), Err(CommandError::BuildLimit), "a replaced level is not bought again");
    assert_eq!(b.result().attrition_credits[1], -320);
}

#[test]
fn a_cancelled_or_orphaned_purchase_gives_the_credits_back() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Empire_Star_Base_3", "Nebulon_B_Frigate"]));
    let mut b = defended(&cat, "Empire", "Empire_Star_Base_3", &[], 1000);
    let w1 = cat.buildable_id("ES_Improved_Weapons_L1_Upgrade").unwrap();
    let r1 = cat.buildable_id("ES_Enhanced_Reactors_L1_Upgrade").unwrap();
    b.command(1, Command::TacticalBuild { item: w1 }).unwrap();
    b.command(1, Command::TacticalBuild { item: r1 }).unwrap();
    b.command(1, Command::CancelTacticalBuild { item: r1 }).unwrap();
    assert_eq!(b.credits(1), 840);
    assert_eq!(b.build_queue(1).len(), 1);
    // The star base goes: its queue goes with it, paid back. The result then carries only the base's own death credits
    // (`Destruction_Survivors`), the same as in a battle without purchases.
    let kill = |b: &mut Battle| {
        let base = b.ships.iter().position(|s| s.side == 1 && cat.ship(s.def).is_static).unwrap();
        b.ships[base].alive = false;
        b.step();
    };
    kill(&mut b);
    let mut plain = defended(&cat, "Empire", "Empire_Star_Base_3", &[], 1000);
    kill(&mut plain);
    assert_eq!((b.credits(1), b.build_queue(1).len()), (1000, 0));
    assert_eq!(b.result().attrition_credits[1], plain.result().attrition_credits[1]);
}

#[test]
fn only_the_stations_faction_may_buy_from_it() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Empire_Star_Base_2", "Nebulon_B_Frigate"]));
    // A Rebel side holding an Imperial star base (captured planet): the base's lists name the Empire only.
    let b = defended(&cat, "Rebel", "Empire_Star_Base_2", &[], 1000);
    assert!(b.buildable(1).is_empty());
    let w1 = cat.buildable_id("ES_Improved_Weapons_L1_Upgrade").unwrap();
    assert_eq!(b.build_check(1, w1), Err(CommandError::NotBuildable));
}

/// Steps `b` until `side` has finished `item` (panics after `limit` seconds).
fn build(b: &mut Battle, side: u8, item: buildables::BuildId, limit: f32) {
    b.command(side, Command::TacticalBuild { item }).unwrap();
    let end = b.time() + limit;
    while !b.built(side).contains(&item) {
        assert!(b.time() < end, "{item} not built in {limit} s");
        b.step();
    }
    // Bonuses are recomputed once a second.
    for _ in 0..30 {
        b.step();
    }
}

#[test]
fn the_space_upgrades_read_their_effects() {
    let Some((data, _)) = load() else { return };
    // Every `UpgradeObject` of `Upgrades/GC/Space/{Empire,Rebel,Underworld}`.
    let names: Vec<String> = data
        .objects
        .iter()
        .filter(|o| o.kind.eq_ignore_ascii_case("UpgradeObject"))
        .filter(|o| ["Empire", "Rebel", "Underworld"].iter().any(|f| o.source_file.eq_ignore_ascii_case(&format!("Upgrades/GC/Space/{f}/{}", o.source_file.rsplit('/').next().unwrap()))))
        .map(|o| o.name.clone())
        .collect();
    assert_eq!(names.len(), 45);
    let defs = buildables::buildable_defs(&data, &names);
    let bonus = defs.iter().filter(|d| !d.bonuses.is_empty()).count();
    let enable = defs.iter().filter(|d| !d.enables.is_empty()).count();
    let stealth = defs.iter().filter(|d| d.stealth_duration > 1.0).count();
    // 3 x 2 purchased reinforcements are Lua scripts (`Reinforcements.xml`); the Underworld `US_Reinforced_Structure_L1..L3` bonuses
    // have `Activation_Style` Ground_Automatic, so they do nothing in space.
    let none: Vec<&str> = defs.iter().filter(|d| d.bonuses.is_empty() && d.enables.is_empty() && d.stealth_duration == 1.0).map(|d| d.name.as_str()).collect();
    assert_eq!((bonus, enable, stealth), (31, 3, 2), "{none:?}");
    let def = |n: &str| defs.iter().find(|d| d.name == n).unwrap();
    let w2 = &def("ES_Improved_Weapons_L2_Upgrade").bonuses[0];
    assert_eq!((w2.damage, w2.stacking), (0.25, 3));
    assert!(w2.applies("TIE_Fighter", TargetClass::Fighter) && w2.applies("Tartan_Patrol_Cruiser", TargetClass::Corvette) && !w2.applies("Star_Destroyer", TargetClass::Capital));
    assert_eq!(def("US_Targeting_Systems_L1_Upgrade").bonuses[0].range, 0.15);
    assert_eq!(def("US_Cloaking_Generator_L2_Upgrade").stealth_duration, 1.5);
    let armor = &def("US_Magnetically_Sealed_Armor_L3_Upgrade").enables[0];
    assert_eq!((armor.ability.as_str(), armor.unit_types.clone()), ("US_Magnetically_Sealed_Armor_Bonus_L3", vec!["Starviper".to_string(), "Crusader_Gunship".into(), "Kedalbe_Battleship".into()]));
}

#[test]
fn finished_upgrades_boost_the_sides_ships_of_their_classes() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Empire_Star_Base_3", "Nebulon_B_Frigate", "TIE_Fighter_Squadron", "Broadside_Cruiser"]));
    let mut b = defended(&cat, "Empire", "Empire_Star_Base_3", &[("TIE_Fighter_Squadron", 1), ("Broadside_Cruiser", 1)], 10_000);
    let id = |n: &str| cat.buildable_id(n).unwrap();
    let of = |b: &Battle, n: &str| b.ships.iter().find(|s| s.alive && cat.ship(s.def).name.starts_with(n)).unwrap().bonus();
    b.step();
    assert_eq!(of(&b, "TIE_Fighter").speed, 1.0);
    build(&mut b, 1, id("ES_Enhanced_Reactors_L1_Upgrade"), 21.0);
    // L1: fighters, bombers and gunships only (a frigate is not).
    assert!((of(&b, "TIE_Fighter").speed - 1.25).abs() < 1e-5);
    assert_eq!(of(&b, "Broadside_Cruiser").speed, 1.0);
    build(&mut b, 1, id("ES_Enhanced_Reactors_L2_Upgrade"), 31.0);
    // L2 replaces L1 and adds corvettes and frigates; bonuses of one stacking category do not add up.
    assert!((of(&b, "TIE_Fighter").speed - 1.25).abs() < 1e-5);
    assert!((of(&b, "Broadside_Cruiser").speed - 1.25).abs() < 1e-5);
    build(&mut b, 1, id("ES_Reinforced_Armor_L1_Upgrade"), 31.0);
    assert!((of(&b, "TIE_Fighter").taken_hull - 0.8).abs() < 1e-5 && (of(&b, "TIE_Fighter").taken_shield - 0.8).abs() < 1e-5);
    // `Applicable_Unit_Types` Empire_Star_Base_3: the station's own guns.
    build(&mut b, 1, id("ES_Improved_Defenses_L1_Upgrade"), 26.0);
    assert!((of(&b, "Empire_Star_Base_3").damage - 1.15).abs() < 1e-5);
    assert_eq!(of(&b, "TIE_Fighter").damage, 1.0);
    // The enemy gets nothing.
    assert_eq!(of(&b, "Nebulon_B_Frigate"), Bonus::default());
}

#[test]
fn an_enable_ability_upgrade_switches_the_listed_types_locked_ability_on() {
    let Some((data, fs)) = load() else { return };
    // In the data the armour upgrades name Starviper, Crusader_Gunship and Kedalbe_Battleship, which have no ability of that name;
    // only Xizor_Virago has `US_Magnetically_Sealed_Armor_Bonus_L1` (disabled). The rule is checked with Xizor listed.
    let mut cat = Catalog::build(&data, Some(&fs), &["Underworld_Star_Base_1", "Star_Destroyer", "Xizor_Virago", "Starviper"]);
    let xizor = cat.ship(cat.ship_id("Xizor_Virago").unwrap());
    assert!(xizor.locked_redirects.iter().any(|(n, _)| n == "us_magnetically_sealed_armor_bonus_l1"), "{:?}", xizor.locked_redirects);
    assert!(cat.ship(cat.ship_id("Starviper").unwrap()).locked_redirects.iter().all(|(n, _)| !n.starts_with("us_magnetically")));
    let item = cat.buildable_id("US_Reinforced_Structure_L1_Upgrade").unwrap();
    cat.buildables[item as usize].bonuses.clear();
    cat.buildables[item as usize].enables = vec![buildables::EnableDef { ability: "US_Magnetically_Sealed_Armor_Bonus_L1".into(), unit_types: vec!["Xizor_Virago".into()] }];
    let cat = Arc::new(cat);
    let hull_left = |buy: bool| {
        let ship = |n: &str| cat.ship_id(n).unwrap();
        let empire = SideSetup { name: "Empire".into(), units: vec![UnitSetup { ship: ship("Star_Destroyer"), count: 1, tag: 1 }], structures: vec![], ai: false };
        let uw = SideSetup { name: "Underworld".into(), units: vec![UnitSetup { ship: ship("Xizor_Virago"), count: 1, tag: 2 }], structures: vec![ship("Underworld_Star_Base_1")], ai: false };
        let config = BattleConfig {
            side_credits: vec![0, 1000],
            fog_of_war: false,
            record_events: false,
            seed: 1,
            shield_regen_delay: 10_000.0,
            retreat_allowed: vec![false, false],
            side_mults: vec![SideMult { damage: 1.0, health: 1.0, shield: 1.0 }, SideMult { damage: 0.0, health: 80.0, shield: 80.0 }],
            ..BattleConfig::default()
        };
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![empire, uw], config });
        if buy {
            build(&mut b, 1, item, 26.0);
        }
        let isd = b.ships.iter().find(|s| s.side == 0).unwrap().id;
        let x = b.ships.iter().find(|s| s.side == 1 && cat.ship(s.def).name == "Xizor_Virago").unwrap().id;
        b.ships[isd as usize].pos = Vec3::new(0.0, 0.0, 0.0);
        b.ships[x as usize].pos = Vec3::new(600.0, 0.0, 0.0);
        b.command(0, Command::Attack { ships: vec![isd], target: x }).unwrap();
        for _ in 0..30 * 12 {
            b.step();
        }
        let s = &b.ships[x as usize];
        assert!(s.alive);
        s.hull + s.shield
    };
    let (bare, armoured) = (hull_left(false), hull_left(true));
    assert!(armoured > bare + 1.0, "blocked shots: {armoured} vs {bare}");
}

#[test]
fn a_stealth_duration_upgrade_lengthens_timed_stealth() {
    let Some((data, fs)) = load() else { return };
    // The cloaking generators (`Stealth_Duration_Multiplier` 1.25 / 1.5) are on no campaign build bar; the rule is checked with the
    // multiplier on an item the Underworld level 1 base offers.
    let mut cat = Catalog::build(&data, Some(&fs), &["Underworld_Star_Base_1", "Nebulon_B_Frigate", "Tyber_Zann_Interceptor-IV"]);
    let item = cat.buildable_id("US_Reinforced_Structure_L1_Upgrade").unwrap();
    cat.buildables[item as usize].stealth_duration = 1.5;
    let cat = Arc::new(cat);
    let tz = cat.ship(cat.ship_id("Tyber_Zann_Interceptor-IV").unwrap());
    let (k, a) = tz.abilities.iter().enumerate().find(|(_, a)| a.stealth && !a.toggle).expect("a timed stealth ability");
    assert_eq!(a.duration, 180.0);
    let active = |buy: bool| {
        let mut b = defended(&cat, "Underworld", "Underworld_Star_Base_1", &[("Tyber_Zann_Interceptor-IV", 1)], 1000);
        if buy {
            build(&mut b, 1, item, 26.0);
        }
        let s = b.ships.iter().find(|s| cat.ship(s.def).name == "Tyber_Zann_Interceptor-IV").unwrap().id;
        b.command(1, Command::UseAbility { ship: s, ability: k as u16 }).unwrap();
        b.ships[s as usize].abilities[k].active
    };
    assert_eq!(active(false), 180.0);
    assert_eq!(active(true), 270.0);
}

#[test]
fn purchased_reinforcements_arrive_from_the_items_script() {
    let Some((data, fs)) = load() else { return };
    // No structure's bar offers them in 4.0 (the Uplink Command's list is commented out), so the item is put on a level 1 base's bar.
    let mut cat = Catalog::build(&data, Some(&fs), &["Empire_Star_Base_1", "Nebulon_B_Frigate", "Empire_Purchased_Reinforcements_Light"]);
    let item = cat.buildable_id("Empire_Purchased_Reinforcements_Light").unwrap();
    let def = cat.buildable(item);
    assert_eq!((def.cost, def.time, def.script.as_deref()), (5000, 30.0, Some("Reinforcements_Purchase_Empire_Taskforce")));
    // `Reinforcements_Purchase_Empire_Taskforce.lua`: both rolls spawn `assault_list1` after Sleep(10) and Sleep(1).
    let script = def.spawn.clone().expect("a spawn script");
    let names = |list: &[ShipId]| list.iter().map(|&s| cat.ship(s).name.clone()).collect::<Vec<_>>();
    assert_eq!(script.choices.iter().map(|c| names(c)).collect::<Vec<_>>(), vec![vec!["Vigil_Corvette", "Vigil_Corvette", "Gladiator_Star_Destroyer"]; 2]);
    assert_eq!((script.delay, script.place.clone(), script.faction.as_str()), (11.0, SpawnPlace::DefenderPositions, "Empire"));
    let base = cat.ship_id("Empire_Star_Base_1").unwrap();
    cat.ships[base as usize].buildable.push(("empire".into(), item));
    let cat = Arc::new(cat);
    let mut b = defended(&cat, "Empire", "Empire_Star_Base_1", &[], 6000);
    b.command(1, Command::TacticalBuild { item }).unwrap();
    assert_eq!(b.credits(1), 1000);
    let mut came = Vec::new();
    while b.time() < 60.0 {
        let t = b.time();
        let events = b.step().to_vec();
        for e in events {
            if let Event::Spawned { side, ships, hyperspace, .. } = e {
                came.push((t, side, cat.ship(b.ships[ships[0] as usize].def).name.clone(), hyperspace));
            }
        }
    }
    assert_eq!(came.iter().map(|c| c.2.as_str()).collect::<Vec<_>>(), ["Vigil_Corvette", "Vigil_Corvette", "Gladiator_Star_Destroyer"], "{came:?}");
    assert!(came.iter().all(|c| c.1 == 1 && c.3));
    // 30 s to build, the script's 11 s, 3 s out of hyperspace.
    assert!((came[0].0 - 44.0).abs() < 0.1, "{came:?}");
    assert_eq!(b.result().attrition_credits[1], -5000);
}

#[test]
fn every_purchased_reinforcement_script_resolves_its_units() {
    let Some((data, fs)) = load() else { return };
    let items = ["Empire", "Rebel", "Underworld"].map(|f| [format!("{f}_Starbase_Special_Reinforces_Upgrade_Fighter"), format!("{f}_Purchased_Reinforcements_Light")]).concat();
    let names: Vec<&str> = items.iter().map(String::as_str).collect();
    let cat = Catalog::build(&data, Some(&fs), &names);
    for n in &items {
        let def = cat.buildable(cat.buildable_id(n).unwrap());
        let script = def.spawn.as_ref().unwrap_or_else(|| panic!("{n}: {:?}", def.script));
        let units: Vec<String> = script.choices[0].iter().map(|&s| cat.ship(s).name.clone()).collect();
        eprintln!("{n}: {} credits, {} s, {} units {units:?}", def.cost, def.time, units.len());
        assert!(!units.is_empty() && script.choices.iter().all(|c| c.len() == units.len()), "{n}: an unknown unit in {:?}", script.choices);
        assert_eq!((def.cost, def.time), if n.contains("Fighter") { (1500, 20.0) } else { (5000, 30.0) });
    }
}

/// Like [`defended`], on map `map`.
fn defended_on(cat: &Arc<Catalog>, arena: Option<Arc<Arena>>, faction: &str, base: &str) -> Battle {
    let ship = |n: &str| cat.ship_id(n).unwrap_or_else(|| panic!("{n}"));
    let attacker = SideSetup { name: "Rebel".into(), units: vec![UnitSetup { ship: ship("Nebulon_B_Frigate"), count: 1, tag: 1 }], structures: vec![], ai: false };
    let defender = SideSetup { name: faction.into(), units: vec![UnitSetup { ship: ship("Broadside_Cruiser"), count: 1, tag: 2 }], structures: vec![ship(base)], ai: false };
    let half_size = arena.as_ref().map_or(BattleConfig::default().half_size, |a| a.half_size());
    Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![attacker, defender], config: BattleConfig { arena, half_size, fog_of_war: false, ..BattleConfig::default() } })
}

#[test]
fn mercenary_debuffs_weaken_the_enemy_factions_after_the_dummys_countdown() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["E_Mercenary_Outpost", "E_Mercenary_Citadel", "Nebulon_B_Frigate", "Broadside_Cruiser"]));
    let id = |n: &str| cat.buildable_id(n).unwrap();
    let damage = cat.buildable(id("Empire_Enemies_-7%_Damage_Debuff"));
    // `Debuff_Empire_Enemies__Damage.lua` spawns `Empire_Enemies_-7%_Damage` at the `Debuff Marker`; it destroys itself after 10 s and
    // leaves one `<faction>_-7%_Damage` aura per other faction.
    assert_eq!((damage.cost, damage.time, damage.debuff_delay, damage.debuff_marker.as_deref()), (0, 0.0, 10.0, Some("Debuff Marker")));
    let factions: Vec<&str> = damage.debuffs.iter().map(|d| d.faction.as_str()).collect();
    assert_eq!(factions, ["rebel", "underworld", "cis", "republic", "sith", "csa", "warlords", "hapan", "mandalorians", "cartels"]);
    assert!(damage.spawn.is_none(), "the dummy is no battle unit");
    assert!(cat.ship_id("Empire_Enemies_-7%_Damage").is_none());
    let arena = Arena::load(&data, &fs, "_SPACE_PLANET_ALDERAAN_01.TED").map(Arc::new);
    assert!(arena.as_ref().unwrap().markers.iter().any(|m| m.name.eq_ignore_ascii_case("Debuff Marker")));
    let of = |b: &Battle, n: &str| b.ships.iter().find(|s| s.alive && cat.ship(s.def).name == n).unwrap().bonus();

    let mut b = defended_on(&cat, arena.clone(), "Empire", "E_Mercenary_Outpost");
    b.command(1, Command::TacticalBuild { item: id("Empire_Enemies_-7%_Damage_Debuff") }).unwrap();
    for _ in 0..30 * 9 {
        b.step();
    }
    assert_eq!(of(&b, "Nebulon_B_Frigate").damage, 1.0, "the dummy still counts down");
    for _ in 0..30 * 3 {
        b.step();
    }
    assert!((of(&b, "Nebulon_B_Frigate").damage - 0.93).abs() < 1e-5);
    assert_eq!(of(&b, "Broadside_Cruiser").damage, 1.0, "the owner's faction is not hit");
    assert_eq!(b.command(1, Command::TacticalBuild { item: id("Empire_Enemies_-7%_Damage_Debuff") }), Err(CommandError::BuildLimit));
    assert_eq!(b.result().attrition_credits[1], 0);

    // Without a `Debuff Marker` the script finds no place and spawns nothing.
    let mut bare = defended_on(&cat, None, "Empire", "E_Mercenary_Outpost");
    bare.command(1, Command::TacticalBuild { item: id("Empire_Enemies_-7%_Damage_Debuff") }).unwrap();
    for _ in 0..30 * 12 {
        bare.step();
    }
    assert_eq!(of(&bare, "Nebulon_B_Frigate").damage, 1.0);

    // The citadel's speed debuff is the dummy unit itself on the bar: built at the station, no marker needed.
    let speed = cat.buildable(id("Empire_Enemies_-17%_Speed"));
    assert_eq!((speed.debuff_delay, speed.debuff_marker.as_deref()), (10.0, None));
    let mut c = defended_on(&cat, None, "Empire", "E_Mercenary_Citadel");
    c.command(1, Command::TacticalBuild { item: id("Empire_Enemies_-17%_Speed") }).unwrap();
    for _ in 0..30 * 12 {
        c.step();
    }
    assert!((of(&c, "Nebulon_B_Frigate").speed - 0.83).abs() < 1e-5);
}

#[test]
fn the_build_bar_shows_the_next_level_of_each_line() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Empire_Star_Base_3", "Nebulon_B_Frigate"]));
    let mut b = defended(&cat, "Empire", "Empire_Star_Base_3", &[], 200);
    let id = |n: &str| cat.buildable_id(n).unwrap();
    let shown = |b: &Battle| names(&cat, b.build_options(1).iter().map(|o| o.item));
    // Of the base's 7 items: the four L1s (the L2s wait for their L1).
    assert_eq!(shown(&b), ["ES_Enhanced_Reactors_L1_Upgrade", "ES_Improved_Defenses_L1_Upgrade", "ES_Improved_Weapons_L1_Upgrade", "ES_Reinforced_Armor_L1_Upgrade"]);
    assert!(b.build_options(1).iter().all(|o| o.ready.is_ok() && o.progress.is_none()));
    b.command(1, Command::TacticalBuild { item: id("ES_Improved_Weapons_L1_Upgrade") }).unwrap();
    for _ in 0..30 * 5 {
        b.step();
    }
    // Queued: L1 shows its progress (5 of 25 s), L2 shows up; 40 credits are left, too few for anything.
    let opts = b.build_options(1);
    let w1 = opts.iter().find(|o| o.item == id("ES_Improved_Weapons_L1_Upgrade")).unwrap();
    assert_eq!(w1.ready, Err(CommandError::BuildLimit));
    assert!((w1.progress.unwrap() - 0.2).abs() < 0.01, "{w1:?}");
    let w2 = opts.iter().find(|o| o.item == id("ES_Improved_Weapons_L2_Upgrade")).unwrap();
    assert_eq!((w2.ready.clone(), w2.progress), (Err(CommandError::NotEnoughCredits), None));
    assert!(b.build_options(0).is_empty());
}

/// An AI defender `faction` with `base` (star base level `level`) and `credits`; the attacker idles far away.
fn ai_defended(cat: &Arc<Catalog>, base: &str, level: u32, credits: i64) -> Battle {
    let ship = |n: &str| cat.ship_id(n).unwrap_or_else(|| panic!("{n}"));
    let attacker = SideSetup { name: "Rebel".into(), units: vec![UnitSetup { ship: ship("Nebulon_B_Frigate"), count: 1, tag: 1 }], structures: vec![], ai: false };
    let defender = SideSetup { name: "Empire".into(), units: vec![], structures: vec![ship(base)], ai: true };
    let config = BattleConfig { side_credits: vec![0, credits], side_base_level: vec![0, level], fog_of_war: false, ..BattleConfig::default() };
    Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![attacker, defender], config })
}

#[test]
fn the_ai_buys_one_upgrade_at_a_time_while_it_is_rich() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Empire_Star_Base_1", "Empire_Star_Base_3", "Nebulon_B_Frigate"]));
    let run = |base: &str, level: u32, credits: i64, seconds: u32| {
        let mut b = ai_defended(&cat, base, level, credits);
        for _ in 0..30 * seconds {
            b.step();
        }
        (b.credits(1), names(&cat, b.built(1).iter().copied()), b.build_queue(1).len())
    };
    // `SelfIsRich`: above 7500 credits at level 1, above 15000 at level 3.
    assert_eq!(run("Empire_Star_Base_1", 1, 7500, 5), (7500, vec![], 0));
    let (left, _, queued) = run("Empire_Star_Base_1", 1, 7501, 2);
    assert_eq!((left, queued), (7341, 1), "one item, 160 credits");
    assert_eq!(run("Empire_Star_Base_3", 3, 15000, 5), (15000, vec![], 0));
    // One after the other, the first item in bar order each time: reactors L1 (20 s), then L2 (30 s, it replaces L1), then the next.
    let (left, built, queued) = run("Empire_Star_Base_3", 3, 15641, 60);
    assert_eq!(built, ["ES_Enhanced_Reactors_L2_Upgrade"]);
    assert_eq!((left, queued), (15161, 1), "the third purchase is building; 15161 is still above 15000");
}

#[test]
fn gunship_in_a_category_list_matches_no_corvette() {
    let Some((data, fs)) = load() else { return };
    // `Gunship` is `0x3` in `GAMEOBJECTCATEGORYTYPE.XML` (Fighter | Bomber). A unit matches it only when its mask
    // contains both bits. `ES_Improved_Weapons_L1` lists `Fighter, Bomber, Gunship`; L2 adds `Corvette`.
    let bytes = fs.read("Xml/Enum/GAMEOBJECTCATEGORYTYPE.XML").expect("category enum");
    install_category_masks(&bytes);
    assert_eq!(category_mask("Gunship"), 0x3);
    assert_eq!(category_mask("Hero"), 0x0640_0000, "masks come from the enum file");
    assert_eq!(category_mask("Fighter") | category_mask("Bomber"), 0x3);
    assert!(!category_contains(category_mask("Fighter"), category_mask("Gunship")));
    assert!(category_contains(category_mask("Fighter") | category_mask("Bomber"), category_mask("Gunship")));
    assert!(!category_contains(category_mask("Corvette"), category_mask("Gunship")));
    assert_eq!(TargetClass::from_token("Gunship"), None);
    let flak = data.objects.hardpoint("HP_Flak_Satellite").expect("flak hardpoint");
    assert!(restriction_allows(TargetClass::Fighter, category_mask("Fighter"), &flak.fire_category_restrictions), "flak still shoots fighters");
    assert!(restriction_allows(TargetClass::Bomber, category_mask("Bomber"), &flak.fire_category_restrictions), "flak still shoots bombers");
    assert!(!restriction_allows(TargetClass::Corvette, category_mask("Corvette"), &flak.fire_category_restrictions));
    assert!(!restriction_allows(TargetClass::Bomber, category_mask("Fighter") | category_mask("Bomber"), &flak.fire_category_restrictions), "both bits is a gunship");
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Empire_Star_Base_3", "Nebulon_B_Frigate", "Tartan_Patrol_Cruiser"]));
    assert_eq!(cat.ship(cat.ship_id("Tartan_Gunship").unwrap()).class, TargetClass::Corvette);
    let mut b = defended(&cat, "Empire", "Empire_Star_Base_3", &[("Tartan_Patrol_Cruiser", 1)], 10_000);
    let id = |n: &str| cat.buildable_id(n).unwrap();
    let tartan = |b: &Battle| b.ships.iter().find(|s| s.alive && cat.ship(s.def).name == "Tartan_Gunship").unwrap().bonus().damage;
    build(&mut b, 1, id("ES_Improved_Weapons_L1_Upgrade"), 26.0);
    assert_eq!(tartan(&b), 1.0, "L1 does not boost a corvette");
    build(&mut b, 1, id("ES_Improved_Weapons_L2_Upgrade"), 36.0);
    assert!((tartan(&b) - 1.25).abs() < 1e-5, "L2 does");
}
