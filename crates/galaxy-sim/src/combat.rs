//! Statistical auto-resolve combat.
//!
//! Every ship, structure and garrison squadron becomes one combatant with the shield, hull and
//! weapon numbers derived from the hardpoint/projectile data. In fixed rounds each combatant
//! spreads its damage-per-second over random living enemies; the damage-type versus armor-type
//! matrix from `Gameconstants.xml` scales every hit, shields absorb before hull. Fighting continues
//! until at most one side has survivors or the round limit is reached (draw).

use crate::rng::Rng;
use crate::world::*;

/// What a combatant is made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A ship or squadron of a fleet.
    Unit(UnitTypeId),
    /// The planet's star base.
    Starbase(StarbaseTypeId),
    /// An orbital structure.
    Structure(StructureTypeId),
    /// A squadron spawned by a star base or structure (not persistent).
    Garrison(UnitTypeId),
}

/// A group of identical combatants of one side.
#[derive(Debug, Clone, Copy)]
pub struct Member {
    /// What they are.
    pub source: Source,
    /// Fleet they come from, if any.
    pub fleet: Option<FleetId>,
    /// How many.
    pub count: u32,
}

/// All forces of one faction in a battle.
#[derive(Debug, Clone)]
pub struct SideInput {
    /// Faction.
    pub faction: FactionId,
    /// Members.
    pub members: Vec<Member>,
}

/// Outcome of [`resolve`].
#[derive(Debug, Clone)]
pub struct CombatResult {
    /// Surviving members per side (same order as the input), with updated counts.
    pub survivors: Vec<Vec<Member>>,
    /// Index of the only side with survivors, `None` for a draw or mutual destruction.
    pub winner: Option<usize>,
    /// Rounds fought.
    pub rounds: u32,
    /// Fleets that left the battle by retreating (tactical battles only).
    pub retreated: Vec<FleetId>,
}

struct Inst {
    member: usize,
    src: Source,
    hull: f32,
    shield: f32,
    max_shield: f32,
}

impl World {
    /// Combat numbers of a source.
    pub fn stats_of(&self, src: Source) -> Option<&CombatStats> {
        match src {
            Source::Unit(u) | Source::Garrison(u) => Some(&self.units[u as usize].combat),
            Source::Starbase(s) => Some(&self.starbases[s as usize].combat),
            Source::Structure(s) => self.structures[s as usize].combat.as_ref(),
        }
    }

    /// Rough strength estimate (Lanchester square law): `sqrt(durability * dps)` summed over groups.
    pub fn strength(&self, members: impl IntoIterator<Item = (Source, u32)>) -> f32 {
        let (mut ehp, mut dps) = (0.0f32, 0.0f32);
        for (src, n) in members {
            if let Some(s) = self.stats_of(src) {
                ehp += s.durability() * n as f32;
                dps += s.raw_dps() * n as f32;
            }
        }
        (ehp * dps).sqrt()
    }
}

/// Resolves a battle between the given sides.
pub fn resolve(world: &World, sides: &[SideInput], rng: &mut Rng) -> CombatResult {
    resolve_allied(world, sides, rng, &Default::default(), &[])
}

/// Like [`resolve`], but factions in `allied` (sorted pairs) do not shoot at each other.
/// `mults` are the difficulty multipliers by faction id (empty = all neutral).
pub fn resolve_allied(world: &World, sides: &[SideInput], rng: &mut Rng, allied: &std::collections::BTreeSet<(FactionId, FactionId)>, mults: &[crate::state::Difficulty]) -> CombatResult {
    resolve_tuned(world, sides, rng, allied, mults, Tuning::of(world))
}

/// The calibration knobs of the statistical resolver (`SimConfig::combat_focus`, `combat_base_factor`; see `examples/calibrate.rs`).
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    /// Every weapon looks at this many random enemies and fires at the weakest (1 = pure random spread).
    pub focus: u32,
    /// Multiplier on hull, shield and damage of star bases, structures and their garrison squadrons.
    pub base_factor: f32,
    /// Damage efficiency of fighters and bombers, corvettes, frigates, capital and super capital ships (small craft lose time closing in and miss).
    pub efficiency: [f32; 4],
    /// Crowding: a side with more than `crowd.0` ships deals its damage scaled by `(crowd.0 / ships) ^ crowd.1` (big swarms get in each other's way; `crowd.1` 0 = off).
    pub crowd: (f32, f32),
    /// Ships beyond the faction's population cap wait in a reserve and arrive in waves as losses make room.
    pub waves: bool,
}

impl Tuning {
    /// The values of the world's config.
    pub fn of(world: &World) -> Self {
        Self { focus: world.config.combat_focus, base_factor: world.config.combat_base_factor, efficiency: world.config.combat_efficiency, crowd: world.config.combat_crowd, waves: world.config.combat_waves }
    }
}

/// [`resolve_allied`] with explicit [`Tuning`].
pub fn resolve_tuned(world: &World, sides: &[SideInput], rng: &mut Rng, allied: &std::collections::BTreeSet<(FactionId, FactionId)>, mults: &[crate::state::Difficulty], tuning: Tuning) -> CombatResult {
    let focus = tuning.focus;
    let base = |src: Source| if matches!(src, Source::Unit(_)) { 1.0 } else { tuning.base_factor };
    let crowd: Vec<f32> = sides
        .iter()
        .map(|s| {
            let n: u32 = s.members.iter().filter(|m| matches!(m.source, Source::Unit(_))).map(|m| m.count).sum();
            if tuning.crowd.1 > 0.0 && n as f32 > tuning.crowd.0 { (tuning.crowd.0 / n as f32).powf(tuning.crowd.1) } else { 1.0 }
        })
        .collect();
    let eff = |src: Source| match src {
        Source::Unit(u) | Source::Garrison(u) => match world.units[u as usize].category {
            alamo_data::ShipCategory::Fighter | alamo_data::ShipCategory::Bomber => tuning.efficiency[0],
            alamo_data::ShipCategory::Corvette | alamo_data::ShipCategory::Transport => tuning.efficiency[1],
            alamo_data::ShipCategory::Frigate => tuning.efficiency[2],
            _ => tuning.efficiency[3],
        },
        _ => 1.0,
    };
    let mult = |f: FactionId| mults.get(f as usize).copied().unwrap_or_default();
    let dt = world.config.combat_round_seconds;
    // Expand into instances.
    let mut insts: Vec<Vec<Inst>> = sides
        .iter()
        .map(|s| {
            let mut v = Vec::new();
            for (mi, m) in s.members.iter().enumerate() {
                let Some(st) = world.stats_of(m.source) else { continue };
                for _ in 0..m.count {
                    let dm = mult(s.faction);
                    v.push(Inst { member: mi, src: m.source, hull: st.hull.max(1.0) * dm.health * base(m.source), shield: st.shield * dm.shield * base(m.source), max_shield: st.shield * dm.shield * base(m.source) });
                }
            }
            v
        })
        .collect();

    // Population cap: ships beyond the faction's `Space_Tactical_Unit_Cap` wait in a reserve and are called in as losses make room (they arrive after
    // the reinforcement delay), like in the tactical battle. Ships of the reserve that are never called survive with their fleet.
    let unit_pop = |src: Source| match src {
        Source::Unit(u) => world.units[u as usize].population.max(0),
        _ => 0,
    };
    let caps: Vec<i32> = sides.iter().map(|s| world.factions[s.faction as usize].space_unit_cap).collect();
    let mut reserve: Vec<std::collections::VecDeque<Inst>> = (0..insts.len()).map(|_| Default::default()).collect();
    let mut arriving: Vec<Vec<(u32, Inst)>> = (0..insts.len()).map(|_| Vec::new()).collect();
    if tuning.waves {
        for (si, side) in insts.iter_mut().enumerate() {
            let mut used = 0;
            let all = std::mem::take(side);
            for inst in all {
                let pop = unit_pop(inst.src);
                if pop == 0 || used + pop <= caps[si] {
                    used += pop;
                    side.push(inst);
                } else {
                    reserve[si].push_back(inst);
                }
            }
        }
    }
    let delay_rounds = (5.0 / dt).ceil() as u32;

    let mut rounds = 0;
    while rounds < world.config.combat_max_rounds {
        if tuning.waves {
            for si in 0..insts.len() {
                let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut arriving[si]).into_iter().partition(|(at, _)| *at <= rounds);
                arriving[si] = later;
                insts[si].extend(now.into_iter().map(|(_, i)| i));
                let mut used: i32 = insts[si].iter().chain(arriving[si].iter().map(|(_, i)| i)).map(|i| unit_pop(i.src)).sum();
                while let Some(front) = reserve[si].front() {
                    let pop = unit_pop(front.src);
                    if used + pop > caps[si] {
                        break;
                    }
                    used += pop;
                    let inst = reserve[si].pop_front().expect("front exists");
                    arriving[si].push((rounds + delay_rounds, inst));
                }
            }
        }
        let alive: Vec<usize> = (0..insts.len()).filter(|&i| !insts[i].is_empty() || !arriving[i].is_empty() || !reserve[i].is_empty()).collect();
        if alive.len() <= 1 {
            break;
        }
        rounds += 1;
        // Damage accumulators: (shield damage, hull damage that needs shields down, direct hull damage).
        let mut acc: Vec<Vec<(f32, f32, f32)>> = insts.iter().map(|v| vec![(0.0, 0.0, 0.0); v.len()]).collect();
        let mut dealt = false;
        for &si in &alive {
            let enemies: Vec<usize> = alive.iter().copied().filter(|&o| {
                    let (a, b) = (sides[si].faction, sides[o].faction);
                    !insts[o].is_empty() && world.hostile(a, b) && !allied.contains(&(a.min(b), a.max(b)))
                }).collect();
            if enemies.is_empty() {
                continue;
            }
            let dmg_mult0 = mult(sides[si].faction).damage;
            for a in 0..insts[si].len() {
                let Some(att) = world.stats_of(insts[si][a].src) else { continue };
                let dmg_mult = dmg_mult0 * base(insts[si][a].src) * eff(insts[si][a].src) * crowd[si];
                for w in &att.weapons {
                    dealt = true;
                    let ts = enemies[rng.below(enemies.len() as u64) as usize];
                    let mut ti = rng.below(insts[ts].len() as u64) as usize;
                    if focus > 1 {
                        let left = |i: usize| insts[ts][i].hull + insts[ts][i].shield - (acc[ts][i].0 + acc[ts][i].1 + acc[ts][i].2);
                        for _ in 1..focus {
                            let c = rng.below(insts[ts].len() as u64) as usize;
                            if left(c) < left(ti) {
                                ti = c;
                            }
                        }
                    }
                    let tstats = world.stats_of(insts[ts][ti].src).expect("target has stats");
                    let sm = world.damage.multiplier(w.damage_type, tstats.shield_armor);
                    let hm = world.damage.multiplier(w.damage_type, tstats.armor);
                    let e = &mut acc[ts][ti];
                    if w.shield_dps > 0.0 {
                        e.0 += w.shield_dps * sm * dt * dmg_mult;
                    }
                    if w.hull_dps > 0.0 {
                        if w.shield_dps > 0.0 {
                            e.1 += w.hull_dps * hm * dt * dmg_mult;
                        } else {
                            e.2 += w.hull_dps * hm * dt * dmg_mult;
                        }
                    }
                }
            }
        }
        if !dealt && arriving.iter().all(|a| a.is_empty()) && reserve.iter().all(|r| r.is_empty()) {
            break;
        }
        for (si, side) in insts.iter_mut().enumerate() {
            for (i, inst) in side.iter_mut().enumerate() {
                let (sd, through, direct) = acc[si][i];
                let mut hull_dmg = direct;
                if sd > 0.0 || through > 0.0 {
                    if inst.shield > 0.0 && sd > 0.0 {
                        if sd >= inst.shield {
                            let overflow = (sd - inst.shield) / sd;
                            inst.shield = 0.0;
                            hull_dmg += through * overflow;
                        } else {
                            inst.shield -= sd;
                        }
                    } else {
                        hull_dmg += through;
                    }
                }
                inst.hull -= hull_dmg;
                if hull_dmg == 0.0 && sd == 0.0 {
                    let regen = world.stats_of(inst.src).map_or(0.0, |s| s.shield_regen);
                    inst.shield = (inst.shield + regen * dt).min(inst.max_shield);
                }
            }
            side.retain(|i| i.hull > 0.0);
        }
    }

    let survivors: Vec<Vec<Member>> = insts
        .iter()
        .enumerate()
        .map(|(si, side)| {
            let mut counts = vec![0u32; sides[si].members.len()];
            for i in side.iter().chain(arriving[si].iter().map(|(_, i)| i)).chain(reserve[si].iter()) {
                counts[i.member] += 1;
            }
            sides[si]
                .members
                .iter()
                .zip(counts)
                .filter(|(_, c)| *c > 0)
                .map(|(m, c)| Member { count: c, ..*m })
                .collect()
        })
        .collect();
    let alive: Vec<usize> = (0..survivors.len()).filter(|&i| !survivors[i].is_empty()).collect();
    let winner = (alive.len() == 1 && rounds < world.config.combat_max_rounds).then(|| alive[0]);
    CombatResult { survivors, winner, rounds, retreated: Vec::new() }
}
