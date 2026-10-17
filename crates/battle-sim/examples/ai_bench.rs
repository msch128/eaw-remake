//! Tactical AI bench: the AI against the AI or a scripted player on the real Remake data (the AI attacks as side 0, or defends a station
//! as side 1), with the numbers that show how well each side fights: result, losses, damage, how much of the time its big ships had an
//! enemy warship in reach without firing, where its hull hits landed, how much enemy firepower it faced, how it concentrated its fire, what
//! its weapon abilities were used on and how much its fighters and bombers fired. RULES.md 9e has the results.
//!
//! Usage: `cargo run --release -p battle-sim --example ai_bench -- [scenario filter] [seeds=N] [detail]`
//! (`TRACE=<scenario>` prints what the AI's big ships do every 10 s of seed 1, with `TRACE_CRAFT=1` its fighters and bombers).

use std::collections::HashMap;
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

type Fleet = &'static [(&'static str, u32)];

/// How side 1 is played.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Player {
    /// The AI.
    Ai,
    /// Holds and fires at what comes in range.
    Hold,
    /// Every big ship attacks the weakest enemy warship near the fleet, the fighters attack-move on it (focus fire, as a player does).
    Focus,
}

struct Scenario {
    name: &'static str,
    map: Option<&'static str>,
    ai: Fleet,
    other: Fleet,
    base: &'static [&'static str],
    player: Player,
    /// The AI defends the base (side 1) and the player attacks it (side 0, everything at the station first); else the AI attacks (side 0) and
    /// the base is the other side's.
    ai_defends: bool,
}

const EMPIRE: Fleet = &[("Star_Destroyer", 2), ("Victory_Destroyer", 2), ("Acclamator", 1), ("Broadside_Cruiser", 1), ("Tartan_Patrol_Cruiser", 2)];
const REBEL: Fleet = &[("MC-80_Star_Cruiser", 2), ("Nebulon_B_Frigate", 3), ("Assault_Frigate_Mark_II", 2), ("CR-90_Corvette", 3), ("X-Wing_Squadron", 3)];
const REBEL_BIG: Fleet = &[("MC-80_Star_Cruiser", 4), ("Nebulon_B_Frigate", 4), ("Assault_Frigate_Mark_II", 3), ("CR-90_Corvette", 4), ("X-Wing_Squadron", 4)];
const EMPIRE_SSD: Fleet = &[("Executor_Star_Dreadnought", 1), ("Star_Destroyer", 2), ("Victory_Destroyer", 2), ("TIE_Fighter_Squadron", 3)];
const EMPIRE_TIE: Fleet = &[("Star_Destroyer", 2), ("Victory_Destroyer", 2), ("TIE_Fighter_Squadron", 3)];
const KUAT_DEFENCE: Fleet = &[("MC-80_Star_Cruiser", 1), ("Nebulon_B_Frigate", 2), ("CR-90_Corvette", 2)];

const SCENARIOS: &[Scenario] = &[
    Scenario { name: "mirror-ai", map: None, ai: EMPIRE, other: EMPIRE, base: &[], player: Player::Ai, ai_defends: false },
    Scenario { name: "rebel-vs-empire-ai", map: None, ai: REBEL, other: EMPIRE_TIE, base: &[], player: Player::Ai, ai_defends: false },
    Scenario { name: "mirror-hold", map: None, ai: EMPIRE, other: EMPIRE, base: &[], player: Player::Hold, ai_defends: false },
    Scenario { name: "mirror-focus", map: None, ai: EMPIRE, other: EMPIRE, base: &[], player: Player::Focus, ai_defends: false },
    Scenario { name: "rebel-vs-empire-focus", map: None, ai: REBEL, other: EMPIRE_TIE, base: &[], player: Player::Focus, ai_defends: false },
    Scenario { name: "kuat-hold", map: Some("_SPACE_PLANET_KUAT_01.ted"), ai: EMPIRE, other: KUAT_DEFENCE, base: &["Rebel_Star_Base_2"], player: Player::Hold, ai_defends: false },
    Scenario { name: "ssd-hold", map: None, ai: REBEL_BIG, other: EMPIRE_SSD, base: &[], player: Player::Hold, ai_defends: false },
    Scenario { name: "ssd-focus", map: None, ai: REBEL_BIG, other: EMPIRE_SSD, base: &[], player: Player::Focus, ai_defends: false },
    Scenario { name: "defend-vs-isd", map: None, ai: REBEL, other: EMPIRE_TIE, base: &["Rebel_Star_Base_2"], player: Player::Focus, ai_defends: true },
    Scenario { name: "defend-vs-ssd", map: None, ai: REBEL_BIG, other: EMPIRE_SSD, base: &["Rebel_Star_Base_3"], player: Player::Focus, ai_defends: true },
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seeds: u64 = args.iter().find_map(|a| a.strip_prefix("seeds=")).and_then(|v| v.parse().ok()).unwrap_or(3);
    let detail = args.iter().any(|a| a == "detail");
    let filter = args.iter().find(|a| !a.contains('=') && *a != "detail").cloned().unwrap_or_default();
    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files");
    let (data, _) = load_galaxy(&fs);
    let names: Vec<&str> = SCENARIOS.iter().flat_map(|s| s.ai.iter().chain(s.other.iter()).map(|x| x.0).chain(s.base.iter().copied())).collect();
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    println!("columns: won, seconds, big ships lost / started, damage dealt to warships (k), by big ships to fighters (k), idle = enemy warship in reach but");
    println!("no shot for 1.5 s, none = no enemy warship in reach, on weapons = share of hardpoint damage that hit weapons, faced = enemy weapon firepower");
    println!("faced (M damage-rate x s), focus = damage share of the most damaged enemy warship per 10 s, boosts = weapon abilities used (share on corvettes and smaller),");
    println!("craft firing = share of fighter and bomber seconds with a shot in the last 3 s, on warships = their damage to warships (k)");
    for s in SCENARIOS.iter().filter(|s| s.name.contains(&filter)) {
        let arena = s.map.and_then(|m| Arena::load(&data, &fs, m)).map(Arc::new);
        let runs: Vec<[Stats; 2]> = std::thread::scope(|sc| {
            let jobs: Vec<_> = (1..=seeds)
                .map(|seed| {
                    let (cat, arena) = (&cat, arena.clone());
                    sc.spawn(move || run(cat, arena, s, seed))
                })
                .collect();
            jobs.into_iter().map(|j| j.join().expect("battle")).collect()
        });
        println!("== {} (player: {:?})", s.name, s.player);
        for (seed, r) in (1..).zip(&runs) {
            println!("  seed {seed}: AI {} | other {}", r[0].line(), r[1].line());
        }
        let mean = |side: usize| Stats::mean(&runs.iter().map(|r| r[side].clone()).collect::<Vec<_>>());
        println!("  mean:   AI {} | other {}", mean(0).line(), mean(1).line());
        if detail {
            for (k, class) in ["corvettes", "frigates", "capitals"].iter().enumerate() {
                let mut d = Detail::default();
                for r in &runs {
                    d.add(&r[0].after[k]);
                }
                if d.samples > 0.0 {
                    println!("  AI {class:9} after contact: {}", d.line());
                }
            }
        }
    }
}

/// What one side did in a battle.
#[derive(Debug, Default, Clone)]
struct Stats {
    won: f32,
    duration: f32,
    /// Big ships (frigates, capitals, corvettes; no fighters, bombers or structures) at the start and lost.
    big: f32,
    lost: f32,
    /// Damage dealt to enemy warships and structures, and by own big ships to fighters and bombers.
    dealt_big: f32,
    big_on_small: f32,
    /// Share of the big ships' seconds with an enemy warship in weapon reach without a shot in the last 1.5 s, and with none in reach.
    idle_in_reach: f32,
    none_in_reach: f32,
    /// Hardpoint damage dealt, and the part of it that hit weapons.
    hp_damage: f32,
    on_weapons: f32,
    /// Enemy firepower faced: the damage rate of the enemy's living weapon hardpoints, summed over the battle's seconds.
    faced: f32,
    /// Per 10 s window with damage on enemy warships: the share of the most damaged one, weighted by the window's damage.
    focus: f32,
    focus_weight: f32,
    /// Weapon abilities used (full salvo, rocket attack, faster reload) and those used on corvettes or smaller targets.
    boosts: f32,
    boosts_small: f32,
    /// Fighter and bomber seconds, those with a shot in the last 3 s, and the damage fighters and bombers dealt to warships.
    craft: f32,
    craft_firing: f32,
    craft_on_big: f32,
    /// After the side's first hit on an enemy warship, per class (corvette, frigate, capital or bigger), see [`Detail`].
    after: [Detail; 3],
}

/// Ship-seconds of a class, those without an enemy warship in reach, of those the seconds by order (idle, move, attack-move, attack, guard),
/// with a fighter as target, moving (above 10% of top speed), and the summed distance to the nearest enemy warship.
#[derive(Debug, Default, Clone)]
struct Detail {
    samples: f32,
    none: f32,
    orders: [f32; 5],
    on_small: f32,
    moving: f32,
    gap: f32,
}

impl Detail {
    fn line(&self) -> String {
        let n = self.none.max(1.0);
        format!(
            "{:5.0} s, {:4.1}% none in reach: idle {:3.0}% move {:3.0}% attack-move {:3.0}% attack {:3.0}% guard {:3.0}%, at fighters {:3.0}%, moving {:3.0}%, nearest warship {:5.0} away",
            self.samples,
            100.0 * self.none / self.samples.max(1.0),
            100.0 * self.orders[0] / n,
            100.0 * self.orders[1] / n,
            100.0 * self.orders[2] / n,
            100.0 * self.orders[3] / n,
            100.0 * self.orders[4] / n,
            100.0 * self.on_small / n,
            100.0 * self.moving / n,
            self.gap / n
        )
    }

    fn add(&mut self, o: &Detail) {
        self.samples += o.samples;
        self.none += o.none;
        for k in 0..5 {
            self.orders[k] += o.orders[k];
        }
        self.on_small += o.on_small;
        self.moving += o.moving;
        self.gap += o.gap;
    }
}

impl Stats {
    fn line(&self) -> String {
        format!(
            "won {:.2} t {:4.0} lost {:4.1}/{:4.1} dealt {:5.0}k big->small {:4.1}k idle {:4.1}% none {:4.1}% on weapons {:4.1}% faced {:5.1}M focus {:3.0}% boosts {:4.1} ({:3.0}% small) craft firing {:3.0}% on warships {:4.1}k",
            self.won,
            self.duration,
            self.lost,
            self.big,
            self.dealt_big / 1000.0,
            self.big_on_small / 1000.0,
            self.idle_in_reach * 100.0,
            self.none_in_reach * 100.0,
            100.0 * self.on_weapons / self.hp_damage.max(1.0),
            self.faced / 1.0e6,
            100.0 * self.focus / self.focus_weight.max(1.0),
            self.boosts,
            100.0 * self.boosts_small / self.boosts.max(1.0),
            100.0 * self.craft_firing / self.craft.max(1.0),
            self.craft_on_big / 1000.0
        )
    }

    fn mean(list: &[Stats]) -> Stats {
        let n = list.len().max(1) as f32;
        let f = |g: fn(&Stats) -> f32| list.iter().map(g).sum::<f32>() / n;
        Stats {
            won: f(|s| s.won),
            duration: f(|s| s.duration),
            big: f(|s| s.big),
            lost: f(|s| s.lost),
            dealt_big: f(|s| s.dealt_big),
            big_on_small: f(|s| s.big_on_small),
            idle_in_reach: f(|s| s.idle_in_reach),
            none_in_reach: f(|s| s.none_in_reach),
            hp_damage: f(|s| s.hp_damage),
            on_weapons: f(|s| s.on_weapons),
            faced: f(|s| s.faced),
            focus: f(|s| s.focus),
            focus_weight: f(|s| s.focus_weight),
            boosts: f(|s| s.boosts),
            boosts_small: f(|s| s.boosts_small),
            craft: f(|s| s.craft),
            craft_firing: f(|s| s.craft_firing),
            craft_on_big: f(|s| s.craft_on_big),
            after: Default::default(),
        }
    }
}

fn side(cat: &Catalog, name: &str, units: Fleet, structures: &[&str], ai: bool) -> SideSetup {
    SideSetup {
        name: name.into(),
        units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: c, tag: 1 }).collect(),
        structures: structures.iter().map(|n| cat.ship_id(n).unwrap_or_else(|| panic!("{n}"))).collect(),
        ai,
    }
}

fn run(cat: &Arc<Catalog>, arena: Option<Arc<Arena>>, s: &Scenario, seed: u64) -> [Stats; 2] {
    let other_ai = s.player == Player::Ai;
    let (ai, player) = if s.ai_defends { (1u8, 0u8) } else { (0u8, 1u8) };
    let sides = if s.ai_defends {
        vec![side(cat, "Other", s.other, &[], other_ai), side(cat, "AI", s.ai, s.base, true)]
    } else {
        vec![side(cat, "AI", s.ai, &[], true), side(cat, "Other", s.other, s.base, other_ai)]
    };
    let half = arena.as_ref().map_or(10_000.0, |a| a.half_size());
    let config = BattleConfig { seed, half_size: half, arena, retreat_allowed: vec![false, false], ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let small = |d: &ShipDef| matches!(d.class, TargetClass::Fighter | TargetClass::Bomber);
    let big = |x: &Ship| !small(cat.ship(x.def)) && !cat.ship(x.def).is_static;
    let mut st = [Stats::default(), Stats::default()];
    let start: Vec<u32> = b.ships.iter().filter(|x| big(x)).map(|x| x.id).collect();
    for &i in &start {
        st[b.ships[i as usize].side as usize].big += 1.0;
    }
    let mut last_shot: HashMap<u32, f32> = HashMap::new();
    let mut window: [HashMap<u32, f32>; 2] = [HashMap::new(), HashMap::new()];
    let mut samples = [0.0f32; 2];
    let mut contact: [Option<f32>; 2] = [None, None];
    let mut focus = NONE;
    let trace_this = std::env::var("TRACE").is_ok_and(|v| v == s.name) && seed == 1;
    while !b.is_over() && b.time() < 1500.0 {
        let t = b.time();
        if s.player == Player::Focus && b.tick().is_multiple_of(30) {
            focus = player_focus(&mut b, cat, focus, player);
        }
        if trace_this && b.tick().is_multiple_of(300) {
            if std::env::var("TRACE_CRAFT").is_ok() {
                trace_craft(&b, cat, ai);
            } else {
                trace(&b, cat, ai);
            }
        }
        let events = b.step().to_vec();
        for e in &events {
            match *e {
                Event::ProjectileFired { ship, .. } => {
                    last_shot.insert(ship, t);
                }
                Event::Hit { ship, attacker, damage, kind, .. } => {
                    let (a, v) = (&b.ships[attacker as usize], &b.ships[ship as usize]);
                    let k = a.side as usize;
                    let vdef = cat.ship(v.def);
                    if !small(vdef) {
                        st[k].dealt_big += damage;
                        if small(cat.ship(a.def)) {
                            st[k].craft_on_big += damage;
                        }
                        *window[k].entry(ship).or_insert(0.0) += damage;
                        contact[k].get_or_insert(t);
                    } else if big(a) {
                        st[k].big_on_small += damage;
                    }
                    if let HitKind::Hardpoint(h) = kind {
                        st[k].hp_damage += damage;
                        if vdef.hardpoints.get(h as usize).is_some_and(|h| h.kind == HpKind::Weapon) {
                            st[k].on_weapons += damage;
                        }
                    }
                }
                Event::AbilityActivated { ship, ability } => {
                    let x = &b.ships[ship as usize];
                    let a = &cat.ship(x.def).abilities[ability as usize];
                    if a.salvo || a.rocket || a.weapon_delay_mult < 0.99 {
                        let k = x.side as usize;
                        st[k].boosts += 1.0;
                        if x.target != NONE && (cat.ship(b.ships[x.target as usize].def).class as u8) < TargetClass::Frigate as u8 {
                            st[k].boosts_small += 1.0;
                        }
                    }
                }
                _ => {}
            }
        }
        if b.tick().is_multiple_of(300) {
            for k in 0..2 {
                let sum: f32 = window[k].values().sum();
                if sum > 0.0 {
                    st[k].focus += window[k].values().copied().fold(0.0, f32::max);
                    st[k].focus_weight += sum;
                }
                window[k].clear();
            }
        }
        if b.tick().is_multiple_of(30) {
            for x in b.ships.iter().filter(|x| x.alive && !x.retreated) {
                let d = cat.ship(x.def);
                let fp: f32 = d.hardpoints.iter().zip(&x.hps).filter(|(h, st)| h.kind == HpKind::Weapon && st.alive).map(|(h, _)| h.dps).sum();
                st[1 - x.side as usize].faced += fp;
            }
            for x in b.ships.iter().filter(|x| x.alive && !x.retreated && small(cat.ship(x.def))) {
                let k = x.side as usize;
                st[k].craft += 1.0;
                if last_shot.get(&x.id).is_some_and(|&l| t - l <= 3.0) {
                    st[k].craft_firing += 1.0;
                }
            }
            for x in b.ships.iter().filter(|x| x.alive && !x.retreated && big(x)) {
                let d = cat.ship(x.def);
                let gap = b
                    .ships
                    .iter()
                    .filter(|o| o.alive && !o.retreated && o.side != x.side && !small(cat.ship(o.def)) && b.is_visible_to(x.side, o.id))
                    .map(|o| o.pos.dist(x.pos) - cat.ship(o.def).radius * 0.75)
                    .fold(f32::MAX, f32::min);
                let reach = gap <= d.max_weapon_range;
                let k = x.side as usize;
                samples[k] += 1.0;
                if !reach {
                    st[k].none_in_reach += 1.0;
                } else if last_shot.get(&x.id).is_none_or(|&l| t - l > 1.5) {
                    st[k].idle_in_reach += 1.0;
                }
                if contact[k].is_some() {
                    let class = match d.class {
                        TargetClass::Corvette | TargetClass::Transport => 0,
                        TargetClass::Frigate => 1,
                        _ => 2,
                    };
                    let a = &mut st[k].after[class];
                    a.samples += 1.0;
                    if !reach {
                        a.none += 1.0;
                        a.orders[match x.order {
                            Order::Idle => 0,
                            Order::Move(_) | Order::Retreat => 1,
                            Order::AttackMove(_) => 2,
                            Order::Attack(_) => 3,
                            Order::Guard(_) => 4,
                        }] += 1.0;
                        if x.target != NONE && small(cat.ship(b.ships[x.target as usize].def)) {
                            a.on_small += 1.0;
                        }
                        if x.speed > 0.1 * d.max_speed {
                            a.moving += 1.0;
                        }
                        a.gap += gap.min(30_000.0);
                    }
                }
            }
        }
    }
    for k in 0..2 {
        st[k].idle_in_reach /= samples[k].max(1.0);
        st[k].none_in_reach /= samples[k].max(1.0);
        st[k].duration = b.time();
        st[k].won = f32::from(u8::from(b.winner() == Some(k as u8)));
    }
    for &i in &start {
        if !b.ships[i as usize].alive {
            st[b.ships[i as usize].side as usize].lost += 1.0;
        }
    }
    let [a, o] = st;
    if ai == 0 { [a, o] } else { [o, a] }
}

/// The scripted player (`me`): while the enemy has a structure, everything attacks it; then its big ships attack the weakest visible enemy
/// warship within 5000 units of the fleet's centre (else the nearest) and the fighters attack-move on it. Returns the target.
fn player_focus(b: &mut Battle, cat: &Catalog, old: u32, me: u8) -> u32 {
    const ADVANCE: u32 = u32::MAX - 1;
    let small = |x: &Ship| matches!(cat.ship(x.def).class, TargetClass::Fighter | TargetClass::Bomber);
    let enemy = 1 - me;
    let all: Vec<u32> = b.ships.iter().filter(|x| x.alive && !x.retreated && x.side == me && !cat.ship(x.def).is_static).map(|x| x.id).collect();
    let mine: Vec<u32> = all.iter().copied().filter(|&i| !small(&b.ships[i as usize])).collect();
    if mine.is_empty() {
        return NONE;
    }
    if let Some(station) = b.ships.iter().find(|x| x.alive && x.side == enemy && cat.ship(x.def).is_static).map(|x| x.id) {
        if station != old {
            let _ = b.command(me, Command::Attack { ships: all, target: station });
        }
        return station;
    }
    let centre = mine.iter().fold(Vec3::ZERO, |a, &i| a + b.ships[i as usize].pos) * (1.0 / mine.len() as f32);
    let enemies: Vec<&Ship> = b.ships.iter().filter(|x| x.alive && !x.retreated && x.side == enemy && !small(x) && b.is_visible_to(me, x.id)).collect();
    let health = |x: &Ship| x.hull + x.shield;
    let near = enemies.iter().filter(|x| x.pos.dist(centre) < 5000.0).min_by(|a, c| health(a).total_cmp(&health(c)).then(a.id.cmp(&c.id)));
    let target = near.or_else(|| enemies.iter().min_by(|a, c| a.pos.dist(centre).total_cmp(&c.pos.dist(centre)))).map(|x| x.id);
    let Some(target) = target else {
        // Nothing in sight: advance on the enemy's side.
        if old != ADVANCE {
            let to = b.ships.iter().filter(|x| x.side == enemy).map(|x| x.pos).next().unwrap_or(Vec3::ZERO);
            let _ = b.command(me, Command::AttackMove { ships: all, to });
        }
        return ADVANCE;
    };
    if target != old {
        let _ = b.command(me, Command::Attack { ships: mine, target });
    }
    if target != old || b.tick().is_multiple_of(150) {
        let fighters: Vec<u32> = all.iter().copied().filter(|&i| small(&b.ships[i as usize])).collect();
        let to = b.ships[target as usize].pos;
        let _ = b.command(me, Command::AttackMove { ships: fighters, to });
    }
    target
}

/// Prints what each of the AI side's big ships does (diagnostics, `TRACE`).
fn trace(b: &Battle, cat: &Catalog, ai: u8) {
    let small = |d: &ShipDef| matches!(d.class, TargetClass::Fighter | TargetClass::Bomber);
    let enemies: Vec<&Ship> = b.ships.iter().filter(|o| o.alive && !o.retreated && o.side != ai && !small(cat.ship(o.def))).collect();
    println!("t={:.0}: enemy warships {} (visible to the AI: {})", b.time(), enemies.len(), enemies.iter().filter(|o| b.is_visible_to(ai, o.id)).count());
    for x in b.ships.iter().filter(|x| x.alive && !x.retreated && x.side == ai && !small(cat.ship(x.def)) && !cat.ship(x.def).is_static) {
        let d = cat.ship(x.def);
        let near = enemies.iter().map(|o| (o.pos.dist(x.pos), o.id, b.is_visible_to(ai, o.id))).min_by(|a, c| a.0.total_cmp(&c.0));
        let tgt = if x.target == NONE { "-".to_string() } else { format!("{}#{}", cat.ship(b.ships[x.target as usize].def).name, x.target) };
        println!(
            "   {}#{} at ({:.0},{:.0}) v {:.0}/{:.0} {:?} target {} nearest {:?} range {:.0}",
            d.name,
            x.id,
            x.pos.x,
            x.pos.y,
            x.speed,
            d.max_speed,
            x.order,
            tgt,
            near.map(|n| (n.0 as i32, n.1, n.2)),
            d.max_weapon_range
        );
    }
}

/// Prints the AI side's fighters and bombers (diagnostics, `TRACE_CRAFT`).
fn trace_craft(b: &Battle, cat: &Catalog, ai: u8) {
    let small = |d: &ShipDef| matches!(d.class, TargetClass::Fighter | TargetClass::Bomber);
    let craft: Vec<&Ship> = b.ships.iter().filter(|x| x.alive && !x.retreated && x.side == ai && small(cat.ship(x.def))).collect();
    println!("t={:.0}: AI craft {}", b.time(), craft.len());
    for x in craft.iter().take(12) {
        let d = cat.ship(x.def);
        let tgt = if x.target == NONE {
            "-".to_string()
        } else {
            let t = &b.ships[x.target as usize];
            format!("{}#{} at {:.0}", cat.ship(t.def).name, x.target, t.pos.dist(x.pos))
        };
        let carrier = if x.carrier == NONE { "-".to_string() } else { format!("{:.0} from carrier", b.ships[x.carrier as usize].pos.dist(x.pos)) };
        println!("   {}#{} {:?} target {} {} candidates {}", d.name, x.id, x.order, tgt, carrier, x.candidates.len());
    }
}
