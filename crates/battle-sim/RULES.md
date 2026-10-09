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
* Large ships close to the range that puts 80% of their weapon damage in reach (x0.95) and hold there, turning to the
  bearing (bow, broadside or stern) at which the largest share of their hardpoint damage can bear on the target
  (`ShipDef::bearing_dps`, computed from the arcs). Defender ships with a star base `Guard` the planet; attackers `AttackMove`.
* Big ships stop at that range with an arrival speed profile (no overshoot), start turning to the weapon bearing in the last stretch of the approach, and position themselves against the best non-fighter target (fighters and bombers are fired at but not chased, unless explicitly ordered). Restart hysteresis 1.12 x standoff (estimate).
* Orders: `Move`, `AttackMove`, `Attack(ship)`, `AttackHardpoint(ship, hardpoint)` (aims at the hardpoint and the hit
  routes to it), `Stop`, `Retreat`, stances Aggressive / Defensive / HoldFire.
* No ship-ship collision or avoidance, no formation keeping for capital ships.

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
  defender: enemy >= 3 x own (4 x with a living station) and age > 120 s (> 30 s when it has no station). Also required: the side's `BattleConfig::retreat_allowed` (`Allowed_To_Retreat_Space`:
  campaign only, faction holds more than one planet, never Pirates; galaxy-sim sets it per `TacticalSide::may_retreat`), the retreat countdown (60 s) passed, and no enemy interdiction
  (`RetreatBlocked` is simply retried). The thresholds and ages are data; "force" is an **estimate**: sum over living ships of `sqrt((hull + shield) x best-bearing damage rate)` (the engine's
  `FriendlyForceUnnormalized` is not visible), reinforcements are not counted (`ReinforcementsUnnormalized` x 0.8 is ignored), the "mainly transports" clause and the Hard-difficulty
  "finish low-health enemies before leaving" step are not modelled. Retreating ships must still fly out of the arena, so a late order saves only the fast or distant ships.
* **Focus weak (`Prevent_Retreat_On_Priority_Target`, implemented):** a non-fighter enemy with less than 33% of hull + shield left (data threshold) inside weapon reach gets its priority weight halved
  (factor 0.5 is an estimate), so ships finish off wounded targets instead of spreading fire.
* **Ability use (implemented generically from the ability numbers, not names):** besides the weapon boosts, tractor beams and missile shields of section 5, AI ships now use expiring abilities
  with `speed_mult > 1.05` when fleeing or when the target is more than twice the weapon range away while on a move order, `shield_regen_mult > 1.05` when the shield is below 50% and
  was hit in the last 3 s, and `take_damage_mult < 0.95` when the hull is below 60% under fire (the percentages are estimates; the data does not list AI hints for these).
* **Separation (new, estimate):** mobile non-fighter ships push each other apart when closer than 0.85 x the sum of their (frigate+ shrunk) radii, half of the overlap each, at most one
  tick of speed per tick; stations are excluded (defenders guard the station position). Measured with 8 ISD vs 6 MC-80 + 6 Nebulon-B: worst overlap 16% of the radii (before: ships stacked).
* **Already data-driven before:** target priorities by `Targeting_Priority_Set` (counter types, fighters vs bombers/capitals), hardpoint priorities (shield generator, engines, hangar, weapons) for
  attack-move sets (off by default, see section 4), squadron launch and replacement from carriers/stations (section 3), defenders guarding their station, attackers attack-moving.
* **Not implemented:** Flank/Escort/Bombing-run goals as separate behaviours (bombers use their priority sets), AI-called reinforcements (`Command::Reinforce` needs a reinforcement pool
  from the galaxy), the Death Star, stealth/cloak (`Try_Ability STEALTH`), and formation keeping for capital ships.

### 9b. Bombing runs, intercept, formations (new; all numbers are estimates, the data has no AI values for them)
* **Bombing run:** a `Bomber` that reaches a frigate-or-larger target (ships and stations) flies straight on for 3-5 s (turn at most +-0.15 rad/s) before turning back for the next pass; against small targets and for fighters the old 1-2.5 s random break-away stays. Test: a bomber makes repeated passes on a static capital.
* **Intercept:** a `Fighter` with a carrier prefers (weight x0.3 within its priority set) enemy bombers that come within 3500 units of that carrier. **Escort** is the existing carrier/squadron-leader following; bombers get no extra escort behaviour. The intercept rule has no dedicated test.
* **Formations:** each fleet unit remembers its lateral position in the start line (`Ship::lane`, +-1500 max). Under attack-move orders, while the enemy is more than 2500 units away, the aim point is shifted sideways by that lane, so the fleet advances line abreast; broadside arcs still come from the bearing logic (section 4). While no enemy is within twice the weapon range a big ship does not outrun slower friendly big ships within 2500 units (cohesion, never below 40% of its own speed).
  Test: 3 fast + 3 slow frigates are 25 units apart in depth after 25 s (2450 without cohesion) and still 1400 units wide.
* **Ability rules** (section 9) now have a unit test (`ai_uses_speed_shield_and_damage_abilities_by_their_numbers`).

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
* `navigation: false` gives the old behaviour for comparisons (`galaxy-sim/examples/navcheck`: Bilbringi 61 rocks and Hoth, 4 Star Destroyers + 3 Victory + fighters vs MC-80, Nebulon, CR-90 + fighters, 300 s, three seeds: no hull overlaps after the opening, no ship inside a rock, no planned path through a rock; fighters deep inside a hull 307 / 177 / 319 / 372 samples before, 0 after).
