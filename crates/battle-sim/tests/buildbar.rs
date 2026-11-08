//! The tactical build bar of star bases and stations (`buildables.rs`) on the real Remake data; skipped when the game is not installed.

use std::sync::Arc;

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

/// An attacker fleet far away and a defender `faction` with `base` (and `extra` structures), `credits` for the defender.
fn defended(cat: &Arc<Catalog>, faction: &str, base: &str, extra: &[&str], credits: i64) -> Battle {
    let ship = |n: &str| cat.ship_id(n).unwrap_or_else(|| panic!("{n}"));
    let attacker = SideSetup { name: "Rebel".into(), units: vec![UnitSetup { ship: ship("Nebulon_B_Frigate"), count: 1, tag: 1 }], structures: vec![], ai: false };
    let mut structures = vec![ship(base)];
    structures.extend(extra.iter().map(|n| ship(n)));
    let defender = SideSetup { name: faction.into(), units: vec![], structures, ai: false };
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
