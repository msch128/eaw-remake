//! The rule "a battle is not created again at a planet with the same line-up within `SimConfig::battle_repeat_seconds`", checked from
//! the outside on the event stream. Tests and soak runs assert with it that the rule holds; the game uses it to say once that a battle
//! repeats and to count the rest instead of writing a line per tick.

use std::collections::BTreeMap;

use crate::state::{Event, PendingBattle};
use crate::world::{PlanetId, World};

/// What [`BattleRepeats::see`] makes of a battle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sighting {
    /// A battle the rule allows. `after_repeats` is the length of the run of repeats at the planet that ended with it (0 as a rule).
    Fresh {
        /// Repeats seen at the planet since the last fresh battle there.
        after_repeats: u32,
    },
    /// The same line-up at the same planet again inside the window: the `nth` time in a row.
    Repeat {
        /// 1 for the first repeat.
        nth: u32,
    },
}

#[derive(Debug, Clone, Copy)]
struct Last {
    time: f64,
    line_up: u64,
    repeats: u32,
}

/// Watches `Event::BattlePending` for battles that come back unchanged.
#[derive(Debug, Clone)]
pub struct BattleRepeats {
    window: f64,
    last: BTreeMap<PlanetId, Last>,
    total: u64,
}

impl BattleRepeats {
    /// A watch with the given window in game seconds.
    pub fn new(window_seconds: f32) -> Self {
        Self { window: f64::from(window_seconds), last: BTreeMap::new(), total: 0 }
    }

    /// A watch with the window of the world's rule (`SimConfig::battle_repeat_seconds`).
    pub fn for_world(world: &World) -> Self {
        Self::new(world.config.battle_repeat_seconds)
    }

    /// Records a battle created at game time `time`.
    pub fn see(&mut self, time: f64, battle: &PendingBattle) -> Sighting {
        let repeat = self.last.get(&battle.planet).filter(|l| l.line_up == battle.line_up && time - l.time < self.window).map(|l| l.repeats + 1);
        let ended = self.last.get(&battle.planet).map_or(0, |l| l.repeats);
        self.last.insert(battle.planet, Last { time, line_up: battle.line_up, repeats: repeat.unwrap_or(0) });
        match repeat {
            Some(nth) => {
                self.total += 1;
                Sighting::Repeat { nth }
            }
            None => Sighting::Fresh { after_repeats: ended },
        }
    }

    /// Records every battle created in `events` (all of game time `time`) and returns the ones that are repeats.
    pub fn check(&mut self, time: f64, events: &[Event]) -> Vec<PendingBattle> {
        let mut repeated = Vec::new();
        for e in events {
            if let Event::BattlePending(b) = e
                && matches!(self.see(time, b), Sighting::Repeat { .. })
            {
                repeated.push(b.clone());
            }
        }
        repeated
    }

    /// Repeats seen so far.
    pub fn total(&self) -> u64 {
        self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battle(id: u32, planet: PlanetId, line_up: u64) -> PendingBattle {
        PendingBattle { id, planet, fleets: vec![1], factions: vec![0, 1], defender: Some(1), line_up }
    }

    #[test]
    fn the_same_line_up_inside_the_window_is_a_repeat() {
        let mut w = BattleRepeats::new(10.0);
        assert_eq!(w.see(100.0, &battle(1, 4, 77)), Sighting::Fresh { after_repeats: 0 });
        assert_eq!(w.see(101.0, &battle(2, 4, 77)), Sighting::Repeat { nth: 1 });
        assert_eq!(w.see(102.0, &battle(3, 4, 77)), Sighting::Repeat { nth: 2 });
        assert_eq!(w.total(), 2);
        // A run of repeats is measured from the last battle, not from the first: one per tick never becomes fresh.
        assert_eq!(w.see(111.5, &battle(4, 4, 77)), Sighting::Repeat { nth: 3 });
    }

    #[test]
    fn a_changed_line_up_another_planet_or_a_later_time_is_fresh() {
        let mut w = BattleRepeats::new(10.0);
        w.see(100.0, &battle(1, 4, 77));
        assert_eq!(w.see(101.0, &battle(2, 4, 78)), Sighting::Fresh { after_repeats: 0 }, "a ship was lost or a fleet arrived");
        assert_eq!(w.see(101.0, &battle(3, 5, 78)), Sighting::Fresh { after_repeats: 0 }, "another planet");
        assert_eq!(w.see(111.0, &battle(4, 4, 78)), Sighting::Fresh { after_repeats: 0 }, "the window has passed");
        assert_eq!(w.total(), 0);
    }

    #[test]
    fn the_battle_after_a_run_reports_how_long_it_was() {
        let mut w = BattleRepeats::new(10.0);
        w.see(100.0, &battle(1, 4, 77));
        w.see(101.0, &battle(2, 4, 77));
        w.see(102.0, &battle(3, 4, 77));
        assert_eq!(w.see(103.0, &battle(4, 4, 80)), Sighting::Fresh { after_repeats: 2 });
        assert_eq!(w.see(120.0, &battle(5, 4, 80)), Sighting::Fresh { after_repeats: 0 });
    }

    #[test]
    fn check_returns_the_repeats_of_an_event_batch() {
        let mut w = BattleRepeats::new(10.0);
        let events = [Event::BattlePending(battle(1, 4, 77)), Event::WeekEnded { week: 3 }, Event::BattlePending(battle(2, 4, 77)), Event::BattlePending(battle(3, 6, 5))];
        let repeated = w.check(50.0, &events);
        assert_eq!(repeated.iter().map(|b| b.id).collect::<Vec<_>>(), [2]);
    }
}
