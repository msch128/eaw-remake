//! Scenario tests of the strategic layer on a synthetic route graph (no game data needed).

use galaxy_ai::perception::Snapshot;
use galaxy_ai::strategy::{AiTier, Phase, Strategy, StrategyConfig};

/// A line of `n` planets 0-1-2-...; `owners[i]` owns planet i. Players: 0 (us), 1 (enemy), both hostile to each other.
fn line(owners: &[Option<u32>], strength: &[(usize, u32, f64)]) -> Snapshot {
    let n = owners.len();
    let mut s = Snapshot { time: 100.0, weeks: 1.0, ..Snapshot::default() };
    s.owner = owners.to_vec();
    s.real = vec![true; n];
    s.real_count = n as u32;
    s.adjacency = (0..n).map(|i| [i.wrapping_sub(1), i + 1].into_iter().filter(|&j| j < n).map(|j| j as u32).collect()).collect();
    s.income = vec![100.0; n];
    s.max_income = 100.0;
    s.slots = vec![(2, 2); n];
    s.structures = vec![Vec::new(); n];
    s.starbase = vec![(0, 5); n];
    s.names = vec!["Us".into(), "Them".into()];
    s.owned = vec![Vec::new(); 12];
    for (i, o) in owners.iter().enumerate() {
        if let Some(o) = o {
            s.owned[*o as usize].push(i as u32);
        }
    }
    s.enemy = vec![vec![false, true], vec![true, false]];
    s.strength = vec![Vec::new(); n];
    s.power = vec![Vec::new(); n];
    for &(pl, p, v) in strength {
        s.strength[pl].push((p, v));
        s.power[pl].push((p, v));
    }
    s.static_defense = vec![0.0; n];
    s.credits = vec![0.0; 12];
    s.net_income = vec![0.0; 12];
    s.tech = vec![0; 12];
    s.max_tech = vec![5; 12];
    s.human = vec![false; 12];
    s.total_power = vec![0.0; 12];
    s
}

fn strategy(snap: &Snapshot) -> Strategy {
    let mut st = Strategy::new(StrategyConfig::default());
    st.refresh(snap);
    st
}

#[test]
fn flank_prefers_the_weak_spot_while_the_main_stack_is_away() {
    // We hold 0 with a fleet; the enemy owns 1..=5. Target 1 is weakly held in both cases.
    let owners = [Some(0), Some(1), Some(1), Some(1), Some(1), Some(1)];
    let away = line(&owners, &[(0, 0, 100.0), (1, 1, 10.0), (5, 1, 400.0)]);
    let near = line(&owners, &[(0, 0, 100.0), (1, 1, 10.0), (2, 1, 400.0)]);
    let (fa, fn_) = (strategy(&away).factor(&away, 0, 1, true), strategy(&near).factor(&near, 0, 1, true));
    assert!(fa > fn_ * 1.2, "flank {fa:.2} should beat no flank {fn_:.2}");
}

#[test]
fn risk_distance_and_value_shape_the_score() {
    let owners = [Some(0), Some(1), Some(1), Some(1), None, None];
    let snap = line(&owners, &[(0, 0, 100.0), (1, 1, 20.0)]);
    let st = strategy(&snap);
    let weak = st.factor(&snap, 0, 1, true);
    let strong = {
        let heavy = line(&owners, &[(0, 0, 100.0), (1, 1, 900.0)]);
        strategy(&heavy).factor(&heavy, 0, 1, true)
    };
    assert!(weak > strong, "a weakly held target ({weak:.2}) beats a heavily held one ({strong:.2})");
    // Distance: the neutral planet next door beats the one three hops away.
    let owners = [Some(0), None, None, None, None, None];
    let snap = line(&owners, &[(0, 0, 100.0)]);
    let st = strategy(&snap);
    assert!(st.factor(&snap, 0, 1, true) > st.factor(&snap, 0, 5, true));
}

#[test]
fn expansion_drive_fades_when_large() {
    let mut owners = vec![None; 60];
    owners[0] = Some(0);
    let snap = line(&owners, &[(0, 0, 100.0)]);
    let small = strategy(&snap).factor(&snap, 0, 1, true);
    let mut big = snap.clone();
    big.owned[0] = (0..30).collect(); // pretend we own half of the map
    let large = strategy(&big).factor(&big, 0, 1, true);
    assert!(small > large, "{small:.2} vs {large:.2}");
}

#[test]
fn honest_information_hides_distant_forces_unless_the_tier_sees_the_map() {
    let owners = [Some(0), None, None, None, None, Some(1)];
    let snap = line(&owners, &[(0, 0, 100.0), (5, 1, 800.0)]);
    let near_owners = [Some(0), None, None, Some(1), None, None];
    let near_snap = line(&near_owners, &[(0, 0, 100.0), (3, 1, 800.0)]);
    let honest = strategy(&snap);
    assert!(!honest.is_visible(0, 5) && honest.is_visible(0, 1));
    assert_eq!(honest.believed_hostile_strength(&snap, 0, 5), 0.0, "never seen: nothing believed");
    let mut cheat = Strategy::new(StrategyConfig::default());
    cheat.tiers.insert(0, AiTier { name: "Brutal".into(), full_map_knowledge: true, ..AiTier::default() });
    cheat.refresh(&snap);
    assert!(cheat.is_visible(0, 5));
    assert_eq!(cheat.believed_hostile_strength(&snap, 0, 5), 800.0);
    // Memory: once seen, the value is kept after the planet leaves sensor range.
    let mut st = Strategy::new(StrategyConfig { vision_hops: 3, ..StrategyConfig::default() });
    st.refresh(&near_snap);
    assert_eq!(st.believed_hostile_strength(&near_snap, 0, 3), 800.0);
    st.config.vision_hops = 0;
    let later = line(&near_owners, &[(0, 0, 100.0)]);
    st.refresh(&later);
    assert!(!st.is_visible(0, 3));
    assert_eq!(st.believed_hostile_strength(&later, 0, 3), 800.0, "remembered");
}

#[test]
fn outgunned_factions_consolidate_and_defend_the_valuable() {
    let owners = [Some(0), Some(0), Some(1), Some(1), None, None];
    let snap = line(&owners, &[(0, 0, 10.0), (2, 1, 1000.0)]);
    let st = strategy(&snap);
    assert_eq!(st.phase(0), Phase::Consolidate);
    assert_eq!(st.phase(1), Phase::Expand);
    // The threatened own planet next to the enemy is worth defending more than a safe one far from it.
    let threatened = st.factor(&snap, 0, 1, false);
    let safe = st.factor(&snap, 0, 0, false);
    assert!(threatened > safe, "{threatened:.2} vs {safe:.2}");
}

#[test]
fn personalities_change_the_appetite() {
    let owners = [Some(0), Some(1), Some(1), None, None, None];
    let snap = line(&owners, &[(0, 0, 200.0), (1, 1, 100.0)]);
    let mut bold = Strategy::new(StrategyConfig::default());
    bold.config.personalities.insert(0, galaxy_ai::Personality { aggression: 1.6, expansion: 1.0, caution: 0.6 });
    bold.refresh(&snap);
    let mut careful = Strategy::new(StrategyConfig::default());
    careful.config.personalities.insert(0, galaxy_ai::Personality { aggression: 0.7, expansion: 1.0, caution: 1.5 });
    careful.refresh(&snap);
    assert!(bold.factor(&snap, 0, 1, true) > careful.factor(&snap, 0, 1, true));
}

#[test]
fn cost_of_a_refresh_is_small() {
    // A 200-planet ring with fleets everywhere: 12 players.
    let n = 200;
    let owners: Vec<Option<u32>> = (0..n).map(|i| Some((i % 12) as u32)).collect();
    let mut snap = line(&owners, &[]);
    snap.names = (0..12).map(|i| format!("P{i}")).collect();
    snap.owned = vec![Vec::new(); 12];
    for (i, o) in owners.iter().enumerate() {
        snap.owned[o.unwrap() as usize].push(i as u32);
    }
    snap.enemy = vec![vec![true; 12]; 12];
    for i in 0..n {
        snap.strength[i].push(((i % 12) as u32, 100.0));
        snap.power[i].push(((i % 12) as u32, 100.0));
    }
    let mut st = Strategy::new(StrategyConfig::default());
    st.refresh(&snap);
    let t = std::time::Instant::now();
    for _ in 0..10 {
        st.refresh(&snap);
        for p in 0..12 {
            for pl in 0..n as u32 {
                let _ = st.factor(&snap, p, pl, true);
            }
        }
    }
    let per = t.elapsed().as_secs_f64() * 100.0;
    println!("refresh + all factors: {per:.2} ms");
    assert!(per < 30.0, "{per} ms");
}
