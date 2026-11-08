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

#[test]
fn blast_data_and_which_hardpoints_carry_it() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs as &dyn FileSource), &["Krayt_Class_Destroyer", "Savage_Destroyer", "Tyber_Zann_The_Merciless", "Death_Star_II"]);
    let def = |n: &str| cat.ship(cat.ship_id(n).unwrap_or_else(|| panic!("{n}")));
    let blast = |n: &str| def(n).abilities.iter().find(|a| a.kind == "BLAST").and_then(|a| a.blast.clone().map(|b| (a.recharge, b.charge, b.mult))).unwrap_or_else(|| panic!("{n} has BLAST"));
    assert_eq!(blast("Krayt_Class_Destroyer"), (90.0, 5.0, 1.0), "Aggressor_Blast: Charge_Up_Seconds 5");
    assert_eq!(blast("Tyber_Zann_The_Merciless"), (90.0, 5.0, 1.0), "Peacebringer_Blast");
    assert_eq!(blast("Savage_Destroyer"), (90.0, 1.0, 1.0), "Savage_Blast: 1 s, Damage_Multiplier 1");
    let armed = |n: &str| -> Vec<(String, String, String)> {
        let d = def(n);
        d.hardpoints.iter().filter_map(|h| Some((h.name.clone(), cat.projectiles[h.projectile? as usize].name.clone(), cat.projectiles[h.blast_projectile? as usize].name.clone()))).collect()
    };
    assert_eq!(armed("Krayt_Class_Destroyer"), [("HP_Aggressor_Mega_Plasma".to_string(), "Proj_Ship_Krayt_Megaweapon_Damage".to_string(), "Proj_Ship_Krayt_Special_Megaweapon_Damage".to_string())], "the main gun fires normally and blasts");
    assert_eq!(armed("Savage_Destroyer").len(), 5, "mass driver and 4 ion cannons");
    // The Death Star has no BLAST: its super laser hardpoint stays ability-only.
    let ds = def("Death_Star_II");
    let sl = ds.hardpoints.iter().find(|h| h.name.eq_ignore_ascii_case("HP_Death_Star_Superlaser")).expect("super laser hardpoint");
    assert!(sl.projectile.is_none() && sl.blast_projectile.is_none());
}

#[test]
fn a_charged_krayt_blast_fires_the_special_megaweapon_once() {
    let Some((data, fs)) = load() else { return };
    let mut b = duel(&data, &fs, "Krayt_Class_Destroyer", "Star_Destroyer", 2500.0);
    let (krayt, t) = (b.ships.iter().position(|s| s.side == 0).unwrap() as u32, b.ships.iter().position(|s| s.side == 1).unwrap() as u32);
    b.command(0, Command::Attack { ships: vec![krayt], target: t }).unwrap();
    let cat = b.catalog().clone();
    let (normal, special) = (cat.projectile_id("Proj_Ship_Krayt_Megaweapon_Damage").unwrap(), cat.projectile_id("Proj_Ship_Krayt_Special_Megaweapon_Damage").unwrap());
    let fired = |b: &mut Battle, steps: usize| -> (usize, usize) {
        let mut n = (0, 0);
        for _ in 0..steps {
            for e in b.step() {
                if let Event::ProjectileFired { ship, projectile, .. } = *e
                    && ship == krayt
                {
                    n.0 += (projectile == normal) as usize;
                    n.1 += (projectile == special) as usize;
                }
            }
        }
        n
    };
    // 20 s: the main gun fires on its own (every 10 s).
    let before = fired(&mut b, 600);
    assert!(before.0 >= 1 && before.1 == 0, "normal shots only: {before:?}");
    let k = ability(&b, krayt, "BLAST").unwrap();
    b.command(0, Command::UseAbility { ship: krayt, ability: k as u16 }).expect("blast");
    let charging = fired(&mut b, 4 * 30);
    assert_eq!(charging.1, 0, "nothing special during the 5 s charge");
    let after = fired(&mut b, 3 * 30);
    eprintln!("before {before:?}, charging {charging:?}, after {after:?}");
    assert_eq!(after.1, 1, "one blast shot right after the charge");
    let later = fired(&mut b, 20 * 30);
    assert_eq!(later.1, 0, "only one");
    assert!(later.0 >= 1, "then normal fire again");
}

#[test]
fn hutt_v_wings_fire_one_ion_shot_from_their_container_data() {
    // Before: ION_CANNON_SHOT was not in the ability list (its data sits on `Hutt_V-Wing_Squadron_Container`).
    let Some((data, fs)) = load() else { return };
    let mut b = duel(&data, &fs, "Hutt_V-Wing_Squadron", "Nebulon_B_Frigate", 600.0);
    let cat = b.catalog().clone();
    let vw = cat.ship(cat.ship_id("Hutt_V_Wing_Fighter").unwrap());
    let a = vw.abilities.iter().find(|a| a.kind == "ION_CANNON_SHOT").expect("the V-wing has the ion shot");
    let shot = a.shot.as_ref().unwrap();
    let ion = cat.projectile_id("Proj_Ship_Ion_Cannon_Small").unwrap();
    assert_eq!((a.recharge, shot.proj, shot.mult), (20.0, Some(ion), 1.0), "Recharge_Seconds 20, Projectile_Types_Override");
    let wings: Vec<u32> = b.ships.iter().filter(|s| s.side == 0).map(|s| s.id).collect();
    assert_eq!(wings.len(), 5);
    let t = b.ships.iter().position(|s| s.side == 1).unwrap() as u32;
    b.command(0, Command::Attack { ships: wings.clone(), target: t }).unwrap();
    for _ in 0..60 {
        b.step();
    }
    let k = ability(&b, wings[0], "ION_CANNON_SHOT").unwrap() as u16;
    b.command(0, Command::UseAbility { ship: wings[0], ability: k }).expect("on the current target");
    let mut shots: Vec<ProjId> = Vec::new();
    for _ in 0..90 {
        for e in b.step() {
            if let Event::ProjectileFired { ship, projectile, .. } = *e
                && ship == wings[0]
            {
                shots.push(projectile);
            }
        }
    }
    let ions = shots.iter().filter(|&&p| p == ion).count();
    eprintln!("V-wing shots in 3 s: {} of them ion", ions);
    assert_eq!((shots.first(), ions), (Some(&ion), 1), "the next shot is the one ion shot: {shots:?}");
}

#[test]
fn an_ai_v_wing_squadron_spends_its_ion_shots_on_a_frigate() {
    let Some((data, fs)) = load() else { return };
    let ion_shots = |ai: bool| {
        let cat = Arc::new(Catalog::build(&data, Some(&fs as &dyn FileSource), &["Hutt_V-Wing_Squadron", "Nebulon_B_Frigate"]));
        let unit = |n: &str| UnitSetup { ship: cat.ship_id(n).unwrap(), count: 1, tag: 0 };
        let sides = vec![
            SideSetup { name: "Hutts".into(), units: vec![unit("Hutt_V-Wing_Squadron")], structures: vec![], ai },
            SideSetup { name: "Empire".into(), units: vec![unit("Nebulon_B_Frigate")], structures: vec![], ai: false },
        ];
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config: BattleConfig { seed: 5, fog_of_war: false, ..BattleConfig::default() } });
        let t = b.ships.iter().position(|s| s.side == 1).unwrap();
        let near = b.ships.iter().find(|s| s.side == 0).unwrap().pos + Vec3::new(800.0, 0.0, 0.0);
        b.ships[t].pos = near;
        b.ships[t].stance = Stance::HoldFire;
        let ion = cat.projectile_id("Proj_Ship_Ion_Cannon_Small").unwrap();
        let mut n = 0;
        for _ in 0..30 * 40 {
            n += b.step().iter().filter(|e| matches!(e, Event::ProjectileFired { projectile, .. } if *projectile == ion)).count();
            // Only the frigate itself: its fighters are taken out as they launch.
            for s in b.ships.iter_mut().filter(|s| s.side == 1 && s.id as usize != t) {
                s.alive = false;
            }
        }
        n
    };
    let (player, ai) = (ion_shots(false), ion_shots(true));
    eprintln!("ion shots in 40 s: player side {player}, AI side {ai}");
    assert_eq!(player, 0, "a player side without autofire keeps them");
    assert!(ai >= 5, "each of the 5 V-wings fires at least one (recharge 20 s): {ai}");
}

#[test]
fn the_viscount_shield_flare_hits_everything_within_12500_after_8_s() {
    // Before: SHIELD_FLARE was not in the ability list.
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs as &dyn FileSource), &["Viscount_Star_Defender_P4", "Nebulon_B_Frigate"]));
    let v = cat.ship(cat.ship_id("Viscount_Star_Defender_P4").unwrap());
    let a = v.abilities.iter().find(|a| a.kind == "SHIELD_FLARE").expect("P4 has the shield flare");
    let f = a.flare.as_ref().unwrap();
    assert_eq!((a.recharge, f.charge, f.radius, f.max_per_victim), (180.0, 8.0, 12500.0, 10000.0), "Jury_Rig_Shield");
    assert_eq!(cat.projectiles[f.proj as usize].name, "Proj_Viscount_Jury_Rig_Shield_Blast");
    let unit = |n: &str, count| UnitSetup { ship: cat.ship_id(n).unwrap(), count, tag: 0 };
    let sides = vec![
        SideSetup { name: "Rebel".into(), units: vec![unit("Viscount_Star_Defender_P4", 1)], structures: vec![], ai: false },
        SideSetup { name: "Empire".into(), units: vec![unit("Nebulon_B_Frigate", 3)], structures: vec![], ai: false },
    ];
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config: BattleConfig { seed: 2, fog_of_war: false, ..BattleConfig::default() } });
    let vis = b.ships.iter().position(|s| s.side == 0 && s.carrier == NONE && cat.ship(s.def).name.starts_with("Viscount")).unwrap() as u32;
    let frigates: Vec<u32> = b.ships.iter().filter(|s| s.side == 1 && cat.ship(s.def).name == "Nebulon_B_Frigate").map(|s| s.id).collect();
    let at = b.ships[vis as usize].pos;
    for (i, &fr) in frigates.iter().enumerate() {
        b.ships[fr as usize].pos = at + Vec3::new([2000.0, 9000.0, 16000.0][i], 0.0, 0.0);
        b.ships[fr as usize].stance = Stance::HoldFire;
    }
    b.ships[vis as usize].stance = Stance::HoldFire;
    b.step();
    let health = |b: &Battle| frigates.iter().map(|&f| b.ships[f as usize].hull + b.ships[f as usize].shield).collect::<Vec<f32>>();
    let start = health(&b);
    let k = ability(&b, vis, "SHIELD_FLARE").unwrap() as u16;
    b.command(0, Command::UseAbility { ship: vis, ability: k }).expect("flare");
    for _ in 0..(7.8 * 30.0) as usize {
        b.step();
    }
    assert_eq!(health(&b), start, "nothing while it charges");
    assert!(b.ships[vis as usize].shield > 0.0);
    for _ in 0..15 {
        b.step();
    }
    let end = health(&b);
    let lost: Vec<f32> = start.iter().zip(&end).map(|(a, e)| a - e).collect();
    eprintln!("lost by the frigates at 2000 / 9000 / 16000: {lost:?}; Viscount shield {}", b.ships[vis as usize].shield);
    assert!(lost[0] > lost[1] && lost[1] > 0.0, "nearer takes more (5 dropoff tiers)");
    assert_eq!(lost[2], 0.0, "outside the 12500 radius");
    assert_eq!(b.ships[vis as usize].shield, 0.0, "the shield went into the wave");
}

#[test]
fn an_ai_viscount_flares_when_three_frigates_are_near() {
    let Some((data, fs)) = load() else { return };
    let used = |frigates: u32| {
        let cat = Arc::new(Catalog::build(&data, Some(&fs as &dyn FileSource), &["Viscount_Star_Defender_P4", "Nebulon_B_Frigate"]));
        let unit = |n: &str, count| UnitSetup { ship: cat.ship_id(n).unwrap(), count, tag: 0 };
        let sides = vec![
            SideSetup { name: "Rebel".into(), units: vec![unit("Viscount_Star_Defender_P4", 1)], structures: vec![], ai: true },
            SideSetup { name: "Empire".into(), units: vec![unit("Nebulon_B_Frigate", frigates)], structures: vec![], ai: false },
        ];
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config: BattleConfig { seed: 2, fog_of_war: false, ..BattleConfig::default() } });
        let vis = b.ships.iter().position(|s| s.side == 0 && cat.ship(s.def).name.starts_with("Viscount")).unwrap();
        let at = b.ships[vis].pos;
        for (i, s) in b.ships.iter_mut().filter(|s| s.side == 1).enumerate() {
            s.pos = at + Vec3::new(3000.0, 1500.0 * i as f32, 0.0);
            s.stance = Stance::HoldFire;
        }
        let k = b.catalog().ship(b.ships[vis].def).abilities.iter().position(|a| a.flare.is_some()).unwrap();
        (0..30 * 20).any(|_| {
            b.step();
            // Only the frigates: their fighters are taken out as they launch.
            for s in b.ships.iter_mut().filter(|s| s.side == 1 && s.carrier != NONE) {
                s.alive = false;
            }
            b.ships[vis].abilities[k].cooldown > 0.0
        })
    };
    let (two, three) = (used(2), used(3));
    eprintln!("AI flare with 2 frigates near: {two}, with 3: {three}");
    assert!(!two && three);
}

#[test]
fn the_ravager_leeches_400_shield_per_second_and_triples_its_damage() {
    // Before: LEECH_SHIELDS was not in the ability list.
    let Some((data, fs)) = load() else { return };
    let mut b = duel(&data, &fs, "Ravager", "Nebulon_B_Frigate", 1500.0);
    let cat = b.catalog().clone();
    let (rav, t) = (b.ships.iter().position(|s| s.side == 0 && cat.ship(s.def).name == "Ravager").unwrap() as u32, b.ships.iter().position(|s| s.side == 1).unwrap() as u32);
    let k = ability(&b, rav, "LEECH_SHIELDS").expect("the Ravager has it");
    let a = &cat.ship(b.ships[rav as usize].def).abilities[k];
    let l = a.leech.as_ref().unwrap();
    assert_eq!((a.recharge, a.duration, a.cause_damage_mult, l.drain, l.min_range, l.max_range), (75.0, 30.0, 3.0, 400.0, 10.0, 2000.0), "Nihilus_Eat_You_Now");
    assert_eq!(l.types, ["millennium_falcon"]);
    assert!(a.supports_autofire);
    b.ships[rav as usize].stance = Stance::HoldFire;
    b.step();
    let shield0 = b.ships[t as usize].shield;
    b.command(0, Command::UseAbility { ship: rav, ability: k as u16 }).expect("a frigate 1500 away");
    for _ in 0..5 * 30 {
        b.step();
    }
    let drained = shield0 - b.ships[t as usize].shield;
    eprintln!("frigate shield {shield0} -> {} in 5 s", b.ships[t as usize].shield);
    assert!(drained >= 1500.0_f32.min(shield0 - 1.0), "about 400 per second: {drained}");
    assert!(b.ships[rav as usize].abilities[k].active > 0.0);
    // The beam breaks when the target is out of reach, and the bonus with it.
    let far = b.ships[rav as usize].pos + Vec3::new(6000.0, 0.0, 0.0);
    b.ships[t as usize].pos = far;
    b.step();
    b.step();
    assert_eq!(b.ships[rav as usize].abilities[k].active, 0.0);
    // Too far to start.
    b.ships[rav as usize].abilities[k].cooldown = 0.0;
    assert!(b.command(0, Command::UseAbility { ship: rav, ability: k as u16 }).is_err());
}

#[test]
fn concentrate_fire_marks_the_target_for_half_again_the_damage() {
    // Before: CONCENTRATE_FIRE was an estimated self buff (delay x0.65, damage dealt and taken x1.25); the data marks the target.
    let Some((data, fs)) = load() else { return };
    let mut cat = Catalog::build(&data, Some(&fs as &dyn FileSource), &["TF-24_Requiem", "Star_Destroyer"]);
    // The Requiem's escort (`Reinforcements_TF24`) would arrive and join the fight; only the two Star Destroyers may shoot here.
    let requiem = cat.ship_id("TF-24_Requiem").unwrap();
    cat.ships[requiem as usize].spawn_script = None;
    let cat = Arc::new(cat);
    let req = cat.ship(cat.ship_id("TF-24_Requiem").unwrap());
    let a = req.abilities.iter().find(|a| a.kind == "CONCENTRATE_FIRE").expect("the Requiem has it");
    let c = a.concentrate.as_ref().expect("Requiem_Home_One_Ability");
    assert_eq!((a.recharge, a.duration, c.radius, c.take_increase, c.speed_cut), (20.0, 15.0, 3000.0, 0.5, 0.0));
    assert_eq!((a.weapon_delay_mult, a.cause_damage_mult, a.take_damage_mult), (1.0, 1.0, 1.0), "no self buff");
    // The Requiem marks a Star Destroyer that another Star Destroyer shoots; the Requiem itself holds fire.
    let run = |mark: bool, gap: f32| -> Result<f32, CommandError> {
        let unit = |n: &str| UnitSetup { ship: cat.ship_id(n).unwrap(), count: 1, tag: 0 };
        let sides = vec![
            SideSetup { name: "Empire".into(), units: vec![unit("TF-24_Requiem"), unit("Star_Destroyer")], structures: vec![], ai: false },
            SideSetup { name: "Rebel".into(), units: vec![unit("Star_Destroyer")], structures: vec![], ai: false },
        ];
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config: BattleConfig { seed: 4, fog_of_war: false, ..BattleConfig::default() } });
        let find = |b: &Battle, n: &str| b.ships.iter().position(|s| s.carrier == NONE && cat.ship(s.def).name == n).unwrap() as u32;
        let r = find(&b, "TF-24_Requiem");
        let side_sd = |b: &Battle, side: u8| b.ships.iter().position(|s| s.side == side && s.carrier == NONE && cat.ship(s.def).name == "Star_Destroyer").unwrap() as u32;
        let (gunner, sd) = (side_sd(&b, 0), side_sd(&b, 1));
        let at = b.ships[r as usize].pos;
        b.ships[sd as usize].pos = at + Vec3::new(gap, 0.0, 0.0);
        b.ships[gunner as usize].pos = at + Vec3::new(gap, 900.0, 0.0);
        b.ships[sd as usize].stance = Stance::HoldFire;
        b.ships[r as usize].stance = Stance::HoldFire;
        b.command(0, Command::Attack { ships: vec![gunner], target: sd }).unwrap();
        b.ships[r as usize].target = sd;
        // The frigate is firing before the mark.
        for _ in 0..10 * 30 {
            b.step();
        }
        if mark {
            let k = req.abilities.iter().position(|a| a.concentrate.is_some()).unwrap() as u16;
            let pos = b.ships[sd as usize].pos;
            b.command(0, Command::UseAbilityAt { ship: r, ability: k, pos })?;
            assert_eq!(b.ships[sd as usize].mod_value(ModKind::TakeDamage), 0.5);
        }
        // What the other Star Destroyer's hits do (shield and hull; the shield regenerates in between, so the health left would not show it).
        let mut lost = 0.0;
        for _ in 0..10 * 30 {
            for e in b.step() {
                if let Event::Hit { ship, attacker, damage, .. } = *e
                    && ship == sd
                    && attacker == gunner
                {
                    lost += damage;
                }
            }
        }
        for _ in 0..6 * 30 {
            b.step();
        }
        assert_eq!(b.ships[sd as usize].mod_value(ModKind::TakeDamage), 0.0, "after 15 s the mark is gone");
        Ok(lost)
    };
    let (plain, marked) = (run(false, 2500.0).unwrap(), run(true, 2500.0).unwrap());
    eprintln!("Star Destroyer hits in 10 s: {plain} plain, {marked} marked");
    assert!(plain > 0.0 && (marked / plain - 1.5).abs() < 0.2, "x1.5: {}", marked / plain);
    assert!(run(true, 5000.0).is_err(), "beyond Effective_Radius 3000");
}

#[test]
fn the_ai_concentrates_fire_on_capital_ships_only() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs as &dyn FileSource), &["TF-24_Requiem", "Star_Destroyer", "Nebulon_B_Frigate"]));
    let used = |enemy: &str| {
        let unit = |n: &str| UnitSetup { ship: cat.ship_id(n).unwrap(), count: 1, tag: 0 };
        let sides = vec![
            SideSetup { name: "Empire".into(), units: vec![unit("TF-24_Requiem")], structures: vec![], ai: true },
            SideSetup { name: "Rebel".into(), units: vec![unit(enemy)], structures: vec![], ai: false },
        ];
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config: BattleConfig { seed: 4, fog_of_war: false, ..BattleConfig::default() } });
        let r = b.ships.iter().position(|s| s.side == 0 && cat.ship(s.def).name == "TF-24_Requiem").unwrap();
        let e = b.ships.iter().position(|s| s.side == 1 && s.carrier == NONE).unwrap();
        b.ships[e].pos = b.ships[r].pos + Vec3::new(2000.0, 0.0, 0.0);
        b.ships[e].stance = Stance::HoldFire;
        let k = cat.ship(b.ships[r].def).abilities.iter().position(|a| a.concentrate.is_some()).unwrap();
        (0..30 * 30).any(|_| {
            b.step();
            for s in b.ships.iter_mut().filter(|s| s.side == 1 && s.carrier != NONE) {
                s.alive = false;
            }
            b.ships[r].abilities[k].cooldown > 0.0
        })
    };
    let (capital, frigate) = (used("Star_Destroyer"), used("Nebulon_B_Frigate"));
    eprintln!("AI concentrate fire: on a Star Destroyer {capital}, on a Nebulon-B {frigate}");
    assert!(capital && !frigate, "Default_Unit_Target_In_Range: Super or Capital targets");
}
