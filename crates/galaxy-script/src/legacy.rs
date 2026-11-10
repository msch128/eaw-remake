//! Name tables for the saves that have none.
//!
//! The builds before the name tables wrote only the number of planets, unit types and factions of their world and a fingerprint of the planet,
//! faction, unit and structure names. The ids in such a file are indices into the catalog of the build that wrote it, and that catalog was
//! smaller than today's: besides the ships and squadrons of the data, the unit catalog holds build items the converter derives, and each kind
//! of them arrived as one block ([`Block`]). An earlier catalog is therefore today's without some of the blocks. [`tables`] tries the
//! combinations and takes the one whose count and fingerprint are those of the file. A match proves every name of the four hashed tables, so
//! the ids can then be translated like those of any other save; no match means the file cannot be read, and the error says so. Star bases
//! were never part of the fingerprint; their table has been built the same way since the first build that wrote saves.

use alamo_data::GameData;
use galaxy_sim::{NameTables, UnitRole, World};

use crate::save::{SaveHeader, fingerprint};

/// A kind of build item that the unit catalog gained at some point, after the ships and squadrons read from the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Block {
    /// Hero and ship upgrade items (`UnitRole::Upgrade`), since 2026-10-15.
    Upgrades,
    /// `Repair_Phase_<n>_<ship>` items (also `UnitRole::Upgrade`), since 2026-11-08.
    PhaseRepairs,
    /// Purchases without a ship (`UnitRole::Dummy`), since 2026-11-08.
    Dummies,
    /// Smugglers, underbosses and field agents (`UnitRole::Agent`), since 2026-11-08.
    Agents,
    /// Structure level swaps (`UnitRole::StructureSwap`), since 2026-11-08.
    StructureSwaps,
}

impl Block {
    const ALL: [Block; 5] = [Block::Upgrades, Block::PhaseRepairs, Block::Dummies, Block::Agents, Block::StructureSwaps];

    fn label(self) -> &'static str {
        match self {
            Block::Upgrades => "hero and ship upgrades",
            Block::PhaseRepairs => "phase repairs",
            Block::Dummies => "purchases without a ship",
            Block::Agents => "agents",
            Block::StructureSwaps => "structure swaps",
        }
    }

    /// The block a unit type belongs to; `None` for a ship or squadron of the data.
    fn of(name: &str, role: &UnitRole) -> Option<Block> {
        match role {
            UnitRole::Ship | UnitRole::Research(_) => None,
            // The catalog names the repair items `Repair_Phase_<n>_<ship>` after the Remake script; nothing else tells them from the upgrades.
            UnitRole::Upgrade { .. } if name.starts_with("Repair_Phase_") => Some(Block::PhaseRepairs),
            UnitRole::Upgrade { .. } => Some(Block::Upgrades),
            UnitRole::Dummy => Some(Block::Dummies),
            UnitRole::Agent => Some(Block::Agents),
            UnitRole::StructureSwap { .. } => Some(Block::StructureSwaps),
        }
    }
}

/// The ships and squadrons of the data as the builds before 2026-10-11 listed them: an object was a squadron only by its kind, so an object of
/// another kind that lists squadron members was a ship when it has a space model and absent when it has none; ships first, then squadrons.
fn early_data_units(data: &GameData) -> Vec<String> {
    let (mut ships, mut squadrons) = (Vec::new(), Vec::new());
    for def in data.objects.space_units() {
        let squadron = def.kind.eq_ignore_ascii_case("squadron");
        if squadron {
            squadrons.push(def.name);
        } else if def.space_model.is_some() {
            ships.push(def.name);
        }
    }
    ships.extend(squadrons);
    ships
}

/// Finds the unit table with `count` entries that, with the other tables of `now`, has the fingerprint `wanted`. `blocks` gives the block of
/// every unit of `now` (`None` = from the data); `early` yields the data units in the early order and is only asked when today's order does not
/// fit. Returns the table and how it differs from today's catalog.
fn search(count: usize, wanted: u64, now: &NameTables, blocks: &[Option<Block>], early: impl FnOnce() -> Vec<String>) -> Option<(Vec<String>, String)> {
    let fits = |units: Vec<String>| {
        if units.len() != count {
            return None;
        }
        let tables = NameTables { units, ..now.clone() };
        (fingerprint(&tables) == wanted).then_some(tables.units)
    };
    // Every set of blocks, the fullest catalogs first.
    let mut sets: Vec<u32> = (0..1u32 << Block::ALL.len()).collect();
    sets.sort_by_key(|set| std::cmp::Reverse(set.count_ones()));
    let has = |set: u32, block: Block| set & (1 << Block::ALL.iter().position(|b| *b == block).expect("listed")) != 0;
    let without = |set: u32| {
        let missing: Vec<&str> = Block::ALL.iter().filter(|b| !has(set, **b) && blocks.contains(&Some(**b))).map(|b| b.label()).collect();
        if missing.is_empty() { "the catalog of this build".to_string() } else { format!("the catalog of a build without {}", missing.join(", ")) }
    };
    let derived = |set: u32| now.units.iter().zip(blocks).filter(move |(_, b)| b.is_some_and(|b| has(set, b))).map(|(n, _)| n.clone());
    for &set in &sets {
        let units = now.units.iter().zip(blocks).filter(|(_, b)| b.is_none_or(|b| has(set, b))).map(|(n, _)| n.clone()).collect();
        if let Some(units) = fits(units) {
            return Some((units, without(set)));
        }
    }
    let early = early();
    for &set in &sets {
        if let Some(units) = fits(early.iter().cloned().chain(derived(set)).collect()) {
            return Some((units, format!("{}, with the squadrons as the first builds listed them", without(set))));
        }
    }
    None
}

/// The name tables a save without them was written with, and a note on which catalog that is. `now` are the tables of `world`, the world of
/// the match the save is loaded into. Fails with the reason when none of the catalogs this build can rebuild is the one of the file.
pub(crate) fn tables(header: &SaveHeader, now: &NameTables, world: &World, data: &GameData) -> Result<(NameTables, String), String> {
    let [planets, units, factions] = header.world;
    if planets != now.planets.len() || factions != now.factions.len() {
        return Err(format!(
            "the save has no name tables and was written with {planets} planets and {factions} factions; the campaign now has {} and {}",
            now.planets.len(),
            now.factions.len()
        ));
    }
    let blocks: Vec<Option<Block>> = world.units.iter().map(|u| Block::of(&u.name, &u.role)).collect();
    match search(units, header.fingerprint, now, &blocks, || early_data_units(data)) {
        Some((units, which)) => Ok((NameTables { units, ..now.clone() }, format!("no name tables in the save: its {} unit types are {which}", header.world[1]))),
        None => Err(format!(
            "the save has no name tables, and its catalog ({units} unit types, fingerprint {:016x}) is none of the earlier catalogs this build can rebuild from the game data ({} unit types now), so its ids cannot be read",
            header.fingerprint,
            now.units.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Today's catalog: three units of the data, then one item of each derived kind in the order the catalog builds them.
    fn today() -> (NameTables, Vec<Option<Block>>) {
        let units = names(&["Ship_A", "Ship_B", "Squadron_C", "Hero_Upgrade", "Repair_Phase_2_Big", "Shield_Purchase", "Smuggler", "Mine_Upgrade"]);
        let blocks = vec![None, None, None, Some(Block::Upgrades), Some(Block::PhaseRepairs), Some(Block::Dummies), Some(Block::Agents), Some(Block::StructureSwaps)];
        (NameTables { planets: names(&["P", "Q"]), factions: names(&["Rebel"]), units, structures: names(&["Mine", "Big_Mine"]), starbases: names(&["Base"]) }, blocks)
    }

    /// What a build with the unit table `units` wrote into its header.
    fn written(now: &NameTables, units: &[&str]) -> (usize, u64) {
        (units.len(), fingerprint(&NameTables { units: names(units), ..now.clone() }))
    }

    fn no_early() -> Vec<String> {
        panic!("today's order fits, the early one must not be asked for")
    }

    #[test]
    fn the_catalog_of_this_build_is_found_first() {
        let (now, blocks) = today();
        let (count, wanted) = (now.units.len(), fingerprint(&now));
        let (units, which) = search(count, wanted, &now, &blocks, no_early).expect("found");
        assert_eq!(units, now.units);
        assert_eq!(which, "the catalog of this build");
    }

    /// A build of 2026-11-08: everything but the phase repairs, which sit in the middle of today's catalog.
    #[test]
    fn a_catalog_without_the_phase_repairs_is_rebuilt() {
        let (now, blocks) = today();
        let old = ["Ship_A", "Ship_B", "Squadron_C", "Hero_Upgrade", "Shield_Purchase", "Smuggler", "Mine_Upgrade"];
        let (count, wanted) = written(&now, &old);
        let (units, which) = search(count, wanted, &now, &blocks, no_early).expect("found");
        assert_eq!(units, names(&old));
        assert_eq!(which, "the catalog of a build without phase repairs");
    }

    /// A build of October 2026: the data units and the upgrades.
    #[test]
    fn a_catalog_with_only_the_upgrades_is_rebuilt() {
        let (now, blocks) = today();
        let old = ["Ship_A", "Ship_B", "Squadron_C", "Hero_Upgrade"];
        let (count, wanted) = written(&now, &old);
        let (units, which) = search(count, wanted, &now, &blocks, no_early).expect("found");
        assert_eq!(units, names(&old));
        assert_eq!(which, "the catalog of a build without phase repairs, purchases without a ship, agents, structure swaps");
    }

    /// The first builds: no derived items, and the data units listed by the early squadron rule (here `Squadron_C` was not in the catalog
    /// and the two ships were listed the other way round).
    #[test]
    fn a_catalog_of_the_first_builds_is_rebuilt_from_the_early_unit_list() {
        let (now, blocks) = today();
        let old = ["Ship_B", "Ship_A"];
        let (count, wanted) = written(&now, &old);
        let (units, which) = search(count, wanted, &now, &blocks, || names(&old)).expect("found");
        assert_eq!(units, names(&old));
        assert_eq!(which, "the catalog of a build without hero and ship upgrades, phase repairs, purchases without a ship, agents, structure swaps, with the squadrons as the first builds listed them");
    }

    #[test]
    fn a_catalog_that_is_none_of_them_is_not_guessed() {
        let (now, blocks) = today();
        // The right count and a wrong name; a different structure table; a count nothing has.
        let (count, wanted) = written(&now, &["Ship_A", "Ship_B", "Squadron_X", "Hero_Upgrade"]);
        assert!(search(count, wanted, &now, &blocks, || names(&["Ship_A", "Ship_B"])).is_none());
        let other = NameTables { structures: names(&["Mine"]), ..now.clone() };
        assert!(search(now.units.len(), fingerprint(&other), &now, &blocks, || names(&["Ship_A", "Ship_B"])).is_none());
        assert!(search(99, fingerprint(&now), &now, &blocks, || names(&["Ship_A", "Ship_B"])).is_none());
    }

    #[test]
    fn unit_roles_fall_into_their_blocks() {
        assert_eq!(Block::of("TIE_Fighter_Squadron", &UnitRole::Ship), None);
        assert_eq!(Block::of("Ackbar_Upgrade", &UnitRole::Upgrade { from: 1, to: 2 }), Some(Block::Upgrades));
        assert_eq!(Block::of("Repair_Phase_2_Executor", &UnitRole::Upgrade { from: 2, to: 1 }), Some(Block::PhaseRepairs));
        assert_eq!(Block::of("Rebel_Shield_Upgrade", &UnitRole::Dummy), Some(Block::Dummies));
        assert_eq!(Block::of("Smuggler_Team", &UnitRole::Agent), Some(Block::Agents));
        assert_eq!(Block::of("Upgrade_Golan", &UnitRole::StructureSwap { from: 1, to: 2, downgrade: false, max_per_planet: None }), Some(Block::StructureSwaps));
    }
}
