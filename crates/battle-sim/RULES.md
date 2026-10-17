# Tactical space battle rules used by `battle-sim`

Everything is derived from the Remake 4.0 data (`Data/Xml`) plus the `.alo` models for hardpoint
positions. Where the engine (closed source) defines a rule that is not visible in the data, the value
is an **estimate** and marked as such; estimates live in `BattleConfig` or in `defs.rs`.

## 1. Units, space and time

* Fixed step `dt` = 1/30 s. Battle space: X forward at yaw 0, Y left, Z up; ships move in the XY plane.
* XML speeds (`Max_Speed`, projectile `Max_Speed`) are per frame in the data and multiplied by
  `Catalog::speed_scale` = 30 (the 30 Hz logic tick; per-frame to per-second) to get units per second (gives Star Destroyer 90 u/s, laser bolts ~1000 u/s,
  turbolasers 1350 u/s). `Max_Rate_Of_Turn` is used as radians per second. Acceleration times by size class are estimates
  (fighter 1.5 s, corvette 4 s, frigate 6 s, capital 10 s, super 16 s to reach full speed).
* Model space (`.alo`) is Z-up with the ship's bow along -Y; it maps to battle space with `(x, y, z) -> (-y, x, z)`.
  Verified with the Star Destroyer: the bow turbolaser `HP_ISD_1_New_TL_05` ends up ahead (+X) and the shield
  generator dome behind (-X).

## 2. Ships, hardpoints, projectiles

| Rule | Data |
|---|---|
| Hull `Tactical_Health`, shield `Shield_Points`; `Shield_Refresh_Rate` is gained once per `ShieldRechargeIntervalInSecs` (2 s) as a continuous `rate/2` per second. `BattleConfig::shield_regen_delay` (default 0 = regenerate even under fire) can pause it after a hit; see section 8 for why 0 is the default | unit XML (`Units/Space/*.xml`), star bases `Buildings/Space/GC/**`, structures |
| `Armor_Type` / `Shield_Armor_Type` and the 673-entry `Damage_To_Armor_Mod` matrix scale hull and shield damage | `Xml/Gameconstants.xml` |
| Hardpoint position = `Fire_Bone_A` (else `Attachment_Bone`) in the ship `.alo`. Fixed weapons fire along that bone's local +X axis. **Turrets** (`Is_Turret`, `Model_To_Attach`) take their rest direction from the barrel of the attached turret model: `FP_01` minus `Barrel` bone in `Art/Models/<Model_To_Attach>`, rotated by the mount bone (the mount axes alone are authored arbitrarily: mirrored turret pairs point opposite ways) | `Units/Space/Hardpoints/*.xml`, `Art/Models/*.ALO` via `alamo-model` |
| Arc = larger of the fire cone (`Fire_Cone_Width`/`Height`, full angles in degrees) and the turret extents (`Turret_Rotate/Elevate_Extent_Degrees`), measured in the mount frame (forward = rest direction, up = bone Z). A mount whose arc cannot reach the battle plane at all is re-centered on the ship's bow | hardpoint XML |
| Range `Fire_Range_Distance`, `Fire_Min_Range_Distance` (measured to the target's bounding sphere) | hardpoint XML |
| Volley: `Fire_Pulse_Count` pulses `Fire_Pulse_Delay_Seconds` apart, then a random recharge between `Fire_Min/Max_Recharge_Seconds` | hardpoint XML |
| `Fire_Inaccuracy_Distance` per target class is added as aim error | hardpoint XML |
| `Fire_Category_Restrictions` lists the classes the weapon must **not** fire at (only ion cannons use it: no fighters, bombers, gunships, transports, corvettes) | hardpoint XML |
| Hardpoint `Health`, `Is_Destroyable`, `Is_Targetable` | hardpoint XML |
| Projectile: `Projectile_Damage`, `Damage_Type`, `Max_Speed`, `Max_Rate_Of_Turn` (homing), `Projectile_Max_Flight_Distance`, `Projectile_Does_Shield/Hitpoint_Damage`, blast area range/damage/victims, ion stun duration and speed/rate reduction | `Xml/PROJECTILES.XML` |
| Fighters fire a hull weapon: `Projectile_Types`, `Projectile_Fire_*`, range `Targeting_Max_Attack_Distance`, cone `Turret_Rotate_Extent_Degrees` | unit XML |
| A "facing" hardpoint (`Proj_Facing_Dummy`, damage 1, no hull/shield damage) makes the ship turn its bow towards its target | hardpoint XML |

Hit resolution: a projectile is a point moving along a segment; it hits the first enemy bounding sphere it crosses
(capital and frigate spheres shrunk to 75%). Shields absorb first (`damage x shield matrix`); if the shield is
depleted by the hit the remainder goes to the hull (`damage x armor matrix`). Projectiles without shield damage bypass
shields; ion cannons (no hull damage) only hurt shields. When shields are down every hull hit also damages the
destroyable hardpoint nearest to the impact (or the explicitly aimed one), full amount (estimate).

Destroyed hardpoints: weapons stop firing; **shield generators** (all of them) collapse the shield and stop regeneration;
**engines** scale speed by the surviving fraction (minimum 25%); **fighter bays** stop squadron replacement.
Ion stun slows the target and its fire rate for `Projectile_Ion_Stun_Duration`.

## 3. Squadrons, carriers, stations

* A `Squadron` object expands into `Squadron_Units` fighters placed with `Squadron_Offsets`.
* `Starting_Spawned_Units_Tech_0` of ships and star bases are launched at battle start; when a launched squadron is
  wiped out, a replacement from `Reserve_Spawned_Units_Tech_0` appears after `Spawned_Squadron_Delay_Seconds`,
  as long as a fighter-bay hardpoint (if the ship has any) is alive.
* Star bases, shipyards and defense structures are static combatants with their own hardpoints.
* Fighters stay with their squad leader / carrier when idle and fly strafing runs (approach, break away 1-2.5 s) against targets.

## 4. Targeting and movement (AI)

* Every 0.4 s a ship scans enemies and keeps a candidate list sorted by `Targeting_Priority_Set` weight (lower first, then
  distance). The sets come from `Xml/TARGETINGPRIORITYSETFILES.XML` (`SpaceUnitTargetingPriorities.xml`): `Attack_Priorities`
  (class -> weight, `Golan` for Golan platforms), `Category_Exclusions`, `Hard_Point_Priorities`. A ship uses its own
  `Targeting_Priority_Set` (e.g. `Capital`). The `<set>_Attack_Move` variants (category exclusions and hardpoint sniping
  order: shield generator, engines, hangar, weapons) exist in the data but the Remake scripts only assign such sets to ground
  task forces, so `BattleConfig::attack_move_sets` (default off) enables them for AI attack-move orders. Classes missing from a set are only opportunity targets (lowest priority).
* Each weapon fires at the first candidate that is in range, inside its arc and allowed. Heavy guns (damage >= 150) and
  shield-only weapons never fire at fighters or bombers.
* Large ships close to the range that puts 80% of their weapon damage in reach (x0.95) and hold there; a long-range specialist (hull-damaging weapons backed by 25% of the damage that out-range that by 1.5x, e.g. a missile ship) holds at 0.8 of its longest range (estimate), plus half the target radius, turning to the
  bearing (bow, broadside or stern) at which the largest share of their hardpoint damage can bear on the target
  (`ShipDef::bearing_dps`, computed from the arcs; a damaged ship counts only the guns it still has). The standing orders of an AI side come from its fleet AI (section 9c).
* Big ships stop at that range with an arrival speed profile (no overshoot), start turning to the weapon bearing in the last stretch of the approach, and position themselves against the best non-fighter target (fighters and bombers are fired at but not chased, unless explicitly ordered). Restart hysteresis 1.12 x standoff (estimate).
* Orders: `Move`, `AttackMove`, `Attack(ship)`, `AttackHardpoint(ship, hardpoint)` (aims at the hardpoint and the hit
  routes to it), `Stop`, `Retreat`, stances Aggressive / Defensive / HoldFire.
* No ship-ship collision or avoidance; AI fleets keep a formation on the approach (section 9c); a group move ends in a formation at its point (section 9d).

## 5. Abilities

`Unit_Abilities_Data` entries with an `Expiration_Seconds` become abilities with the `Mod_Multiplier` values of the
data: `SPEED_MULTIPLIER`, `SHIELD_REGEN_MULTIPLIER` (negative values are clamped to 0), `WEAPON_DELAY_MULTIPLIER`
(applied to the recharge, below 1 = faster) and `TAKE_DAMAGE_MULTIPLIER`. `FULL_SALVO` uses the per-hardpoint
`Full_Salvo_Weapon_Delay_Multiplier`, `POWER_TO_WEAPONS` additionally the per-hardpoint
`Power_To_Weapons_Mode_Weapon_Delay_Multiplier`. `Recharge_Seconds` starts after the effect ends. AI sides trigger
weapon boosts when something is in range. Also implemented:

* `TRACTOR_BEAM`: needs a living tractor-beam hardpoint; picks the current or nearest enemy in `Activation_Max_Range` whose
  class is in `Applicable_Unit_Categories` and slows it by `Target_Speed_Decrease_Percent` while active and in range.
* `MISSILE_SHIELD`: a toggle (`Expiration_Seconds` 0) that destroys enemy missiles, torpedoes and rockets inside
  `Effective_Radius` and pins the ship (`Disable_Movement`); AI sides switch it on under missile fire and off again.
* `INTERDICT`: a toggle that pins the ship and, while on (or for `Is_Interdictor` units), blocks the enemy `Retreat` command.

Not implemented: stealth, sensor jamming, and every other ability type.

## 6. Reinforcements, unit cap, retreat, victory

* `Command::Reinforce` adds ships at the side's spawn after `reinforce_delay` (5 s = `Skirmish_Reinforcement_Delay_Frames` 150 at 30 fps)
  if `population` of everything in play plus the request is within `unit_cap` (150, the `Space_Tactical_Unit_Cap` value
  of the faction XML). Reinforcement points and the reinforcement zone are not modelled.
* Retreat becomes available after `Space_Retreat_Allowed_Countdown_Seconds` (60 s, data) and is blocked by enemy interdictors. `Space_Retreat_Attrition_Factor` is 0 in the data.
  On the order (player or AI) every mobile unit turns towards its own spawn point and heads there; stations stay. A jump timer `BattleConfig::retreat_jump_seconds`
  (default 10 s, **estimate**: the user said 10 or 20 s and the data has no value; `Battle::retreat_jump_in(side)` gives the countdown) runs. At zero every living
  unit with at least one living `HpKind::Engine` hardpoint (ships without engine hardpoints count as having drives) escapes and counts as retreated survivor; the others are lost.
  Units spawned in the battle do not escape on their own: a carrier's launched squadrons escape only if the carrier does, squadrons called in as reinforcements are lost.
  Pressing Retreat again while the timer runs calls the retreat off (EaW; `Command::CancelRetreat`): the timer stops, the retreating units stop and wait for orders,
  `Event::RetreatCancelled` is reported (`Space_Retreat_Cancel_SFXEvent`), and a later order starts a full timer again. A second `Command::Retreat` still only keeps the timer.
* A side loses when it has no ship other than fighters/bombers left (a fighter-only side loses when its fighters are gone).
  The loser's remaining fighters are destroyed with it. Battles are a draw after `max_time` (60 min: the data has no battle time limit, this only guards headless runs). Mobile ships climb or sink towards their target's height at half top speed (flight is otherwise planar, and launch-bay or spawn height offsets used to keep ships out of weapon range for ever).
* Arena: `BattleConfig::arena` (`Arena::load(data, fs, "<map>.ted")` via the `alamo-map` crate) gives the playable bounds,
  `Attacker Entry Position` markers (attackers, at -Y), `Defending Forces Position` markers (defenders, at +Y), the
  `Space Station Position` (star base), structure marker spots (`Shipyard N Marker`, `Golan All-in-One Marker`, ...) and asteroids
  as round obstacles that absorb projectiles and push ships away (radius by name: huge 600, large 350, medium 180,
  small 80). Without an arena fleets start in rows by size class in an empty square of `half_size` = 10000. Nebulae, the planet
  and backdrop props are not obstacles.

## 7. Performance design

Per step: snapshot of positions and a uniform grid (500 u cells, ship bounding boxes inserted in all cells they overlap),
then rayon-parallel target scans, movement, weapon updates (fire requests) and projectile flight/collision. Hit
application, upkeep and spawning are sequential and in index order. Randomness comes from per-entity generators seeded by
`(seed, tick, id)`, so the result is identical with or without threads (tested).

## 8. Balance notes (duels, `cargo run --release -p battle-sim --example battle_duel`)

The outcome of capital duels hinges on one mechanic the data cannot settle: whether shields recharge while under
fire. `Shield_Refresh_Rate` is gained once per 2 s (`ShieldRechargeIntervalInSecs`); the Remake comments say the regen was
boosted 2.5x deliberately. MC-80 (shield 30000, regen 600/2 s = 300/s, hull 20000, no shield-generator hardpoint) versus
Star Destroyer (shield 15000, regen 150/s, hull 40000, shield generator 1500 hp):

| regen while hit | ISD vs MC-80 | 2 v 2 | 5 v 5 | 10 v 10 |
|---|---|---|---|---|
| continuous (default; matches "an MC-80 with shields up often beats an ISD") | MC-80 wins (800 s) | MC-80, 2:0 | draw, 1:2 left | ISD wins, 4 of 10 left |
| paused 2 s after a hit | ISD wins (199 s) | ISD 2:0 | ISD 4:0 | ISD 6:0 |

A single ISD deals about 290 effective shield DPS to an MC-80 (ideal 827; ion cannons only fire at frigates and larger,
many guns are out of arc or range) against 300/s regeneration, so it cannot break the MC-80 shield alone; the MC-80
(about 210 DPS) does break the ISD shield (150/s regen). Focus fire of several ships flips the result, so large fleets favour the
Empire. Hardpoint sniping (shield generators) is not used by default; with `attack_move_sets` on, MC-80 fleets snipe the ISD
shield generator and win every size.

### Time units and depleted shields (audit)

All XML `*_Seconds` fields are real seconds; `Max_Speed` (ships and projectiles) is per frame and is multiplied by 30 (the 30 Hz logic tick). A shield drained to zero stays offline for `Depleted_Shield_Disable_Time` (4 s, from Gameconstants) before regeneration resumes; this is what makes continuously regenerating shields breakable and shortens the 1v1 ISD vs MC-80 duel from ~800 s to ~270 s. Squadron-only fleets start at the side origin (rows are packed), not 2100 units ahead of it. Energy (`Energy_Capacity`) is not modelled; neither the ISD nor the MC-80 uses it.

## 9. Tactical AI review against the Remake data (what is data, what is estimate)
Source: `XML/AI/GoalFunctions/Remake_SpaceCombat.xml`, `PerceptualEquations/Remake_SpaceCombatEquations.xml`, `Scripts/AI/Spacemode/*.lua`. The Remake tactical AI is a set of goals
(Destroy / Flank / Bomb / Strike / Focus_Weak / Escort / Reinforce / Retreat); the engine part that matters for us is implemented as follows.

* **Retreat (`Retreat_From_Space`, implemented):** an AI side orders `Command::Retreat` (every 2 s check) when it is clearly outgunned. Attacker: enemy force >= 2 x own force and battle age > 120 s;
  defender: enemy >= 3 x own (4 x with a living station) and age > 120 s (> 30 s when it has no station). "Defender" is the galaxy's `IsDefender` (`BattleConfig::defenders`, galaxy-sim
  sets it from `TacticalSide::defender`; without it a side that brought structures defends), so a defending fleet without a station may leave after 30 s instead of being treated as an
  attacker. A side made mostly of transports (own force < 3 x the transports' force, `Forces_Are_Not_Mainly_Transports`) leaves whatever the odds, an attacker already after 30 s.
  Also required: the side's `BattleConfig::retreat_allowed` (`Allowed_To_Retreat_Space`: campaign only, faction holds more than one planet, never Pirates; galaxy-sim sets it per
  `TacticalSide::may_retreat`), the retreat countdown (60 s) passed, and no enemy interdiction (`RetreatBlocked` is simply retried). The thresholds and ages are data, and so is
  the force (`FriendlyForceUnnormalized`, `EnemyForceUnnormalized`): the units' `AI_Combat_Power` (`ShipDef::ai_power`; the Remake names its force total
  `Player_AI_Combat_Power_Space_Total`): Executor 65000, Star Destroyer 1800, MC-80 1750, Victory 1200, Assault Frigate 1200, Nebulon-B 300, CR-90 100, X-wing 10, TIE 4, Rebel star
  base 3 3750, Golan III 19500. Each unit counts with the share of hull and shield it has left (**estimate**: the engine's weighting is not visible); units the side has not called
  in yet or that are on their way count at 0.8 (`ReinforcementsUnnormalized {Parameter_Attenuator = 0.8}`, read as a factor: estimate). Units without the tag count 0 (of 987 armed
  space units 78 have none: dummies, story units, transports). The aggressive mode of 9c compares the same force. Until 2026-10-17 the force was our estimate
  `sqrt((hull + shield) x best-bearing damage rate)`, which put the Executor at 11 Star Destroyers instead of the data's 36 (user finding: the AI kept fighting after its station
  was destroyed, without a chance against a Super Star Destroyer fleet). Measured (`examples/ai_probe.rs`, seeds 1-3, scripted player attacking the station,
  before = main f9710b3): Executor, 3 Star Destroyers, 3 Victorys and 4 TIE squadrons against a Rebel star base 3 with 22 ships: before, the station fell after 126-133 s at
  4.5-4.9x, the AI fought on until 230-260 s and lost 11-13 of its 22 ships; now it leaves at 62 s (3.7-3.8x) and loses 4. The Executor alone against 16 ships and the base: order
  at 300-320 s (7-8 of 16 lost) before, 96-102 s (1 lost) now; against a Golan III and 10 ships 180-240 s (4-7 of 10 lost) before, 108-172 s (2-4 lost) now. An even fight in which
  the station falls (6 Star Destroyers, 4 Victorys against 10 ships and a base 1, 1.4-1.5x when it falls) is fought on until the enemy is 3x stronger (194-206 s). Tests:
  `tactical_ai::an_ai_defender_without_a_chance_against_a_super_star_destroyer_retreats_at_the_latest_when_its_station_falls`, `..._with_a_fair_chance_fights_on_after_losing_its_station`,
  `an_ai_attacker_that_is_losing_badly_retreats` (6 MC-80s and 4 Nebulon-Bs 4500 from an Executor and a star base: order at 62 s instead of 80 s).
  **Emergency retreat (U94, project rule, estimate):** after the retreat countdown, without waiting for the 120 s, a side whose survival chance is below 25% (enemy force > 3 x own)
  leaves while capital ships or units of 15+ population are left, or (user finding 2026-10-16: a nearly destroyed fleet of frigates and corvettes fought to the last ship) once its
  ships without fighters and bombers have lost half of the strength they had at their peak in this battle (a side of fighters and bombers only keeps
  the data rules). With the data's force this rule makes a defender facing a Super Star Destroyer leave at the end of the countdown even while its station stands
  (the data alone would keep it until the station falls or 120 s). Tests: `a_nearly_destroyed_ai_fleet_retreats_before_it_is_wiped_out`
  (3 Acclamators and 3 Tartan pairs against a defended planet: order at 110 s instead of 122 s, 3 of 3 Acclamators escape instead of 2 / 1; 4 Tartan pairs: order at 68 s instead of
  never), `a_defender_without_a_station_leaves_after_30_s_and_its_reserve_counts`.
  Not modelled: the Normal/Hard checks of `RetreatPlan.lua` (abort when engines of more than half the frigate/capital combat rating are down; Hard finishes off enemies below 20%
  hull and shield first) and `AI_CAN_NOT_RETREAT` while `FocusWeak.lua` runs. Retreating ships must still fly out of the arena, so a late order saves only the fast or distant ships.
* **Focus weak (`Prevent_Retreat_On_Priority_Target`, implemented):** a non-fighter enemy with less than 33% of hull + shield left (data threshold) inside weapon reach gets its priority weight halved
  (factor 0.5 is an estimate), so ships finish off wounded targets instead of spreading fire.
* **Ability use (implemented generically from the ability numbers, not names):** besides the weapon boosts, tractor beams and missile shields of section 5, AI ships now use expiring abilities
  with `speed_mult > 1.05` when fleeing or when the target is more than twice the weapon range away while on a move order, `shield_regen_mult > 1.05` when the shield is below 50% and
  was hit in the last 3 s, and `take_damage_mult < 0.95` when the hull is below 60% under fire (the percentages are estimates; the data does not list AI hints for these).
* **Separation (new, estimate):** mobile non-fighter ships push each other apart when closer than 0.85 x the sum of their (frigate+ shrunk) radii, half of the overlap each, at most one
  tick of speed per tick; stations are excluded (defenders guard the station position). Measured with 8 ISD vs 6 MC-80 + 6 Nebulon-B: worst overlap 16% of the radii (before: ships stacked).
* **Already data-driven before:** target priorities by `Targeting_Priority_Set` (counter types, fighters vs bombers/capitals), hardpoint priorities (shield generator, engines, hangar, weapons) for
  attack-move sets (off by default, see section 4), squadron launch and replacement from carriers/stations (section 3), defenders guarding their station, attackers attack-moving.
* **Not implemented:** Flank/Escort/Bombing-run goals as separate behaviours (bombers use their priority sets),
  the Death Star and stealth/cloak (`Try_Ability STEALTH`). AI reinforcements and formations: section 9c.

### 9b. Bombing runs, intercept, formations (new; all numbers are estimates, the data has no AI values for them)
* **Bombing run:** a `Bomber` that reaches a frigate-or-larger target (ships and stations) flies straight on for 3-5 s (turn at most +-0.15 rad/s) before turning back for the next pass; against small targets and for fighters the old 1-2.5 s random break-away stays. Test: a bomber makes repeated passes on a static capital.
* **Intercept:** a `Fighter` with a carrier prefers (weight x0.3 within its priority set) enemy bombers that come within 3500 units of that carrier. **Escort** is the existing carrier/squadron-leader following; bombers get no extra escort behaviour. The intercept rule has no dedicated test.
* **Formations:** each fleet unit remembers its lateral position in the start line (`Ship::lane`, +-1500 max). Under attack-move orders, while the enemy is more than 2500 units away, the aim point is shifted sideways by that lane, so the fleet advances line abreast; broadside arcs still come from the bearing logic (section 4). While no enemy is within twice the weapon range a big ship does not outrun slower friendly big ships within 2500 units (cohesion, never below 40% of its own speed).
  Test: 3 fast + 3 slow frigates are 25 units apart in depth after 25 s (2450 without cohesion) and still 1400 units wide.
* **Ability rules** (section 9) now have a unit test (`ai_uses_speed_shield_and_damage_abilities_by_their_numbers`).

### 9c. Fleet AI: formation, priority target, station defence, reserve (`ai.rs`, user finding 2026-10-16)
The user saw eight AI ships fly at their fleet and station one behind the other and die one by one. Every AI side now runs a plan once a second
after the Remake scripts `MoveToLocation.lua` (`AotR_Beginning_Assault`), `SpaceTacticalFreeStore.lua` and `ReinforceSpace.lua`.
* **Data:** `FREE_STORE_ATTACK_RANGE` 3600 (priority target and station threat), `FREE_STORE_DEFEND_RANGE` 6200, `FIGHTER_AGGRO_RANGE` 3000, the task force
  moves until 5000 from the enemy location (`MoveToLocation.lua`), a unit with an enemy warship or structure in weapon range leaves the task force, the
  priority enemy is picked from the classes station/capital, frigate, corvette in that order by the script's weakest-and-nearest scan
  (`Get_Shield_And_Health`), aggressive mode (`Allowed_As_Defender_Space_Untargeted`: attackers, defenders without structures, defenders whose mobile
  force is 2x the enemy's after 120 s; or the station below 70%, or an own big ship near the enemy), the reserve is called in one unit at a time while
  the population cap has room (`Want_To_Reinforce_Units`).
* **Formation (estimates):** the free big ships form a line (frigates and capitals) with the corvettes and transports 700 units ahead, each row ordered by
  where its ships stand across the line of advance (re-sorted every second, each ship keeps `radius x 2.2 + 60` of room; laid out by the slot layout of
  group moves, section 9d, with this start layout's room, so a band wider than 6000 stands in several rows 700 apart). The slots start at or ahead of
  every ship. The anchor advances at the slowest big ship's speed, slowed down to 25% while a ship is 500-2000 units behind its slot, standing while
  reinforcements arrive within 3000 units, turning no faster than its slowest ship and its outer slots allow; a goal more than 1 rad off the heading lays
  the line out anew. A big ship flies the formation's velocity plus 0.25/s times the distance to its slot, never backwards (it waits for a slot that fell
  behind it), so slow-turning capitals do not circle round their slots. Free ships within 3000 units join, farther ones catch up. Fighters escort the
  formation (carrier squadrons stay with their carrier, the others 1400 units ahead of the line) until an enemy is within 3000 units of them.
* **Attack together (estimate):** once any member has an enemy warship within 1.5x its standoff distance the whole formation is released and attack-moves
  on the priority target (corvettes: below). A missile ship that only reaches the enemy with its longest gun leaves alone.
* **Priority target (estimate of the weight):** ships within 3600 units of the side's priority target count it at half its priority weight (scan), so the
  priority sets still decide between classes (an anti-fighter corvette keeps shooting fighters first). A defender that holds back focuses the enemy nearest
  to its station.
* **Station defence:** a defender that is not aggressive guards its most damaged structure (`Guard`), and attacks when the free store turns aggressive.
* **Search:** with nothing in sight the fleet sweeps the old search points; a point's time limit (90 s, then 45 s) restarts while the fleet still closes in on it.
* **Reserve:** `BattleConfig::ai_reserve` (galaxy-sim passes `TacticalSide::reserve` for AI sides; headless battles use it too). Units arrive at the
  side's reinforcement point, or 1500 units behind the own fleet when a visible enemy is within 3000 units of that point (estimate). Results report them under
  `REINFORCE_TAG`. Test `the_ai_calls_its_reserve_in_while_the_unit_cap_has_room` (mechanics).
* **Corvettes (data):** at contact a corvette of an attacking side follows `Service_Guard`: it attacks fighters, bombers and transports within its aggro range
  (longest weapon range, at least `SHIP_AGGRO_RANGE` 3500), else the enemy nearest to the own frigate, capital ship or station nearest to it (stations and
  capital ships first, then frigates, then corvettes) once that enemy is within its aggro range of it, else it stays beside that ship (150 units of room:
  estimate). Before, the corvettes attack-moved on the priority target with the big ships and died at the enemy station.
* **Measured** (`tactical_ai::the_ai_fleet_advances_as_one_line_and_attacks_together`, 8 Empire ships: 2 ISD, 2 VSD, Acclamator, Broadside, 2 Tartan pairs;
  "fleet" = the 10 ships without fighters; seeds 1 / 2 / 3; "before" = main a8316dd with the hull separation of 5a69699):
  * the same fleet waiting on an open square: before a blob 1577-1595 wide and 1572-1590 deep when the first ship comes within 6000 units of the enemy,
    2 / 2 / 3 fleet ships left, damage dealt / taken 1.33 / 1.30 / 1.32, won after 785 / 453 / 374 s; now a line 3312-3318 wide and 892-897 deep,
    6 / 3 / 4 left, 1.46 / 1.51 / 1.53, won after 348 / 356 / 349 s. Damage in the 60 s after the first hit 71 / 59 / 66k before, 85 / 88 / 84k now.
  * attack on a defended Kuat (MC-80, 2 Nebulon-B, 2 CR-90, star base 2): approach depth 2124-2134 before, 922-947 now; dealt / taken 3.34 / 4.07 / 4.02
    before, 4.50 / 3.91 / 4.83 now; won after 539 / 538 / 547 s before, 377 / 383 / 401 s now; 9 / 9 / 9 fleet ships left before, 9 / 10 / 9 now.
    Half the fleet in action 29 / 29 / 33 s after the first shot before, 39 / 38 / 33 s now, and the damage in the 60 s after the first hit 54 / 57 / 58k
    before, 53 / 54 / 55k now: the long-range Broadside opens the fight while the line still closes in.
* **Damaged ships:** `align_to_target` counts only the weapon hardpoints that are still alive (`HpDef::arc_bins`, `HpDef::dps`); before, two damaged
  capitals could face each other with their dead batteries and stop fighting for good (seen in `even_fights_do_not_trigger_the_retreat` once the
  fleets met as lines: a draw after 1500 s; now decided after about 500-550 s).

### 9d. Group moves: a formation at the destination (`formation.rs`, user rule 2026-10-17)
Project rule: in EaW, once a move order is given, every ship plans its own route to its place without collisions and flies there; 20 Star Destroyers
then stand in a row, tight against each other, if needed in 2-3 rows. Before, a group `Move` / `AttackMove` sent every ship to the one point
and the ships jostled there.
* **Slots:** a `Move` or `AttackMove` of two or more units (a fighter squadron is one unit: its leader flies the slot, the others keep their places around
  it; static units and `FormationGrouping` `Solo` units stay out) lays out one formation centred on the ordered point. It faces the move
  (`Rotate_Formation_Facing_Moves`, data; with the flag off, the way the group faces). Bands front to back: `FormationOrder` (data; 1 wherever a Remake space
  unit sets it), then the escorts (fighters, bombers, corvettes, transports) ahead of the frigates and capital ships (estimate, as the AI line). A band wider
  than 6000 in one row splits into rows of equal count (estimate: 20 Star Destroyers stand in 2 rows of 10), filled front to back by `Formation_Priority`
  (data; likewise 1) and by how far ahead the units already are; every row is ordered by where its units stand across the move, so their ways do not cross.
* **Spacing:** neighbours stand `BetweenFormationSpacing` (GameConstants: 50; a unit's `FormationSpacing` multiplies it) beyond what keeps their hulls
  apart, the larger of the two plan-view hull half widths and the hull separation's 0.85 of the two snapshot radii (an escort keeps `ESCORT_GAP` 150 more
  from a capital ship); rows likewise with the hulls' reach ahead and behind. Star Destroyers stand 589 apart (157 between the hulls), Victorys 383, a
  Victory next to a Star Destroyer 713, corvettes 101. A fighter squadron takes the room of its members' places around its leader (`Ship::formation`),
  each member within 50 of its place (a fighter that holds a place stops within 50), at least 90 every way (estimates, as the AI line): counted as round
  units of radius 90, 4 pairs of the 12 squadrons six Star Destroyers launch reached into each other (1 pair with their places but without the 50), now none.
* **Routes (estimates):** every ship flies its own route, the A* path around rocks and stations and the hull separation. On the last, straight leg a big
  ship or a squadron's leader steers at the point of its slot's line (along the formation's heading) halfway between its own place along that line and the
  slot: it lines up behind its slot on the way and comes in straight (a squadron keeps its leader's heading at rest, so it faces the move: 0.1 degrees off
  instead of up to 12.7). It leaves the other ships of its formation to their slots (steering round them made neighbours that close up side by side push
  each other off their slots), except the ones that stand while it cuts across the lanes (more than 0.35 rad off the formation's heading).
  A ship that faces more than 1.2 rad away from where it steers, or more than 0.35 rad while it is closer to its slot than its turning radius or at the
  start of the move, first turns on the spot to within 0.05 rad of it, to the plain bearing (so a formation sent behind itself turns round where it stands,
  and nobody circles a slot inside its turn). It stops 20 from its slot, counts as parked within 30 and stays parked within 120 (a push does not send it round
  again).
* **Facing:** a parked ship that is more than `FinalFormationFacingMinimumAngle` (data: 5 degrees) off the formation's facing turns to it, and then all the
  way. `FinalFormationFacingDeltaCoefficient` and `Max_Formation_Area` are not used: what they do in the engine is not known.
* **Pace:** space units wait for each other (`WaitsForFormationFormup`; the data turns it off only for ground units): every unit flies the share of its top
  speed it needs to arrive when the last one can (path length, plus the time a big ship needs to turn towards its slot), never below 25% of its top speed
  (estimate); a unit that fights (attack-move with a living target) or has parked does not hold the group up. A unit with `WaitsForFormationFormup` false
  flies at its own speed.
* **Orders:** attack-moving units fight what they meet and then go on to their slots. Any other order takes a unit out of its formation; a single unit moves
  as before. The battle view's move line follows each ship's path to its own slot; the ring stays at the ordered point.
* **AI:** the AI's advancing line (9c) is laid out by the same code with the start layout's room (`radius x 2.2 + 60` across per unit, rows 700 apart, the
  line's own arithmetic), so AI battles come out exactly as before (open square seeds 1-3, Kuat seeds 1-8, the retreat scenario of 9 "Retreat"); a band wider
  than 6000 now stands in rows. With the tight room the AI fleet closed up first and met the enemy 5-9 s later (the battered fleet of
  `a_nearly_destroyed_ai_fleet_retreats_before_it_is_wiped_out` ordered its retreat at 118-122 s instead of 110 s, measured on main 892edaf).
* **Measured** (`group_moves.rs`, seed 1; before = main 892edaf, after = on main 87d8863):
  * 20 Star Destroyers sent 6000 ahead: before a blob 3450 wide and 4900 deep, neighbours 530-2810 apart, still moving after 240 s (926 units and 63 degrees
    in the last 20 s), headings up to 171 degrees off the move; now 2 rows of 10, neighbours 589 apart (157 between the hulls), each row straight within
    0.1 units, every heading within 1 degree of the move, standing after 136 s, hull capsules never more than 0.4% inside each other, and nobody moves or
    turns in the last 20 s.
  * 3 Star Destroyers, 4 Victorys, 4 CR-90s and 2 Tartan pairs sent 5000 off at 40 degrees: before a blob 1610 wide and 1500 deep with the corvettes between
    the capital ships, still moving after 240 s (717 units, 171 degrees in the last 20 s), headings up to 178 degrees off; now the 8 corvettes in a row
    ahead of the 7 big ships, standing after 83 s, headings within 0.2 degrees. Sent on 4000 at a right angle to that move, the rows turn with it (standing after
    121 s, overlap at most 0.5%).
  * Sent 3000 behind themselves, 6 Star Destroyers and 4 Victorys turn on the spot and stand in 2 rows of 5 after 158 s. Attack-moving through 2 CR-90s, a
    fleet of 6 destroys them and forms its line at the point.

## 7. Positional reinforcements, fog of war, more abilities (see ABILITIES.md)

* `Command::ReinforceAt { ship, count, pos }`: allowed when `pos` is inside the battle area, outside the no-spawn zone of every enemy structure and inside an area the side sees (`placement.rs`, shared with the UI).
  The zone radius is the structure's `Reinforcement_Prevention_Radius` where the data has it (only the ground planet shield does), else `BattleConfig::no_spawn_radius` = 2000 (**estimate**, the number the Remake AI
  equations `Target_Is_Near_Friendly_Unit` / `Target_Is_Not_Close_To_Enemy_Units` use for "near"); "seen" = within `Space_FOW_Reveal_Range` of an own ship or structure (or a ping area), always with fog of war off;
  errors `OutsideBattleArea`, `InNoSpawnZone`, `NotVisible`, plus `UnitCap`. Enemy mobile ships do not block a spot. `Battle::reinforcement_zone_check(side, pos)` is the same test for an overlay (green / red). The ships arrive after `reinforce_delay` (5 s) at free space
  (nothing within `reinforce_clearance` = `Space_Reinforcement_Collision_Check_Distance` 150; they move out in rings when the spot is taken), facing the nearest enemy. `Reinforce` without a position still uses the side's point.
* Fog of war (`BattleConfig::fog_of_war`, default on): a ship sees `Space_FOW_Reveal_Range` (`ShipDef::sensor_range`, 1250 where missing); an enemy it saw stays visible for `fog_linger` = `SpaceFOWRegrowTime` 15 s.
  Targeting (`scan`) only considers visible enemies, so idle and attack-moving ships no longer know where the enemy is; the AI respects it because it uses the same scan. `Battle::is_visible_to(side, ship)` for the renderer.
  Stealthed ships (`STEALTH`) are seen only within `stealth_detection` (500, estimate); `FOW_REVEAL_PING` reveals an area for 5 s (estimate).
* Also: `LASER_DEFENSE`, `Force_Healing_Ability`.

## Projectile combat mods
`Projectile_Combat_Mod` (`COMBAT_MOD_SPEED`, `_FIRE_RATE`, `_DEFENSE`, `_HEALTH`, `_SHIELD` with `_Value`, `_Duration`, `_Category_Mask`): the ship hit and every ship within the projectile's `Projectile_Blast_Area_Range` of the impact (the data says this range, not `_Radius`, governs the area) get the change for the duration. Negative values hit enemies of the shooter, positive ones help its friends. Speed and fire rate are multiplied by `1 + value`; damage taken (all, hull only, shield only) is divided by `1 + value`. The strongest value of a kind counts and a new hit refreshes the time; they do not stack (estimate, the data does not say). Example: the proton torpedo's burn, -25% health for 14 s on fighters, gunships, bombers, transports and corvettes.

## Navigation (`nav.rs`, `BattleConfig::navigation`)
* **Static obstacles**: the rocks of the map and the stations form one distance field per battle (250 unit cells). A big ship (corvette and up) that goes to a place plans a path over it with A* and pulls the string, keeping `0.75 x radius + 40` from every rock and station; the path is replanned every 0.5 s and when the destination moves by more than 300 units. A place inside a rock is approached to the rock edge. `Ship::path` holds the waypoints still ahead (last = destination); `battle-view` copies them to `ShipSnap::path` for the move line.
* **Other hulls**: big ships look 4 s ahead (at least two radii) and steer sideways around hulls in their way (head-on meetings pass on the right) and slow down when one is close ahead; the older push-out (85% of the two radii, now also against stations) stays as the hard limit. Estimates: lookahead, weights, 0.5 s replanning.
* **Fighters** fly through rocks and along any line, but are pushed out of the core of every big hull (`0.45 x hull radius + own radius`), except their own carrier.
* **Model hulls (2026-10-16, estimate):** the collision radius is clamped per class (Capital 150..700), so long ships were far larger than their circle: the 9000 unit Executor kept others only 0.85 x 0.75 x 700 away from its centre, and MC-80s (radius 150, hull 540 x 450) sat inside each other for minutes. `ShipDef::hull_shape` is the plan-view capsule of the model's visible full-detail meshes (bounding box corners in battle space: reach ahead, behind, half width; `nav::Hull`; a mesh hull under 0.3 of the radius counts as a placeholder and is ignored), `ShipDef::footprint` falls back to the old circle. Big ships keep both the old circles and the capsules apart (`nav::hull_overlap`, the deeper overlap wins), escorts keep `ESCORT_GAP` 150 from capital hulls, and the push-out runs at up to `PUSH_SPEED` 2 x top speed (it was 1 x: two ships driving for the same point held each other inside). Fighters also keep `FIGHTER_KEEP_OUT` 0.5 of a model hull's half width from its long axis.
  Measured with capsule overlaps above 10% of the half widths (4 ISD, 4 Victory, 3 Arquitens, 3 Tartan, 6 TIE squadrons vs 4 MC-80, 4 Nebulon-B, 3 AF-II, 3 CR-90, 6 X-wing squadrons, 150 s, seeds 1-3): AI against AI 330-427 samples per battle with pairs inside each other for 85-140 s before, none after; the player's big ships all ordered onto one point 700-935 samples and 80-140 s before, 0-41 samples and at most 13 s after; the player attacking an Executor 643-866 samples and 91-114 s before, 0-6 samples and 3 s after. Fighters deep inside a hull that is not their carrier: 1-7% of the fighter samples before, at most 0.1% after. Tests: `tactical_ai::big_hulls_do_not_stay_inside_each_other_and_nobody_parks_in_an_executor`, `navigation::ships_and_fighters_keep_out_of_a_long_hull_along_its_whole_length`.
* `navigation: false` gives the old behaviour for comparisons (`galaxy-sim/examples/navcheck`: Bilbringi 61 rocks and Hoth, 4 Star Destroyers + 3 Victory + fighters vs MC-80, Nebulon, CR-90 + fighters, 300 s, three seeds: no hull overlaps after the opening, no ship inside a rock, no planned path through a rock; fighters deep inside a hull 307 / 177 / 319 / 372 samples before, 0 after).

## Battle end (U27)
A side stays in the fight while any unit it **brought** lives: the ships, squadrons, star base and structures of the setup (the galaxy fleet and planet defences), and called-in reinforcements. Units
**spawned during the battle** (a star base's garrison squadrons, squadrons a carrier launches from its reserve) do not count: they are ignored when the battle ends and removed with the loser. A side
whose brought units are all destroyed or have retreated loses (a retreat of everything left is a win for the other side); a hand-made scenario without setup units counts any ship. (First attempt: all
units counted with a 120 s grace for fighters; the user corrected it to "spawned units may be ignored".) Tests: `a_side_stays_in_the_fight_while_anything_it_brought_lives_even_a_fighter` (mechanics) and,
with real data, `the_battle_ends_with_the_last_brought_unit_whatever_was_spawned_meanwhile` (galaxy-sim, pirate star base with its garrison).

## Hit points are the hardpoints (U47, user rule)
This is the user's damage model, not EaW data. A unit that has destroyable hardpoints (`HpDef::destroyable`: `Is_Destroyable` and health above 0) has **no hull pool of its own**: its hit points are the sum of its
hardpoints and it is destroyed when the last one is destroyed (a star base cannot die while a laser or shield generator lives, a ship with 100 hp and 100 weapons survives 99 destroyed ones). `Ship::hull`
stays as a derived figure for bars and decisions: `ShipDef::hull` times the share of hardpoint health that is left (never 0 while a hardpoint lives).
* **Shield first:** the shield is a second health bar over the whole unit. Projectiles that do shield damage only hit it until it is down (the part of a hit that breaks it carries over to the hardpoints); projectiles without
  shield damage (missiles, proton torpedoes: `ProjDef::shield_damage` is false in the data) bypass it. Unchanged from before.
* **Targeted fire** (`AttackHardpoint`, `aim_hp`) hits the chosen hardpoint; **untargeted** fire hits the living destroyable hardpoint nearest to the impact point (in the ship's frame), not a separate hull pool.
* **Overkill spills:** damage beyond what the hit hardpoint had left goes on to the next nearest hardpoints, **halving at every step**: the overkill of #1 halved is the hit on #2, the overkill of #2 halved is the hit on #3, and so on
  until it is below 1. All numbers are rounded down to whole numbers (the incoming hit is rounded first). A hit that does not destroy its hardpoint spills nothing. (Reading of "100,000 on #1, then 50,000 on #2, 25,000 on #3": the
  spill is what exceeded the previous hardpoint, halved; the user's example ignores the hardpoint health. Easy to change in `damage_ship`.)
* **Units without destroyable hardpoints** (fighters, bombers, small craft) keep a plain hull that dies at 0.
* Effects of a dying hardpoint are unchanged (shield generator down -> shield 0, engines -> speed, weapon stops firing, `Event::HardpointDestroyed`).
Tests: `a_unit_with_hardpoints_dies_when_the_last_hardpoint_is_destroyed_not_by_a_hull_pool`, `overkill_spills_to_the_next_hardpoints_halving_every_step_in_whole_numbers` (mechanics).

## Missile launch arcs (estimate)
Homing missiles (`Projectile_Category` MISSILE with `Max_Rate_Of_Turn` > 0) no longer leave the launcher on the straight target line:
the launch heading is rotated left or right (drawn per shot from the battle seed, deterministic) by
`min(0.6 rad, 0.17 x Max_Rate_Of_Turn)` x 0.6..1.0, and the existing homing turn brings the missile in, which gives the curved arc of
the Remake (Victory Destroyer missiles). The data has no launch angle for missiles, so both factors (`MISSILE_ARC_PER_TURN_RATE`,
`MISSILE_ARC_MAX` in `defs.rs`) are estimates. Damage, blast, hit and interception rules are unchanged. Real-data check
(`real_data.rs`, 2 Victory vs 2 Star Destroyers): 66% of the missiles deviate more than 10 units from the launch-to-impact line
(44% without the offset) and 77% hit or are intercepted before their range runs out (82% without). Rockets (`ROCKET` category,
`Projectile_Rocket_Curve_*` fields, `Max_Rate_Of_Turn` ~0) are unguided and keep their straight flight; the curve fields are not read yet.
The viewer draws missiles at the simulated position and velocity, so the trails follow the arc.

## Behaviour constants added in U52, U60-U66 and U93 (what is data, what is estimate)
The Remake XML has no numbers for these movement and doctrine details; every value below is an **estimate**, kept in `sim.rs` / `defs.rs`, and only shapes how ships move, never damage, health, range or speed from the data.
Squadron formation slots are data (`Squadron_Offsets`, `Starfighter_Formations.xml`); only the fallback `(-20 x i, 25 x (i % 2))` is an estimate. Fighter and bomber squadrons fly that formation widened (row "Squadron spread" below).

| Item | Value | Why it is an estimate |
|---|---|---|
| Engine hardpoint effect (U52) | speed x `0.20 + 0.80 x alive/total engines` (was a 0.25 floor) | the data has `HARD_POINT_ENGINE` hardpoints but no rule for what losing them does |
| Synthesised engine hardpoint (U52) | ships with destroyable hardpoints and none of type engine get one 0.75 radii astern, health 8% of their destroyable hardpoint health clamped to 1000..3000 | some capitals (e.g. Star Destroyer) have no engine hardpoint in the XML |
| Dogfight breakout (U60) | squadrons leave formation within 650 u of fighters/bombers, close-run distance `max(70 + slack, 180)`, slot blend radius 80 u (60%), speed catch-up 1.5 u/s per u behind, 2.0 per u ahead | engine steering is not in the data |
| Broadside approach (U61) | curve starts at 2.2 x stand-off, tangential offset 0.45 x stand-off, stop distance `max(2.5 x braking + 1.5 x radius, 400)` | the data gives gun arcs (used) but not how the AI manoeuvres |
| Bomber attack run (U62) | x1.35 speed and x1.5 acceleration while egressing, climb 0.7 x speed, hardpoint order shield generator, engine, fighter bay, weapon | engine behaviour; the order follows the in-game description of bombers, not a data table |
| Escort avoidance (U65) | corvettes and frigates keep capital hull + own radius + 150 u, repulsion weight 3.5, slow-down inside +50 u; capitals never yield to escorts | collision handling is not in the data |
| Interceptor doctrine (U66) | **replaced by data**: the A-Wing has `Targeting_Priority_Set` `Fighter` (Bomber 1.0, Fighter 2.0), the X-Wing the `Bomber` set, so interceptors prefer bombers through the normal priority weights; the name list and the x0.15 are gone | test `fighter_vs_bomber_targeting_follows_the_priority_set_of_the_unit` (real_data) |
| Carrier screen (earlier) | fighters near their carrier prefer bombers within 3500 u of it, x0.3 | estimate, already labelled in the code |
| Flanking (U93) | corvettes and units faster than 250 u/s swing 3200 + 600 x (id % 3) u (attack-move) or 3500 + 500 x (id % 4) u (move) to the side | the Remake has flanking behaviour but no distances in the XML |
| Squadron spread (2026-10-17) | fighter and bomber squadrons widen `Squadron_Offsets` in the plan until the closest members are 100 u apart (`SQUADRON_GAP`, at most x5 `SQUADRON_SPREAD_MAX`; places on one spot are pushed apart), led by the member nearest the middle; launched ones open out during the flyout; a leader slows down while a member lags (half speed at 2.5 gaps); a fighter at rest keeps its heading | the Remake keeps TIEs about two fighter widths apart (user's video b0703s: 36-51 px at the default camera, about 100 u) while the data offsets are 20 u; the data has no spacing or steering numbers |
| Launch out of the hangar (2026-10-17) | fighters leave along the bay bone's X axis (data: the hangar opening) until 40 u clear of the hull (`LAUNCH_CLEARANCE`), the rest of `Fighter_Bay_Flyout_Distance` level; an axis that does not lead out within 150 u (`LAUNCH_EXIT_MAX`; the Star Destroyer's runs along the belly to the bow) drops them below the hull first; a fighter under or over its own carrier keeps clear of the hull's height until it is out from under it, inside that height the carrier counts in the fighter keep-out; a squadron member without a target flies at its leader's height, its slot is a place in the plane | the data gives the bay bones and the flyout distance, not the clearance; 40 u is half the size a fighter is drawn at the default camera |
Visual-only effects of U67-U74 (ribbons, debris, glow, ripples, sensor rings, hyperspace tunnel) do not change simulation state.
The 5 s hold of the victory/defeat banner before the summary (U49) is the user's rule (`OUTCOME_HOLD_SECONDS` in `battle-view`), not an engine value.

## Rocket weave (estimate)
`ROCKET` projectiles with `Projectile_Rocket_Curve_Distance` > 0 weave: after `Projectile_Rocket_Straight_Distance` of flight their heading deviates
by `Projectile_Rocket_Curve_Offset` degrees following one full sine over `Projectile_Rocket_Curve_Distance`, ending on the original line (side
fixed per shooter and target). The data gives the three numbers, not the curve; the shape and the per-shooter side are estimates. Rockets keep
their small `Max_Rate_Of_Turn` homing. Offset 0 (flak, Executor barrage) flies straight.

## Death Star II in a space battle
Data (`SPACEUNITSSUPERS.XML`, `Hardpoints_Empire_Death_Star.xml`): `Death_Star_II` is `In_Background`, has a `SUPER_LASER` ability (`Recharge_Seconds` 45,
`Expiration_Seconds` 25) and a hardpoint `HP_Death_Star_Superlaser` that is a plain laser weapon firing `Proj_Death_Star_SRI` (99999 damage) and names the
same projectile as `Blast_Ability_Fire_Projectile_Type`.
* The hardpoint is the ability's weapon, so it never fires on its own (a hardpoint with `Blast_Ability_Fire_Projectile_Type` has no automatic weapon).
* `In_Background` ships are not targeted and projectiles pass through them (`ShipDef::background`).
* **Countdown (estimate).** The first shot waits for the approach: `TSW_Start_Distance_From_Target` 80000 to `TSW_Attack_Distance_From_Target` 50000 at
  `Max_Speed` 5. The data gives no scale from `Max_Speed` to units per second. Remake video Bqc5Ei6gF8Y (frames 5 s apart, accelerated game speed): the gauge
  reads 390.21 at f0033 and 184.08 at f0057, and the battle started about 27 frames before f0033; at roughly 1.7x game speed that puts the start near
  600 game seconds, so `TSW_SPEED_SCALE` = 10 gives 30000 / 50 = 600 s. This is an estimate; it only moves the first shot, not later recharges.
* The ability starts on that cooldown (`SuperLaserDef::initial_cooldown`); the gauge in `battle-view` shows `max(effect left, cooldown)`.
