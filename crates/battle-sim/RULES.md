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
* Retreat becomes available after `Space_Retreat_Allowed_Countdown_Seconds` (60 s). Retreating ships fly away from the centre and
  leave the battle area; they count as survivors. `Space_Retreat_Attrition_Factor` is 0 in the data.
* A side loses when it has no ship other than fighters/bombers left (a fighter-only side loses when its fighters are gone).
  The loser's remaining fighters are destroyed with it. Battles are a draw after `max_time` (30 min).
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
