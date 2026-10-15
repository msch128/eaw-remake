# Space abilities audit, 2026-10-15

Source: Remake 4.0 `Unit_Abilities_Data` of all space units (`Units/Space`, `SpecialStructures`, hero files). Counts are objects
(variants and squadron twins included), most common first. "Sim" = rules in `battle-sim`; "Card" = button on the command bar;
"Visual" = effect in `battle-view`. Visuals were not checked in a window in this pass (marked "unchecked").

| Ability | Objects | Sim | Card | Visual | Note |
|---|---|---|---|---|---|
| HUNT | 81 | done (new) | done (new) | none in data | toggle, see below |
| TURBO | 55 | done | yes | unchecked | speed/weapon-delay multipliers from data |
| FULL_SALVO | 42 | done | yes | unchecked | per-hardpoint delay |
| SPOILER_LOCK | 33 | done (new) | done (new) | none in data | toggle, speed x1.5, delay x3, shield regen 0; icon `I_SA_S_FOIL_MODE` |
| POWER_TO_WEAPONS | 28 | done | yes | unchecked | |
| STEALTH | 19 | done | yes | unchecked | |
| SENSOR_JAMMING | 18 | done | yes | unchecked | |
| LASER_DEFENSE | 17 | done | yes | unchecked | |
| DEFEND | 17 | done | yes | unchecked | |
| ROCKET_ATTACK | 16 | done | yes | unchecked | reload factor is an estimate |
| BUZZ_DROIDS | 11 | done | yes | unchecked | |
| TRACTOR_BEAM | 10 | done | yes | beam unchecked | |
| FOW_REVEAL_PING | 10 | done | yes | ping ring | |
| REPLENISH_WINGMEN | 10 | done (new) | yes (icon is an estimate) | - | once per battle, auto when no wingman is left; the "leader only hurt while wingmen live" rule is not modelled |
| INTERDICT | 7 | done (fixed) | yes (hand icon is the real `I_SA_INTERDICT`) | rings (estimate) | see below |
| WEAKEN_ENEMY | 6 | done (new) | yes | none | area radius 2000, 60 s, 25% less damage dealt, 25% slower; button uses the caster position until the click-to-aim hook in tactical_ui knows `AbilityDef.weaken` |
| HARMONIC_BOMB | 6 | done (new, active uses only) | button via `Alternate_Icon_Name` | none | carriers drop a +200 hull blast (radius 750, fighters and bombers); Slave I drops a seismic bomb after 5 s (radius 300, 200 damage, 3 falloff tiers); the commented-out uses in the data are ignored |
| SUPER_LASER | 5 | done | yes | beam exists | |
| LURE | 4 | done (new) | yes | none | enemies of the listed types within the radius are ordered to attack the caster, caster takes the ability take-damage multiplier |
| SPRINT | 3 | done | yes | - | |
| ENERGY_WEAPON ("Proton Beam") | 3 | done (new) | yes | none | beam on one Capital/Corvette/Frigate within 2500, 500 per frame x 30 fps (estimate) for 1.5 s, hull only |
| CLUSTER_BOMB | 3 | done (new) | yes | none | 14 ion bombs on a ring (speed x delay from the projectile), 4500 shield damage each in radius 600, no hull damage; ring placement is an estimate |
| LUCKY_SHOT, BLAST | 3 each | not done | - | - | the data gives no numbers (a heavy projectile override, "two very powerful blasts" after 5 s of charge); needs a decision on values |
| CORRUPT_SYSTEMS, MISSILE_SHIELD | 2 / 1 | done | yes | unchecked | |
| SHIELD_FLARE, LEECH_SHIELDS, INVULNERABILITY, CONCENTRATE_FIRE, SWAP_WEAPONS | 1 each | CONCENTRATE_FIRE, SWAP_WEAPONS done; rest missing | - | - | |
| Ground-only (STICKY_BOMB, TARGETED_*, FORCE_*, CAPTURE_VEHICLE, AREA_EFFECT_*, DEPLOY_TROOPERS, EJECT_VEHICLE_THIEF, FIRE_LOBBING_SUPERWEAPON) | - | not space | - | - | out of scope here |

## INTERDICT (gravity well)

Data: toggle (no expiration), `Disable_Movement`, tooltip "Prevent any enemy units from retreating to hyperspace". No radius, no
particle. Immobilizer, Dominator, Eclipse, Sith Interdictor, Glovoc; `Is_Interdictor` stations.
Lua (`ObjectScript_Interdict_Layer_Z`): the AI switches it on while the enemy is retreating and off again after 20 s;
`RetreatPlan.lua` says "Retreat was aborted" when interdicted.

Sim: enemies cannot order a retreat while a field is up (existed); new: a field that comes up during the jump countdown cancels
the retreat (`Event::RetreatAborted`); AI interdictors do the 20 s routine; the interdictor itself cannot move (existed).
Not done: stations with `Active_By_Default` in `SpecialStructures` (E_Gravity_Well_Station) have no abilities loaded, and the
`HARD_POINT_GRAVITY_WELL` hardpoint rule of vanilla (station field dies with the hardpoint). The data has nothing about blocking
reinforcements, so none is blocked.
View: pulsing violet rings around active interdictors (size is an estimate).

## HUNT ("Hunt for Enemies")

Data: toggle without numbers, tooltip "Scout the map for hostile targets. If enemies are detected, this unit will attempt to
destroy them." Fighters, bombers, corvettes, gunships.
Sim: every hunter leaves its formation and picks the visible enemy its `Targeting_Priority_Set` ranks best, then nearest, with a
penalty per other hunter already on it (spreads over the fleet); stations only when nothing else is visible. It chases until the
target dies. With nothing visible it searches an 8x8 grid of the map, nearest unsearched cell first, 60 s per cell at most. A
Move/Attack/Stop order or toggling off ends it. Estimates (labelled in `sim.rs`): weights, crowd penalty, cell time.
Not done: AI sides do not use Hunt (they have their own search).

## Card icons

`I_SA_<TYPE>.TGA` does not exist for TURBO, DEFEND, FOW_REVEAL_PING, MISSILE_SHIELD, SUPER_LASER, CONCENTRATE_FIRE, REPLENISH_WINGMEN and
ENERGY_WEAPON, and most units do not name an `Alternate_Icon_Name`, so their cards were skipped (59 of 65 TURBO objects). `eaw-ui`
now falls back to the closest existing icon (estimates, see `ability_icon`).

## Gravity well station

`E_Gravity_Well_Station` has `Is_Interdictor`; structures now read it, so the field is always up.
