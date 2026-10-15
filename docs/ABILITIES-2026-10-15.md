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
| REPLENISH_WINGMEN | 10 | missing | no | - | next: respawn lost wingmen of the hero squadron |
| INTERDICT | 7 | done (fixed) | yes (hand icon is the real `I_SA_INTERDICT`) | rings (estimate) | see below |
| WEAKEN_ENEMY | 6 | missing | maybe | - | area effect, radius 2000, spawned object |
| HARMONIC_BOMB | 6 | missing | - | - | several uses are commented out in the data |
| SUPER_LASER | 5 | done | yes | beam exists | |
| LURE | 4 | missing | - | - | pulls listed target types |
| SPRINT, LUCKY_SHOT, ENERGY_WEAPON, CLUSTER_BOMB, BLAST | 3 each | SPRINT done, others missing | - | - | hero abilities with sub-ability objects |
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
