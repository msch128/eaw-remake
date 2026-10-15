# battle-sim status

Deterministic, rayon-parallel space-battle simulation (fixed dt 1/30). Rules and balance notes: `RULES.md`.

## What works
- `Catalog::build` from alamo-data + ALO models (hardpoints from bones, turret arcs, Model_To_Attach barrels).
- `Battle::{new, step, command, result, state_hash}`; arenas from .ted maps (`Arena`); abilities, tractor beam, missile shield,
  interdiction, ion stun, hangars/reinforcements, retreat, unit cap.
- Bench `battle_bench` (40v40 about 0.45 ms/step) and `battle_duel` (diagnostics: `delay=`, `snipe`, `range=`, env `TRACE`, `WINDOW`).
- Tests: `tests/{mechanics,real_data,features}.rs`; real-data tests skip without the game.

## Key decisions
- Speeds: XML `Max_Speed` (ships and projectiles) is per frame, scaled x30 (`Catalog::speed_scale`); all `*_Seconds` fields are real seconds.
- Shields: `Shield_Refresh_Rate` per 2 s interval, applied continuously (`shield_regen_delay` = 0); a shield drained to zero is offline for
  `Depleted_Shield_Disable_Time` (4 s). Attack-move priority sets are off by default (`attack_move_sets`).
- Results: 1v1 ISD vs MC-80 about 270 s (MC-80 wins), VSD vs Nebulon-B about 30 s, X-wing vs TIE squadrons about 2 s after contact.

## Gaps / next steps
- Fighter dogfights are very short: no evasion or aim error modelled; torpedo burn effect missing.
- Ships hold one standoff range and bring only one broadside to bear; no orbiting or facing optimisation.
- Energy and `Depleted_Shield_Regen_Cap` are not modelled; acceleration times by size class are estimates.
- The 30 Hz speed scale is an assumption; check against in-game ship speeds.

## Difficulty and carried damage (new)
`BattleConfig::side_mults` (damage dealt, hull and shield multipliers per side) and `BattleConfig::start_health` (ships start with a share of hull/shield);
`GroupResult::health` reports the survivors' hull/shield fractions of single ships (not squadrons) so galaxy-sim can keep the damage.

## Tactical AI review (see RULES.md section 9)
- New: data-based AI retreat (`Retreat_From_Space` thresholds, `BattleConfig::retreat_allowed`), focus on weak ships, generic ability use (speed / shield regen / damage reduction), separation of big ships.
- Tests `tests/tactical_ai.rs` (real data): outgunned AI retreats (and does not when forbidden), even fights are fought out, no ship stacking, real-map duels are decided and deterministic (threads included),
  defenders with a station do not flee. The ability rules have no dedicated test.
- Known gaps: strength estimate for the retreat rule is ours; retreat orders at age 120 s often come too late to save slow ships; AI never calls reinforcements; no flank/escort goals.
- Added bombing runs, fighter intercept near carriers, line-abreast advance with cohesion and a test for the ability rules (RULES.md 9b). Intercept is untested; flank/escort goals, AI reinforcements and stealth remain open.

## Retreat as in EaW (new)
`Command::Retreat` (player or AI) turns every mobile unit towards its own spawn side and starts the jump timer `BattleConfig::retreat_jump_seconds` (10 s, estimate; `Battle::retreat_jump_in(side)`,
`Event::RetreatOrdered`). At zero, units with a living engine hardpoint (none by design counts as having one) escape; the rest, launched squadrons of a carrier that does not escape and reinforcement
squadrons are lost. The old "fly out of the arena" rule is gone. Tests in `tests/mechanics.rs` (retreat_* / squadrons_*).

## Formations and idle behaviour (new)
Squadron members follow the slot `leader position + Squadron_Offsets entry (rotated with the leader)` while idle, ordered to move or attack-move (and without a target), matching the leader's speed and closing the gap at 2 speed units per unit of distance (estimate).
Fighters that only hold a place (slot, carrier escort, move goal) now stop within 50 units (estimate) instead of flying past it for ever (they used to circle: 2500 units/min idle, slot error 200). Idle capitals already held position and facing (0 units/min, 0 degrees).
`tests/formations.rs` is the navcheck: idle capital path per minute, turn, squadron spread, fighter path, closest hull approach. Open: ships still chase and orbit enemies at standoff range in combat.

## Combat movement (new)
Metric `tests/formations.rs::combat_move` (real data, AI vs AI, no retreat): share of time moving, speed and yaw rate of big ships with a live target, split into "in weapon range", "inside the standoff" and "settled" (10 s inside the standoff).
Findings: ships never chased or orbited once settled (0% moving); the visible movement was (a) braking after the approach (the old rule stopped by a hard brake at 1.5x acceleration, passing the standoff at 70-80% speed and ending 20% inside it), (b) capitals picking a fast fighter as the target to position against: they steered after it and tracked it with the broadside alignment (settled yaw rate 5-9 deg/s for Victory vs Nebulon-B), (c) turning the broadside only after stopping, (d) no hysteresis at the standoff edge.
Fix in `sim.rs::mov`: big ships position against the best non-fighter candidate (fighters and bombers are shot at but not chased; explicit attack orders excepted); arrival speed profile `sqrt(2 * 1.5 accel * 0.9 * (dist - stop))` so the ship stops at the standoff; in the last stretch (`stop + 2 braking distances + radius`) it already turns to the weapon bearing of `align_to_target`; restart distance 1.12 x standoff once stopped (1.05 while moving). Standoff is still `ShipDef::preferred_range` (range covering 80% of the damage x 0.95, from the hardpoint data).
Open: the standoff does not yet depend on the target class (guns that cannot fire at it still count); capitals do not pre-turn broadside while still far away.

## Super laser events (2026-10-15)
`Event::SuperLaserCharging { ship, target, fire_in }` (queued with the command, so it reaches the next step) and `Event::SuperLaserFired { ship, target, at }` (when the shot lands; `at` = target position at that moment). Pure view hooks: no state change. Test: `super_laser_destroys_the_chosen_ship_after_charging`.
Broadsides (checked, unchanged): `align_to_target` already turns each capital to the bearing with the most hardpoint damage (`ShipDef::bearing_dps`, from the hardpoint arcs) in the last stretch of the approach and while holding at the standoff; the missing part is still the pre-turn at long range.
