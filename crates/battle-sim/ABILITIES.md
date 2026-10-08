# Ability audit (Remake 4.0 space units)

Counts are `Unit_Ability` entries in `Units/Space/*.xml`, `SPACEUNITS*.XML`, `NAMEDHEROUNITS.XML`; sub-abilities are the `Abilities` children.

## Implemented
| Type / sub-ability | Uses | How |
|---|---|---|
| `TURBO`, `SPRINT`, `SPOILER_LOCK`, `DEFEND`, `POWER_TO_WEAPONS`, `INVULNERABILITY` and any entry with `Expiration_Seconds` | 56 / 3 / 35 / 17 / 24 / 1 | `Mod_Multiplier` values (speed, shield regen, weapon delay, take damage) while active, recharge afterwards; AI uses them by situation |
| `FULL_SALVO` | 34 | per-hardpoint delay multipliers |
| `TRACTOR_BEAM` | 10 | slow the target, range and categories from `Tractor_Beam_Attack_Ability` |
| `MISSILE_SHIELD`, `INTERDICT` | 1, 5 | toggles (destroy missiles in radius; block enemy retreat, pin the ship) |
| `STEALTH` + `Stealth_Ability` | 17 + 10 | toggle (or timed); hidden from enemies unless within `BattleConfig::stealth_detection` (500, estimate) |
| `CORRUPT_SYSTEMS` (Corellian Destroyer of the Rebels, IG-88 of the Underworld) | 2 | `Corrupt_Systems_Ability`: `Activation_Max_Range`, `Duration_In_Seconds`, `Shield_Drain_Per_Second`, `Inaccuracy_Radius` | `Command::UseAbilityAt` at an enemy ship with a shield, inside the range and the pick radius of the chosen point: its shield drains by the rate for the duration (a drained shield stays down the usual 4 s). The AI uses it on the enemy with the most shield in range |
| `FOW_REVEAL_PING` | 10 | `Command::UseAbility[At]`: shows `Effective_Radius` around the ship or the chosen point for 5 s (estimate) |
| `LASER_DEFENSE` + `Laser_Defense_Ability` | 18 + 19 | like the missile shield for `Defense_Duration_In_Secs` (30) in `Effective_Radius` (750), ship keeps moving |
| `Force_Healing_Ability` (`Space_Automatic`) | 64 | `HealDef`: hull `Heal_Amount` + `Heal_Percent` of max per `Heal_Interval_In_Secs`, on the ship or on friends in `Heal_Range` (single target = most damaged) |
| `Combat_Bonus_Ability` (`Space_Automatic`) | 50 | `BonusDef`: while the ship lives, friendly ships of `Applicable_Unit_Categories` get `Damage_Bonus_Percentage` (damage dealt), `Health_Bonus_Percentage` / `Shield_Bonus_Percentage` / `Defense_Bonus_Percentage` (damage taken divided by 1 + bonus), `Fire_Range_Bonus_Percentage` (hardpoint reach) and `Movement_Speed_Bonus_Percentage`. The data has no radius, so the aura covers the whole side (estimate). Per `Stacking_Category` the strongest bonus applies, categories add up; negative values (Gorgon -40% shields) are maluses. Refreshed once per second |
| `Redirect_Blaster_Ability` (not `Initially_Enabled` = No) | 50 | `RedirectDef`: a direct shot hitting the ship inside `Reaction_Arc_In_Degrees` is destroyed with `Block_Chance`, or sent back with `Redirect_Chance` (estimate: its damage is dealt to the shooter instead of re-aiming the bolt). Abilities that start disabled are enabled by story or upgrades and are skipped |
| `ROCKET_ATTACK`, `SWAP_WEAPONS` | 21, 1 | recharge, alternate icon only | for the duration (10 s where the data gives none) missile, rocket and torpedo hardpoints reload at 0.4x delay (`ROCKET_RELOAD_MULT`, estimate); lasers are unchanged. `Command::UseAbility` |
| `SENSOR_JAMMING` | 13 | `Expiration_Seconds` 15 | while active, enemy homing missiles aimed at the ship lose their lock and fly on straight (the 75 `Effective_Radius` is not used). `Command::UseAbility` |
| `BUZZ_DROIDS` + `Buzz_Droids_Ability` | 15 | `Damage_Radius`, `Enemy_Damage_Per_Second`, `Activation_Max_Range` | only variants that hurt enemies (Eclipse lightning storm: 5000 radius, 250 dps): an area placed at the chosen point (`Command::UseAbilityAt`, clamped to max range; without a point it sits on the ship) that damages enemy ships inside, shield first, for the ability duration (20 s where the data has none, estimate). `Own_Damage_Per_Second` 1 and the repair-drone / jamming-pod variants (no enemy damage) have no effect |
| `SUPER_LASER` + `Super_Laser_Ability` | 5 + 6 | `Fire_Frame` 50, `Damage_Per_Frame` 10, categories Capital / Frigate / SpaceStructure | `Command::UseAbilityAt` on a point: the nearest enemy ship of those categories within 600 units is destroyed outright after `Fire_Frame`/30 s (estimate; the 10 damage per frame cannot be the one-shot kill). Fails with `AbilityUnavailable` if no valid ship is there. The galactic planet shot is in galaxy-sim |

## Not implemented
| Type / sub-ability | Uses | What the data says | Effect that is missing |
|---|---|---|---|
| `HUNT` | 78 | no parameters | stance toggle: the aggressive stance already hunts; no separate state |
| `REPLENISH_WINGMEN` | 10 | only in the unused `_BU` backup files | nothing: no loaded unit has it |
| `HARMONIC_BOMB`, `CLUSTER_BOMB`, `BLAST`, `STICKY_BOMB`, `ENERGY_WEAPON` | 5, 3, 3, 2, 3 | bomb / beam sub-abilities | area damage, beam attacks, bombs that stick to a target |
| `LURE`, `WEAKEN_ENEMY`, `SHIELD_FLARE`, `FORCE_CONFUSE`, `CORRUPT_SYSTEMS`, `CONCENTRATE_FIRE` | 4, 1, 1, 1, 1, 1 | hero / special abilities | taunt, damage debuff, shield burst, confusion, hardpoint disabling, focus fire |
| `TARGETED_REPAIR`, `TARGETED_INVULNERABILITY`, `TARGETED_HACK` | 1 each | targeted hero abilities | repair, invulnerability and hack a chosen ship |
| `Retreat_Prevention_Ability`, `Bounty_On_Faction_Ability` | 2, 1 | | other than the interdictor |
| `CAPTURE_VEHICLE`, `AREA_EFFECT_STUN`, `AREA_EFFECT_CONVERT` | 1 each | ground abilities | not space |

Galactic abilities (`Planet_Income_Bonus_Ability`, `Galactic_Stealth_Ability`, `System_Spy_Ability`, `Planet_Destruction_Ability`, `Reduce_Production_*`) belong to galaxy-sim.

## Automatic use (autocast)
`Unit_Ability` has `Supports_Autofire` (89 of 96 entries say True; tractor beam, laser defense, sensor jamming, defend, rocket attack ...) and `Active_By_Default`. `AbilityDef::supports_autofire` / `autocast_default` read them; `AbilityState::autocast` starts as `supports_autofire && autocast_default` (the 11 `Active_By_Default` entries are all in the unused `_BU` files, so every ability starts off). `Command::SetAutocast { ship, ability, on }` switches it (refused for abilities without `Supports_Autofire`). A player ship with autocast on uses that ability with the same rules as an AI ship (the checks in the per-ship loop and in `ai_special_abilities`); ships of AI sides use all abilities as before. `battle-view` exposes it as `ShipSnap::autocast` (same order as `abilities`).
