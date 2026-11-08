//! Special abilities of single shots and charged weapons against the real Remake data. Skipped when the game is not installed.
//!
//! Before: `LUCKY_SHOT` was not in the ability list (the X-wings only name it; its data sits on the squadron's `Red_Squadron_Container`).

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use battle_sim::*;

fn load() -> Option<(alamo_data::GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some((data, fs))
}

/// `attacker` (side 0) against `target` (side 1, holding fire) `gap` units apart, no AI, no fog.
fn duel(data: &alamo_data::GameData, fs: &GameFs, attacker: &str, target: &str, gap: f32) -> Battle {
    let cat = Arc::new(Catalog::build(data, Some(fs as &dyn FileSource), &[attacker, target]));
    let unit = |n: &str| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: 1, tag: 0 };
    let sides = vec![
        SideSetup { name: "Rebel".into(), units: vec![unit(attacker)], structures: vec![], ai: false },
        SideSetup { name: "Empire".into(), units: vec![unit(target)], structures: vec![], ai: false },
    ];
    let mut b = Battle::new(BattleSetup { catalog: cat, sides, config: BattleConfig { seed: 3, fog_of_war: false, ..BattleConfig::default() } });
    let t = b.ships.iter().position(|s| s.side == 1).unwrap();
    let a = b.ships.iter().position(|s| s.side == 0).unwrap();
    b.ships[t].pos = b.ships[a].pos + Vec3::new(gap, 0.0, 0.0);
    b.ships[t].stance = Stance::HoldFire;
    b
}

fn ability(b: &Battle, ship: u32, kind: &str) -> Option<usize> {
    b.catalog().ship(b.ships[ship as usize].def).abilities.iter().position(|a| a.kind.eq_ignore_ascii_case(kind))
}

/// Hull plus shield of ship `t`.
fn left(b: &Battle, t: u32) -> f32 {
    b.ships[t as usize].hull + b.ships[t as usize].shield
}

#[test]
fn red_squadron_craft_take_the_lucky_shot_data_of_their_container() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs as &dyn FileSource), &["Red_Squadron"]);
    for n in ["Luke_XWing", "Red_Leader_XWing", "Wedge_XWing", "Biggs_XWing", "Hobbie_XWing", "Porkins_XWing"] {
        let d = cat.ship(cat.ship_id(n).unwrap_or_else(|| panic!("{n}")));
        let a = d.abilities.iter().find(|a| a.kind == "LUCKY_SHOT").unwrap_or_else(|| panic!("{n} has LUCKY_SHOT"));
        let shot = a.shot.as_ref().unwrap();
        assert_eq!((a.recharge, shot.mult, a.cause_damage_mult), (60.0, 600.0, 1.0), "{n}: Recharge_Seconds 60, CAUSE_DAMAGE_MULTIPLIER 600 on the one shot");
        assert!(a.supports_autofire, "{n}");
        let proj = shot.proj.map(|p| cat.projectiles[p as usize].name.clone());
        let expect = (n == "Red_Leader_XWing").then(|| "Proj_Ship_Laser_Heavy_R".to_string());
        assert_eq!(proj, expect, "{n}: its own Projectile_Types_Override, else its weapon");
    }
}

#[test]
fn luke_lands_one_shot_of_600_times_the_damage() {
    let Some((data, fs)) = load() else { return };
    // The same 4 s once with the ability and once without.
    let run = |use_it: bool| {
        let mut b = duel(&data, &fs, "Red_Squadron", "Nebulon_B_Frigate", 600.0);
        let luke = b.ships.iter().position(|s| s.side == 0 && b.catalog().ship(s.def).name == "Luke_XWing").expect("Luke flies in Red Squadron") as u32;
        let t = b.ships.iter().position(|s| s.side == 1).unwrap() as u32;
        b.command(0, Command::Attack { ships: vec![luke], target: t }).unwrap();
        for _ in 0..30 {
            b.step();
        }
        let k = ability(&b, luke, "LUCKY_SHOT").expect("Luke has the lucky shot");
        let before = left(&b, t);
        if use_it {
            b.command(0, Command::UseAbility { ship: luke, ability: k as u16 }).expect("used on the current target");
            assert!(b.command(0, Command::UseAbility { ship: luke, ability: k as u16 }).is_err(), "then it recharges");
        }
        for _ in 0..120 {
            b.step();
        }
        let st = b.ships[luke as usize].abilities[k];
        (before - left(&b, t), st.active, st.cooldown)
    };
    let (plain, _, _) = run(false);
    let (lucky, active, cooldown) = run(true);
    eprintln!("damage in 4 s: plain {plain}, with the lucky shot {lucky}; cooldown left {cooldown}");
    assert!(plain > 0.0, "Luke hits the frigate");
    assert_eq!(active, 0.0, "the shot was fired");
    assert!((cooldown - 56.0).abs() < 0.5, "60 s from the use: {cooldown}");
    assert!(lucky > plain + 100.0 * 12.0, "the lucky shot does hundreds of times a laser bolt (12): {lucky} vs {plain}");
}
