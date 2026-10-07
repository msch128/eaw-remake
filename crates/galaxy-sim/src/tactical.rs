//! Tactical battle resolver: plays the battle out headless with `battle-sim` at full speed and
//! converts the survivors back into the shape the galactic layer uses.

use battle_sim::{Battle, BattleConfig, BattleSetup, NO_TAG, SideSetup, UnitSetup};

use crate::combat::{CombatResult, Member, SideInput, Source};
use crate::world::World;

/// Runs the battle between `sides`. Returns `None` if some unit has no definition in the catalog,
/// in which case the caller falls back to the statistical resolver.
pub(crate) fn run(world: &World, catalog: &std::sync::Arc<battle_sim::Catalog>, sides: &[SideInput], seed: u64, arena: Option<std::sync::Arc<battle_sim::Arena>>) -> Option<CombatResult> {
    // The side with structures defends, so it takes the second (planet side) slot.
    let has_structs = |s: &SideInput| s.members.iter().any(|m| matches!(m.source, Source::Starbase(_) | Source::Structure(_)));
    let mut order: Vec<usize> = (0..sides.len()).collect();
    if sides.len() == 2 && has_structs(&sides[0]) && !has_structs(&sides[1]) {
        order.swap(0, 1);
    }

    let mut setups = Vec::new();
    for &si in &order {
        let side = &sides[si];
        let mut units = Vec::new();
        let mut structures = Vec::new();
        for (mi, m) in side.members.iter().enumerate() {
            match m.source {
                Source::Unit(u) => {
                    let ship = catalog.ship_id(&world.units[u as usize].name)?;
                    units.push(UnitSetup { ship, count: m.count, tag: mi as u32 });
                }
                Source::Starbase(s) => structures.push(catalog.ship_id(&world.starbases[s as usize].name)?),
                Source::Structure(s) => {
                    let id = catalog.ship_id(&world.structures[s as usize].name)?;
                    structures.extend(std::iter::repeat_n(id, m.count as usize));
                }
                // Garrison squadrons are launched by the star base definition itself.
                Source::Garrison(_) => {}
            }
        }
        setups.push(SideSetup { name: world.factions[side.faction as usize].name.clone(), units, structures, ai: true });
    }

    let config = BattleConfig { seed, record_events: false, unit_cap: i32::MAX / 2, max_time: world.config.tactical_max_seconds, arena, ..BattleConfig::default() };
    let max_steps = (world.config.tactical_max_seconds / config.dt) as u64 + 1;
    let mut battle = Battle::new(BattleSetup { catalog: catalog.clone(), sides: setups, config });
    let steps = battle.run_to_end(max_steps);
    let result = battle.result();

    let survived = |bi: usize, tag: u32, ship: u32| -> u32 {
        result.groups.iter().filter(|g| g.side as usize == bi && g.tag == tag && g.ship == ship).map(|g| g.survived).sum()
    };
    let mut survivors = vec![Vec::new(); sides.len()];
    for (bi, &si) in order.iter().enumerate() {
        for (mi, m) in sides[si].members.iter().enumerate() {
            let count = match m.source {
                Source::Unit(u) => {
                    let ship = catalog.ship_id(&world.units[u as usize].name)?;
                    survived(bi, mi as u32, ship)
                }
                Source::Starbase(s) => survived(bi, NO_TAG, catalog.ship_id(&world.starbases[s as usize].name)?).min(1),
                Source::Structure(s) => survived(bi, NO_TAG, catalog.ship_id(&world.structures[s as usize].name)?),
                Source::Garrison(_) => 0,
            };
            if count > 0 {
                survivors[si].push(Member { count: count.min(m.count), ..*m });
            }
        }
    }
    let winner = result.winner.map(|bi| order[bi as usize]);
    Some(CombatResult { survivors, winner, rounds: (steps / 30) as u32 })
}
