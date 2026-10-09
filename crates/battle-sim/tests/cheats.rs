//! Rule tests with hand-made definitions (no game data needed).

use std::sync::Arc;

use battle_sim::*;

fn laser() -> ProjDef {
    ProjDef {
        name: "laser".into(),
        category: "laser".into(),
        damage: 100.0,
        speed: 2000.0,
        max_distance: 9000.0,
        shield_damage: true,
        hull_damage: true,
        ..ProjDef::default()
    }
}

fn gun(name: &str, health: f32) -> HpDef {
    HpDef {
        name: name.into(),
        kind: HpKind::Weapon,
        health,
        destroyable: health > 0.0,
        targetable: true,
        offset: Vec3::new(10.0, 0.0, 0.0),
        forward: Vec3::new(1.0, 0.0, 0.0),
        projectile: Some(0),
        range: 6000.0,
        half_yaw: std::f32::consts::PI,
        half_pitch: std::f32::consts::FRAC_PI_2,
        recharge_min: 0.5,
        recharge_max: 0.5,
        pulses: 1,
        ..HpDef::default()
    }
}

fn ship(name: &str, hull: f32, shield: f32, hps: Vec<HpDef>) -> ShipDef {
    ShipDef {
        name: name.into(),
        class: TargetClass::Frigate,
        is_static: true,
        hull,
        shield,
        shield_regen: 0.0,
        radius: 100.0,
        hardpoints: hps,
        population: 5,
        ..ShipDef::default()
    }
}
fn setup(catalog: Catalog, a: &str, b: &str, na: u32, nb: u32, config: BattleConfig) -> BattleSetup {
    let catalog = Arc::new(catalog);
    let side = |n: &str, c: u32| SideSetup {
        name: n.into(),
        units: vec![UnitSetup { ship: catalog.ship_id(n).unwrap(), count: c, tag: 7 }],
        structures: vec![],
        ai: true,
    };
    BattleSetup { sides: vec![side(a, na), side(b, nb)], catalog, config }
}

fn cfg() -> BattleConfig {
    BattleConfig { half_size: 6000.0, ..BattleConfig::default() }
}


#[test]
fn win_now_ends_the_battle_for_the_issuing_side() {
    let cat = Catalog::from_defs(vec![ship("a", 1.0e6, 0.0, vec![]), ship("b", 1.0e6, 0.0, vec![])], vec![laser()]);
    let mut b = Battle::new(setup(cat, "a", "b", 1, 2, cfg()));
    b.step();
    assert!(!b.is_over());
    b.command(0, Command::Cheat(BattleCheat::WinNow)).unwrap();
    assert!(b.is_over());
    assert_eq!(b.result().winner, Some(0));
    assert!(b.ships.iter().filter(|s| s.side == 1).all(|s| !s.alive), "the other side is lost");
    assert!(b.cheats.used);
    assert_eq!(b.command(0, Command::Cheat(BattleCheat::WinNow)), Err(CommandError::BattleOver));
}

#[test]
fn invulnerable_ships_take_no_damage_and_the_toggle_turns_it_off() {
    let hull_after = |on: bool| {
        let cat = Catalog::from_defs(vec![ship("gunner", 1.0e6, 0.0, vec![gun("g", 0.0)]), ship("target", 5000.0, 0.0, vec![])], vec![laser()]);
        let mut b = Battle::new(setup(cat, "target", "gunner", 1, 1, cfg()));
        if on {
            b.command(0, Command::Cheat(BattleCheat::Invulnerable(true))).unwrap();
        }
        for _ in 0..300 {
            b.step();
        }
        b.ships[0].hull
    };
    assert_eq!(hull_after(true), 5000.0);
    assert!(hull_after(false) < 5000.0, "without the cheat the ship is shot");
}

#[test]
fn no_cooldowns_lets_an_ability_be_used_again_at_once() {
    let sprint = AbilityDef { kind: "SPRINT".into(), speed_mult: 2.0, recharge: 60.0, duration: 1.0, ..AbilityDef::default() };
    let mut runner = ship("runner", 1000.0, 0.0, vec![]);
    runner.is_static = false;
    runner.abilities = vec![sprint];
    let cat = Catalog::from_defs(vec![runner, ship("idle", 1000.0, 0.0, vec![])], vec![laser()]);
    let ready_after_duration = |cheat: bool| {
        let mut s = setup(cat.clone(), "runner", "idle", 1, 1, cfg());
        s.sides[0].ai = false;
        let mut b = Battle::new(s);
        if cheat {
            b.command(0, Command::Cheat(BattleCheat::NoCooldowns(true))).unwrap();
        }
        b.command(0, Command::UseAbility { ship: 0, ability: 0 }).unwrap();
        for _ in 0..40 {
            b.step();
        }
        b.command(0, Command::UseAbility { ship: 0, ability: 0 }).is_ok()
    };
    assert!(!ready_after_duration(false), "60 s recharge still running");
    assert!(ready_after_duration(true));
}

#[test]
fn no_pop_cap_allows_unlimited_reinforcements() {
    let cat = Catalog::from_defs(vec![ship("a", 1.0e6, 0.0, vec![]), ship("b", 1.0e6, 0.0, vec![])], vec![laser()]);
    let mut c = cfg();
    c.unit_cap = 12;
    let mut b = Battle::new(setup(cat, "a", "b", 1, 1, c));
    let a = b.catalog().ship_id("a").unwrap();
    assert_eq!(b.command(0, Command::Reinforce { ship: a, count: 30 }), Err(CommandError::UnitCap));
    b.command(0, Command::Cheat(BattleCheat::NoPopCap(true))).unwrap();
    assert!(b.command(0, Command::Reinforce { ship: a, count: 30 }).is_ok());
    // Only the issuing side: the other side keeps its cap.
    assert_eq!(b.command(1, Command::Reinforce { ship: a, count: 30 }), Err(CommandError::UnitCap));
}

#[test]
fn ai_paused_stops_ai_ships_from_attack_moving() {
    let moved = |paused: bool| {
        let mut mover = ship("mover", 1000.0, 0.0, vec![]);
        mover.is_static = false;
        mover.max_speed = 800.0;
        mover.accel = 2000.0;
        mover.turn_rate = 6.0;
        let cat = Catalog::from_defs(vec![ship("idle", 1.0e6, 0.0, vec![]), mover], vec![laser()]);
        let mut s = setup(cat, "idle", "mover", 1, 1, cfg());
        s.sides[0].ai = false;
        let mut b = Battle::new(s);
        let start = b.ships[1].pos;
        if paused {
            b.command(0, Command::Cheat(BattleCheat::AiPaused(true))).unwrap();
        }
        for _ in 0..600 {
            b.step();
        }
        b.ships[1].pos.dist(start)
    };
    assert!(moved(false) > 500.0, "the AI ship advances");
    assert!(moved(true) < 1.0, "a paused AI ship stays put");
}
