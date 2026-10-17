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
Squadron members follow the slot `leader position + (own place - leader's place), rotated with the leader` while idle, ordered to move or attack-move (and without a target), matching the leader's speed and closing the gap at 2 speed units per unit of distance (estimate). The places are `ShipDef::spread` (fighter and bomber squadrons: the data offsets widened to a 100 u gap, see RULES.md), the leader is the member nearest the middle of the formation; a leader in formation flight slows down while a member lags, launched squadrons open out of the hangar formation during the flyout, and a fighter at rest keeps its heading (members turn with their leader).
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
## Hull overlaps (2026-10-16)
Ships kept apart only by the class-clamped collision circle, so long hulls were entered (Executor: 9000 long, radius 700; MC-80: radius 150 for a 540 x 450 hull) and pairs sat inside each other for minutes. `ShipDef::hull_shape` (plan-view capsule from the model meshes, `nav::Hull`) now joins the old circle in the separation and the fighter keep-out; the push-out runs at up to 2 x top speed. Measurements and estimates: RULES.md "Model hulls". Not changed: a group move order sends every ship to the same point, so a big fleet still jostles around it for a few seconds (a formation-keeping group move would fix that); `Fighter_Bay_Flyout_Distance` (c106d60) doubles the time fighters spend inside their own carrier's hull (data rule, kept). 2026-10-17: they no longer fly inside it: they leave along the bay bone's axis, from a Star Destroyer below the belly (RULES.md "Launch out of the hangar"); `tests/formations.rs` checks that no launched fighter lies in its own carrier's hull once out of the hangar (before: Star Destroyer 3032, Victory 858, MC-80 4860 of the positions from 3 s after launch, after: none of 55366).

## Fleet AI (2026-10-16, `ai.rs`, RULES.md 9c)
User finding: eight AI ships flew at the player's fleet and station one behind the other and died one by one. Every AI side now plans once a second after
`MoveToLocation.lua`, `SpaceTacticalFreeStore.lua` and `ReinforceSpace.lua`: the big ships advance as a line at the slowest ship's pace (corvettes ahead,
fighters escort), all attack together once one meets the enemy near its standoff (then the corvettes escort the big ships, `Service_Guard`), the side
focuses one priority target, a defender guards its station until
the free store turns aggressive, and the AI calls its reserve in (`BattleConfig::ai_reserve`, filled by galaxy-sim for AI sides). Ships hold their formation
slot by pace (`Ship::group_slot`, mov), so slow capitals never circle round a slot. Damaged ships turn the guns they still have to the target.
Tests: `tactical_ai::the_ai_fleet_advances_as_one_line_and_attacks_together` (before / after numbers in RULES 9c), `mechanics::the_ai_calls_its_reserve_in_while_the_unit_cap_has_room`,
the station test now checks that defenders hold until an enemy is within 3600 units of the station.
Open: the formation dissolves at the enemy location, so a fleet that reaches the station first still trickles towards ships elsewhere; Flank/Bomb/Escort
task forces of the scripts are not separate plans.

## AI retreat (2026-10-17, RULES.md 9 "Retreat")
User finding: the AI did not retreat although its fleet was nearly destroyed. Causes: attackers may only leave after 120 s (data) and the emergency rule (U94) only
covered capital ships and units of 15+ population, so a fleet of frigates and corvettes that was being destroyed before 120 s fought to the last ship; a defending fleet
without a station was treated as an attacker (120 s and 2x instead of the data's 30 s and 3x). Now: the emergency rule also covers a side that has lost half of its
peak strength, the galaxy's defender flag reaches the sim (`BattleConfig::defenders`), the reserve not called in yet counts at 0.8 (`ReinforcementsUnnormalized`), and
the "mainly transports" clause is in. Tests: `tactical_ai::a_nearly_destroyed_ai_fleet_retreats_before_it_is_wiped_out`,
`mechanics::a_defender_without_a_station_leaves_after_30_s_and_its_reserve_counts`.
