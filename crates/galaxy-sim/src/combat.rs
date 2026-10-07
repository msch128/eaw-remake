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
                    v.push(Inst { member: mi, src: m.source, hull: st.hull.max(1.0) * dm.health, shield: st.shield * dm.shield, max_shield: st.shield * dm.shield });
                }
            }
            v
        })
        .collect();

    let mut rounds = 0;
    while rounds < world.config.combat_max_rounds {
        let alive: Vec<usize> = (0..insts.len()).filter(|&i| !insts[i].is_empty()).collect();
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
                    world.hostile(a, b) && !allied.contains(&(a.min(b), a.max(b)))
                }).collect();
            if enemies.is_empty() {
                continue;
            }
            let dmg_mult = mult(sides[si].faction).damage;
            for a in 0..insts[si].len() {
                let Some(att) = world.stats_of(insts[si][a].src) else { continue };
                for w in &att.weapons {
                    dealt = true;
                    let ts = enemies[rng.below(enemies.len() as u64) as usize];
                    let ti = rng.below(insts[ts].len() as u64) as usize;
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
        if !dealt {
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
            for i in side {
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
