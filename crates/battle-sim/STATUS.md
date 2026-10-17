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
Squadron members follow the slot `leader position + (own place - leader's place), rotated with the leader` while idle, ordered to move or attack-move (and without a target), matching the leader's speed and closing the gap at 2 speed units per unit of distance (estimate). The places are the data's `Squadron_Offsets` as they are (`formation_offset`; 20 u between TIE Fighters, see RULES.md "Squadron formation flight"), the leader is the member nearest the middle of the formation; a leader in formation flight slows down while a member lags (from 50 u), launched squadrons follow the leader's place through the flyout, and a fighter at rest keeps its heading (members turn with their leader). A widening of the formation to 100 u (`ShipDef::spread`) existed for one day (2026-10-17) while fighters were drawn enlarged; it is removed.
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

## AI retreat counts the data's force (2026-10-17, RULES.md 9 "Retreat")
User finding: after the user's Super Star Destroyer fleet destroyed the AI's station, the AI fought on without a chance. Cause: the retreat rule compared our strength
estimate `sqrt((hull + shield) x damage)`, which put the Executor at 11 Star Destroyers; the data's `AI_Combat_Power` (what the engine's force perception sums) puts it at 36.
Now `ShipDef::ai_power` is read from the data and the force of a unit is its `AI_Combat_Power` times the share of hull and shield it has left (that weighting is the estimate);
the retreat rule and the defender's aggressive mode use it, the fleet AI's positional weights keep the old estimate. Same battle, seeds 1-3: the station fell after 126-133 s and
the AI fought on until 230-260 s, losing 11-13 of 22 ships; now it leaves at 62 s and loses 4 (later made to wait for its station: see "A defender holds while its station stands"). Probe: `examples/ai_probe.rs`. Open: the Normal/Hard checks of `RetreatPlan.lua`.

## Group moves (2026-10-17, `formation.rs`, RULES.md 9d)
User rule: a group move ends with every ship in its own slot, side by side in rows facing the move, tight (`BetweenFormationSpacing` beyond the hulls), 2-3 rows
for big groups. `Command::Move` / `AttackMove` of several units now lays out a formation at the point (bands by `FormationOrder`, escorts ahead, wide bands in
rows of equal count, row order by where the ships stand; a fighter squadron is one unit, flown by its leader, with the room of its members' places); every ship
flies its own route there (A*, hull separation; it lines up behind its slot and turns on the spot where it would swing out), the group keeps the pace of its
slowest member (`WaitsForFormationFormup`), and parked ships turn to the formation's facing (`FinalFormationFacingMinimumAngle`). The AI's line uses the same
layout with the start layout's room. Tests: `tests/group_moves.rs` (20 Star Destroyers in 2 rows of 10, mixed fleet rows and a second move, attack-move, a move
behind the fleet, launched craft and their squadrons, pace, data tags, determinism) and the unit tests in `formation.rs`. Open:
`FinalFormationFacingDeltaCoefficient` and `Max_Formation_Area` (meaning unknown); the AI's own moves after contact and its fighter escort still go to single
points.

## AI weaknesses measured and fixed (2026-10-17, RULES.md 9e)
User finding with an Executor fleet: the AI played very weakly. `examples/ai_bench.rs` measures the AI against another AI, a holding
player and a scripted focus player (open square, Kuat, Executor fleets). Found: against a holding Executor fleet the AI dealt no damage (it only went for
what it could see and chased TIE bombers while the Executor shelled it from beyond its sight), up to 35% of its full salvos went at corvettes or fighters,
and its ships weighed only the side's priority target. Now the AI plan knows the enemies that hit its units (`Default_Unit_Damaged`), every free unit gets
the free store's target (`Service_Attack`: capital ships take frigates, frigates corvettes, corvettes fighters), and full salvos follow
`Try_Offensive_Ability_Space`. Against the holding Executor fleet the first hit on a warship comes at 102-109 s instead of 200 s (seeds 1-2) and the AI
deals 14k instead of 0.2k (6 seeds); on the open square it takes 8-12% less damage and loses 9.2 instead of 10.0 of 14 ships against a holding fleet;
Kuat is unchanged. Open: hardpoint targeting sets (tried, no effect), kiting, the task force's ability use on approach, the scripts' map-wide
target search.

## Squadrons fly the data's formation again (2026-10-17, user decision 12:50: fighters at their real size)
The widening of fighter and bomber squadrons to 100 u between neighbours (`ShipDef::spread`, `SQUADRON_GAP`, `spread_formation`) is removed: it existed only because the
craft were drawn enlarged to a fixed screen size and ran into each other. Members fly `Squadron_Offsets` as they are (`formation_offset`; TIE Fighter 20 u side by side,
X-wing 50, TIE Interceptor pairs 6 apart in height; `ShipDef::closest_places` gives the closest two places, which battle-view fits its weave to). Kept from the first
version: the leader is the member nearest the middle, launched squadrons follow the leader's place through the flyout, a fighter at rest keeps its heading, the leader waits for
a member that lags (`SQUADRON_LAG_START` 50 u, half speed from `SQUADRON_LAG_HALF` 250 u, the same distances as before). Test: `defs::formation_tests`.
`tests/tactical_ai.rs`: the mirror fight on an open square is now judged on seeds 1-8 (at least 28 ships of 80 left, 3.5 a run) instead of the sum of seeds 1-3 (at least 12 of 30):
that sum was 12 on f9710b3, 11 with the data's formation (4, 3, 4), and both are noise: seeds 1-8 left 34 of 80 on f9710b3 and 36 of 80 now (3-6 a run in both), before the
rule 7 of 30 on seeds 1-3. The Kuat attack (seeds 1-8) is unchanged: depth, width and `half_in` are identical to the digit on both versions, ships left 8.4 on average now against
8.6 (its threshold is 8.0).

## A defender holds while its station stands (2026-10-17, RULES.md 9 "Retreat", polish2 R2)
The Remake's `Retreat_From_Space` lets a defender that has a station consider a retreat only after 120 s ("if I have a station, wait for the defense fleet to spawn") and one
without a station after 30 s. Our own emergency rule U94 (enemy above 3x with capital ships left) sent a defender away at the end of the 60 s retreat countdown (62 s) while
its station still stood. `Battle::ai_retreat` now holds such a defender (`defender && stations > 0 && time <= 120`) until its station falls or 120 s have passed; only the
nearly-destroyed clause (half of the peak strength lost) still lets it leave earlier. Attackers and defenders without a station keep U94 as it was. The player's own retreat
is untouched. Measured (`examples/ai_probe.rs`, seeds 1-3): an Executor fleet against a defended base (15 big ships): the order comes when the station falls, 104-106 s (before: 62 s),
5 / 6 / 7 big ships lost (before 5 / 2 / 4); 22 big ships: 122 s (before 62 s), 6 / 7 / 8 lost (5 / 6 / 6); an Executor alone against a base and 16 ships: 122 s (before 66-76 s),
3 / 3 / 4 lost (1 / 3 / 1); Golan III: 122 s (118-120 s). The AI loses a few more ships for holding on, as the data's wait implies. Tests: `mechanics::a_defender_with_a_station_holds_until_the_station_falls_or_120_s`,
`tactical_ai::an_ai_defender_holds_while_its_station_stands_and_leaves_when_it_falls`, `an_ai_defender_whose_station_outlasts_120_s_leaves_at_120_s_if_it_has_no_chance`; the older
`an_ai_defender_without_a_chance_against_a_super_star_destroyer_retreats_at_the_latest_when_its_station_falls` also checks that it does not leave before. Open: the nearly-destroyed clause counts
the station's own force in the fleet (a standing station keeps `fleet` high), so a defender with a station is rarely "battered" before the station falls.
