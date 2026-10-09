//! Cheat commands (the F2 cheat menu): ordinary journaled [`Command`]s, so saves and replays stay consistent. The first
//! cheat applied marks the game as cheated ([`Galaxy::cheated`]).

use crate::state::{CommandError, CommandOk, Galaxy};
use crate::world::{FactionId, PlanetId, World};

/// One cheat of the galactic game.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Cheat {
    /// Give the issuing faction credits.
    Credits(u32),
    /// Finish whatever is at the front of every production queue of a planet (completes on the next tick).
    FinishProduction {
        /// Planet owned by the issuing faction.
        planet: PlanetId,
    },
    /// Raise the issuing faction's tech level by one (up to the maximum).
    TechUp,
    /// Stop (or resume) the galaxy AI.
    AiPaused(bool),
}

/// Applies `cheat` for `faction`.
pub(crate) fn apply(world: &World, g: &mut Galaxy, faction: FactionId, cheat: Cheat) -> Result<CommandOk, CommandError> {
    if g.factions.get(faction as usize).is_none_or(|f| f.eliminated) {
        return Err(CommandError::Invalid);
    }
    match cheat {
        Cheat::Credits(n) => g.factions[faction as usize].credits += f64::from(n),
        Cheat::FinishProduction { planet } => {
            let ps = g.planets.get_mut(planet as usize).ok_or(CommandError::Invalid)?;
            if ps.owner != Some(faction) {
                return Err(CommandError::NotOwner);
            }
            for q in &mut ps.queues {
                if let Some(item) = q.front_mut() {
                    item.remaining = 0.0;
                }
            }
        }
        Cheat::TechUp => {
            let max = world.factions[faction as usize].max_tech;
            let f = &mut g.factions[faction as usize];
            if f.tech >= max {
                return Err(CommandError::RequirementsNotMet("maximum tech level".into()));
            }
            f.tech += 1;
        }
        Cheat::AiPaused(on) => g.ai_paused = on,
    }
    g.cheated = true;
    Ok(CommandOk::Done)
}
