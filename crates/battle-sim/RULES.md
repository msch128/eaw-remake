# Tactical space battle rules used by `battle-sim`

Everything is derived from the Remake 4.0 data (`Data/Xml`) plus the `.alo` models for hardpoint
positions. Where the engine (closed source) defines a rule that is not visible in the data, the value
is an **estimate** and marked as such; estimates live in `BattleConfig` or in `defs.rs`.

## 1. Units, space and time

* Fixed step `dt` = 1/30 s. Battle space: X forward at yaw 0, Y left, Z up; ships move in the XY plane.
* XML speeds (`Max_Speed`, projectile `Max_Speed`) are per frame in the data and multiplied by
  `Catalog::speed_scale` = 20 to get units per second (estimate; gives Star Destroyer 60 u/s, laser bolts 700 u/s,
  turbolasers 900 u/s). `Max_Rate_Of_Turn` is used as radians per second. Acceleration times by size class are estimates
  (fighter 1.5 s, corvette 4 s, frigate 6 s, capital 10 s, super 16 s to reach full speed).
* Model space (`.alo`) is Z-up with the ship's bow along -Y; it maps to battle space with `(x, y, z) -> (-y, x, z)`.
  Verified with the Star Destroyer: the bow turbolaser `HP_ISD_1_New_TL_05` ends up ahead (+X) and the shield
  generator dome behind (-X).

## 2. Ships, hardpoints, projectiles

| Rule | Data |
|---|---|
| Hull `Tactical_Health`, shield `Shield_Points` and `Shield_Refresh_Rate` per second | unit XML (`Units/Space/*.xml`), star bases `Buildings/Space/GC/**`, structures |
| `Armor_Type` / `Shield_Armor_Type` and the 673-entry `Damage_To_Armor_Mod` matrix scale hull and shield damage | `Xml/Gameconstants.xml` |
| Hardpoint position and firing direction = `Fire_Bone_A` (else `Attachment_Bone`) in the `.alo`; firing direction is the bone's local +X axis | `Units/Space/Hardpoints/*.xml`, `Art/Models/*.ALO` via `alamo-model` |
| Cone: `Fire_Cone_Width` / `Fire_Cone_Height` are full angles in degrees around the bone direction (yaw / pitch) | hardpoint XML |
| Range `Fire_Range_Distance`, `Fire_Min_Range_Distance` (measured to the target's bounding sphere) | hardpoint XML |
| Volley: `Fire_Pulse_Count` pulses `Fire_Pulse_Delay_Seconds` apart, then a random recharge between `Fire_Min/Max_Recharge_Seconds` | hardpoint XML |
| `Fire_Inaccuracy_Distance` per target class is added as aim error | hardpoint XML |
| `Fire_Category_Restrictions`: weapon may only target those classes | hardpoint XML |
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

* Every 0.4 s a ship scans enemies and keeps a candidate list sorted by priority class and distance (fighters prefer
  fighters/bombers, bombers prefer capitals/frigates/structures, everyone else prefers big ships, structures and
  corvettes over bombers over fighters; the `Targeting_Priority_Set` tables are not parsed). Each weapon fires at the
  first candidate that is in range, inside its cone and allowed. Heavy guns (damage >= 150) and shield-only weapons
  never fire at fighters or bombers.
* Large ships close to the damage-weighted mean weapon range (x0.9) and hold there, turning their bow towards the target
  if they have a facing/narrow-cone weapon. Defender ships with a star base `Guard` the planet; attackers `AttackMove`.
* Orders: `Move`, `AttackMove`, `Attack(ship)`, `AttackHardpoint(ship, hardpoint)` (aims at the hardpoint and the hit
  routes to it), `Stop`, `Retreat`, stances Aggressive / Defensive / HoldFire.
* No ship-ship collision or avoidance, no formation keeping for capital ships.

## 5. Abilities

`Unit_Abilities_Data` entries with an `Expiration_Seconds` become abilities with the `Mod_Multiplier` values of the
data: `SPEED_MULTIPLIER`, `SHIELD_REGEN_MULTIPLIER` (negative values are clamped to 0), `WEAPON_DELAY_MULTIPLIER`
(applied to the recharge, below 1 = faster) and `TAKE_DAMAGE_MULTIPLIER`. `FULL_SALVO` uses the per-hardpoint
`Full_Salvo_Weapon_Delay_Multiplier`, `POWER_TO_WEAPONS` additionally the per-hardpoint
`Power_To_Weapons_Mode_Weapon_Delay_Multiplier`. `Recharge_Seconds` starts after the effect ends. AI sides trigger
weapon boosts when something is in range. Not implemented: tractor beam, missile shield, interdiction, stealth,
sensor jamming, and every other ability type.

## 6. Reinforcements, unit cap, retreat, victory

* `Command::Reinforce` adds ships at the side's spawn after `reinforce_delay` (5 s = `Skirmish_Reinforcement_Delay_Frames` 150 at 30 fps)
  if `population` of everything in play plus the request is within `unit_cap` (150, the `Space_Tactical_Unit_Cap` value
  of the faction XML). Reinforcement points and the reinforcement zone are not modelled.
* Retreat becomes available after `Space_Retreat_Allowed_Countdown_Seconds` (60 s). Retreating ships fly away from the centre and
  leave the battle area; they count as survivors. `Space_Retreat_Attrition_Factor` is 0 in the data.
* A side loses when it has no ship other than fighters/bombers left (a fighter-only side loses when its fighters are gone).
  The loser's remaining fighters are destroyed with it. Battles are a draw after `max_time` (30 min).
* Setup places fleets in rows by size class, attackers at -X, defenders near the planet at +X. The real space map
  (`.ted`) bounds and obstacles (asteroids, nebula, the planet) are not read; the arena is a square of `half_size` = 10000.

## 7. Performance design

Per step: snapshot of positions and a uniform grid (500 u cells, ship bounding boxes inserted in all cells they overlap),
then rayon-parallel target scans, movement, weapon updates (fire requests) and projectile flight/collision. Hit
application, upkeep and spawning are sequential and in index order. Randomness comes from per-entity generators seeded by
`(seed, tick, id)`, so the result is identical with or without threads (tested).
