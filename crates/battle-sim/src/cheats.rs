//! Cheat commands of the F2 menu (`Command::Cheat`): applied through the normal command path, so a recorded command list replays them.

use crate::battle::Battle;

/// One cheat of the tactical battle; it applies to the side that issues it (`AiPaused` to every AI side).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleCheat {
    /// The issuing side wins at once: every other side is lost.
    WinNow,
    /// The issuing side's ships take no damage.
    Invulnerable(bool),
    /// The issuing side's abilities have no cooldown.
    NoCooldowns(bool),
    /// The issuing side's reinforcements ignore the unit cap.
    NoPopCap(bool),
    /// AI sides stop giving orders and using abilities.
    AiPaused(bool),
}

/// Cheats in force in a battle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cheats {
    /// Sides whose ships are invulnerable.
    pub invulnerable: Vec<bool>,
    /// Sides without ability cooldowns.
    pub no_cooldowns: Vec<bool>,
    /// Sides without a unit cap.
    pub no_pop_cap: Vec<bool>,
    /// AI sides are idle.
    pub ai_paused: bool,
    /// Any cheat was used.
    pub used: bool,
}

impl Cheats {
    /// Cheats for `sides` sides, none active.
    pub fn new(sides: usize) -> Self {
        Self { invulnerable: vec![false; sides], no_cooldowns: vec![false; sides], no_pop_cap: vec![false; sides], ..Self::default() }
    }

    pub(crate) fn is_invulnerable(&self, side: u8) -> bool {
        self.invulnerable.get(side as usize).copied().unwrap_or(false)
    }

    pub(crate) fn has_no_cooldowns(&self, side: u8) -> bool {
        self.no_cooldowns.get(side as usize).copied().unwrap_or(false)
    }

    pub(crate) fn has_no_pop_cap(&self, side: u8) -> bool {
        self.no_pop_cap.get(side as usize).copied().unwrap_or(false)
    }
}

impl Battle {
    /// Applies a cheat for `side`.
    pub(crate) fn apply_cheat(&mut self, side: u8, cheat: BattleCheat) {
        let i = side as usize;
        self.cheats.used = true;
        match cheat {
            BattleCheat::WinNow => {
                for s in self.ships.iter_mut().filter(|s| s.side != side && !s.retreated) {
                    s.alive = false;
                }
                self.pending.retain(|r| r.side == side);
                self.over = true;
                self.winner = Some(side);
            }
            BattleCheat::Invulnerable(on) => self.cheats.invulnerable[i] = on,
            BattleCheat::NoCooldowns(on) => {
                self.cheats.no_cooldowns[i] = on;
                if on {
                    for s in self.ships.iter_mut().filter(|s| s.side == side) {
                        for a in &mut s.abilities {
                            a.cooldown = 0.0;
                        }
                    }
                }
            }
            BattleCheat::NoPopCap(on) => self.cheats.no_pop_cap[i] = on,
            BattleCheat::AiPaused(on) => {
                self.cheats.ai_paused = on;
                // The AI's standing orders stop (and its ships hold their ground); resuming sets them going again.
                for s in self.ships.iter_mut().filter(|s| self.sides[s.side as usize].ai) {
                    s.order = crate::battle::Order::Idle;
                    s.stance = if on { crate::battle::Stance::Defensive } else { crate::battle::Stance::Aggressive };
                }
            }
        }
    }

    /// Whether `side` is run by the AI right now (a paused AI is not).
    pub(crate) fn is_ai(&self, side: u8) -> bool {
        self.sides[side as usize].ai && !self.cheats.ai_paused
    }
}
