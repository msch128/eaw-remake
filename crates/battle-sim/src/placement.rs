//! Where a side may call in reinforcements. One rule shared by the simulation (`Battle::reinforcement_zone_check`) and the tactical
//! view (which evaluates it on the displayed snapshot to colour the ghost and draw the no-spawn zones).

use crate::battle::CommandError;
use crate::defs::{Catalog, ShipDef};
use crate::math::Vec3;

/// A ship or structure as the placement rule sees it.
#[derive(Debug, Clone, Copy)]
pub struct PlacementUnit {
    /// Side.
    pub side: u8,
    /// Position in battle space.
    pub pos: Vec3,
    /// How far it sees (`Space_FOW_Reveal_Range`).
    pub sensor: f32,
    /// Radius of the no-spawn zone it casts on the other sides (0 = none; only structures cast one).
    pub no_spawn: f32,
}

impl PlacementUnit {
    /// Builds the entry for a living ship of definition `def`; `default_no_spawn` is `BattleConfig::no_spawn_radius`.
    pub fn new(def: &ShipDef, side: u8, pos: Vec3, default_no_spawn: f32) -> Self {
        let no_spawn = if !def.is_static {
            0.0
        } else if def.no_spawn_radius > 0.0 {
            def.no_spawn_radius
        } else {
            default_no_spawn
        };
        Self { side, pos, sensor: if def.sensor_range > 0.0 { def.sensor_range } else { f32::MAX }, no_spawn }
    }

    /// Same, looking the definition up in the catalog.
    pub fn of(catalog: &Catalog, def: crate::ShipId, side: u8, pos: Vec3, default_no_spawn: f32) -> Self {
        Self::new(catalog.ship(def), side, pos, default_no_spawn)
    }
}

/// The no-spawn zones `(centre, radius)` the units cast on `side`: around every enemy structure.
pub fn no_spawn_zones(side: u8, units: &[PlacementUnit]) -> Vec<(Vec3, f32)> {
    units.iter().filter(|u| u.side != side && u.no_spawn > 0.0).map(|u| (u.pos, u.no_spawn)).collect()
}

/// Whether `side` may call in reinforcements at `pos`: inside the battle area (`half` is the usable half size), outside every no-spawn zone of an
/// enemy structure and, with fog of war, inside the sensor range of one of its own ships or a revealed area (`reveals`: centre, radius).
/// Population is checked separately (`Battle::pop_in_play` against the unit cap).
pub fn placement_check(
    half: f32,
    fog: bool,
    side: u8,
    pos: Vec3,
    units: &[PlacementUnit],
    reveals: &[(Vec3, f32)],
) -> Result<(), CommandError> {
    if pos.x.abs() > half || pos.y.abs() > half {
        return Err(CommandError::OutsideBattleArea);
    }
    if no_spawn_zones(side, units).iter().any(|&(c, r)| c.dist(pos) <= r) {
        return Err(CommandError::InNoSpawnZone);
    }
    let seen = !fog || units.iter().any(|u| u.side == side && u.pos.dist(pos) <= u.sensor) || reveals.iter().any(|&(c, r)| c.dist(pos) <= r);
    if !seen {
        return Err(CommandError::NotVisible);
    }
    Ok(())
}
