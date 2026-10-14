# Galactic conquest rules used by `galaxy-sim`

Source of truth: the Remake 4.0 data (`Data/Xml`, `Data/Scripts` of workshop item 1770851727) and
the base game files layered below it. Where the data does not define a rule (the engine is
closed source) the value is an **estimate** and is marked as such; every estimate lives in
`SimConfig` so it can be tuned.

Paths below are relative to the mod's `Data` folder.

## 1. Time

* There are no "weeks" in the data. The simulation defines one week as `SimConfig::week_seconds`
  = 60 s (estimate, chosen so that a Star Destroyer, `Build_Time_Seconds` 70, takes about a week).
  Income is quoted per week but accrues continuously every tick (`tick_seconds` = 1 s).
* `Xml/Gameconstants.xml` has `Object_Max_Speed_Multiplier_Galactic` (5.0), `Fleet_Maintenance_Update_Delay_Seconds`,
  `Space_Retreat_Allowed_Countdown_Seconds` (60) and `Space_Capture_Allowed_Countdown_Seconds` (1), which are
  tactical-mode values and not used by the galactic layer here.

## 2. Economy

| Rule | Source | Implementation |
|---|---|---|
| Planet income = `Planet_Credit_Value` | `Xml/Planets_WW.xml` (e.g. Coruscant_WW 607) | per week per owned planet |
| Structures and star bases change planet income through `Planet_Income_Bonus_Ability` (`Absolute_Income_Modifier`, `Percentage_Income_Modifier`) | e.g. `Buildings/Space/GC/Shipyard_*.xml` (upkeep -35), `Buildings/Ground/GC/**/Mining_*.xml` (+25) | `World::planet_income`: `base * (1 + sum(pct)) + sum(abs)` |
| Ships cost upkeep through the same ability (Star Destroyer -800) | `Units/Space/Units_Space_Empire_Star_Destroyer_Imperial.xml` | summed over all ships of a faction, applied to faction income |
| Faction multiplier `Credits_Accumulation_Factor` | `Xml/FACTIONS.XML` (Rebel 0.95) | multiplies the faction's total income |
| Credit cap `Credit_Cap_Per_Planet` = 400000 per owned planet | `Xml/Gameconstants.xml` | off by default (user rule: credits never expire); `SimConfig::credit_cap_per_planet` > 0 enables it |
| `Political_Income_Curve` 0,0 1,1 5,1 and `Income_Redistribution` 0.6 | `Xml/Gameconstants.xml` | **ignored** (political control is always 1 for owned planets) |
| `Income_Stream_Ability` on mines | `Buildings/Ground/GC/**/Mining_Facility*.xml` | **ignored**: its comment says it only exists in tactical modes, not in the campaign |
| Starting credits, tech levels, home planets | `Xml/Campaigns_Imperial_Web_Way_*.xml` | `FactionInfo` |
| Black market, corruption, trade route `Credit_Gain_Factor` / `Political_Control_Gain` | `Xml/TradeRoutes_Web_Way.xml` | the Web Way lanes all have both at 0, so they only matter as hyperspace lanes |

Population: units have `Population_Value`; the cap is
`SimConfig::base_population_cap` (estimate, 40) plus, for every owned planet, the planet's
`Additional_Population_Capacity`, its star base's `Additional_Population_Capacity` and structure capacities.

## 3. Production

* Three lanes per planet, one item built at a time per lane: ships and research items (lane 0), structures
  (lane 1), star base and tech upgrades (lane 2). Cost is paid when queued (the real game charges progressively; estimate).
* Time = `Build_Time_Seconds * build_time_scale`. `Production_Speed_Mod_Base_Vs_Tech_*` are all 1.0 in the Remake, so no
  speed-up from star base level.
* A ship may be queued when all of these hold (`World::check_build_unit`):
  `Affiliation` contains the faction, `Tech_Level` <= faction tech, `Required_Star_Base_Level` <= the planet's base level,
  every `Required_Special_Structures` group is met (see below; a structure also satisfies the requirement of any object in its
  `Variant_Of_Existing_Type` ancestry), the planet is one of `Required_Planets` (any one listed; names that are not in the galaxy are ignored),
  the planet can hold a star base of the required level (`Max_Space_Base`; otherwise the item is hidden for this planet), not `Build_Initially_Locked` unless unlocked,
  `Build_Limit_Current_Per_Player` / `Build_Limit_Lifetime_Per_Player`, population, credits.
  Objects with `Tech_Level` >= 90 (story-unlockable, terrain-specific templates) are never offered.
* `Build_Initially_Locked` objects are unlocked by research items: completing `Research_<Name>`
  (see `Research/Units/_Default_Base.xml`, mirrored by `Scripts/Library/libResearchFunctions.lua`) unlocks `<Name>`.
* Structures (`SpecialStructure`): slot limits come from the planet's `Special_Structures_Space` (orbital) and
  `Special_Structures_Land` (ground), queued items take a slot already (`World::planet_capacity` -> `PlanetCapacity { space, ground, starbase_level, starbase_max }`,
  each `SlotUse { built, queued, total }`; Coruscant_WW: 7 orbital, 6 ground, station up to level 3); `Build_Max_Instances_Per_Planet`;
  `Required_Star_Base_Level` (shipyards need level 2), `Required_Special_Structures`, `Required_Planets` as for ships. Terrain variants
  (`*_Urban`, `*_Barren`, `*_Sector`) and one-planet buildings (Starforge, refineries, fortresses) are selected by `Required_Planets`.
* `Required_Special_Structures` syntax (checked against the XML, `catalog.rs::parse_requirement`): names separated by whitespace or commas are **all** required,
  names joined by `|` are **alternatives** (`Empire_Outpost_Urban | Imperial_Palace_Urban`), and both mix (`U_Shipyard_1 U_Trade_Hub | U_Gas_Refinery`).
  The refusal text lists every unmet group (`RequirementsNotMet("E_Shipyard_3, E_Uplink_Command")`, alternatives joined by ` | `); `galaxy-play` shows display names.
* One star base per planet (`Lane::Base`, levels chain up to `Max_Space_Base`); orbital buildings and ground buildings are separate slot pools.
  A planet's cap is its `Max_Space_Base` from the planet XML (all real planets of the Remake's `Planets_WW.xml` say 3, the dummy and icon planets 0 or 1); nothing is classified or guessed.
  Which planets may build which ships and shipyards comes from the structures' `Required_Planets` / `Required_Special_Structures` and the star base level they need (`Required_Star_Base_Level`), as in the data.
  A unit or structure whose required structures all need a star base level above the planet's cap is hidden from that planet's menu (`RequirementsNotMet("planet")`); the upgrade button is omitted at the cap (`this planet takes star bases up to level X`).
* `Required_Ground_Base_Level` only occurs on `GroundBase` objects (ground base levels, not modelled: no ground battles). `Required_Orbiting_Units`
  (hero upgrade items such as Piett's 25000 credit upgrade) is not enforced yet.
* Heroes: `Build_Cost_Credits` of the hero ship itself is **1** in the Remake data (e.g. `Piett_Procursator`, `Strikefast`); the real investment is the follow-up upgrade item
  (`Vessel_Upgrade_Template`, 25000+). The menu shows the data value.
* Ground structures only appear if they matter for the galactic game (`StructureType::galactic`, no ground battles): see `STATUS.md` "Build menu split".
* Star base levels are the chain `Prev_Level_Base` / `Next_Level_Base` (`Buildings/Space/GC/<Faction>/Colonies.xml`). Minor
  factions define their levels as unlinked objects (`Minor_Starbase_X`, `_2`, `_3`) which are chained by name and level.
* Tech levels: `*_Level_N_Tech_Upgrade` upgrade objects (`Upgrades/Vanilla*.xml`, flag `Tactical_Build_Increments_Tech_Level`,
  cost `Tactical_Build_Cost_Campaign`). Factions without such objects (minors) get generic upgrades costing 3000 x level (estimate).
* Newly built ships join the idle fleet of that faction at the planet, or create one.

## 4. Movement

* Lanes are the campaign's `Trade_Routes` between real planets (`Xml/TradeRoutes_Web_Way.xml`: `Point_A`, `Point_B`,
  `HS_Speed_Factor` 1-3 for capillary/inter/main lanes). Length is the Euclidean distance of `Galactic_Position` x/y.
* Fleet speed = slowest ship's `Hyperspace_Speed` (clamped 0.1-1.5) x lane `HS_Speed_Factor` x `galactic_speed_scale`
  (6.0, estimate) map units per second. Paths minimise travel time (Dijkstra). A fleet that passes a planet holding hostile
  fleets or a hostile star base is stopped there (blockade).
* `Retreat` sends a fleet back to the planet it came from; a fleet in hyperspace turns around.
* Escape after a lost tactical battle (retreated losers, attackers after a draw): the fleet flies the fastest path to the nearest friendly (own or allied) planet, passing other planets without a battle, and blockades do not stop it (`Galaxy::escaping`, cleared on arrival or a new move order). With no friendly planet left it takes the cheapest lane to the neighbour system (**estimate** for "the first route"; ties by lane index). `World::escape_path`.

## 5. Battles

* A battle is triggered when two hostile parties share a planet. Parties are the factions with fleets in orbit plus the
  owner of the planet if it has a star base or armed structures. All factions are hostile to each other (every faction's
  `Enemies` list in `Xml/FACTIONS.XML` / `Xml/Factions_Minors.xml` names the others).
* `Event::BattlePending` is emitted; with `auto_resolve_battles` the sim resolves it immediately, otherwise the app
  calls `resolve_battle_auto` (or runs a tactical battle and applies its own result).
* Auto-resolve (`combat.rs`) is a placeholder: every ship, star base, armed structure and star base garrison squadron
  (`Starting_Spawned_Units_Tech_0`) is a combatant with shield and hull from `Shield_Points` / `Tactical_Health`, and
  weapons aggregated per damage type from `HardPoint` (`Fire_Pulse_Count`, `Fire_Min/Max_Recharge_Seconds`,
  `Fire_Pulse_Delay_Seconds`) and `Projectile` (`Projectile_Damage`, `Damage_Type`, `Projectile_Does_*_Damage`). The
  damage-type versus armor-type matrix (`Damage_To_Armor_Mod`, 673 entries in `Xml/Gameconstants.xml`) scales hits against shield
  armor and hull armor. Hardpoint health, ranges, cones and speeds are not simulated. Rounds are 4 s; a fight
  with no damage dealt, or exceeding 600 rounds, is a draw and attackers retreat.

## 6. Planet capture without ground combat

The data contradicts itself on ground troops (the mod still ships `GroundCompany` objects and starting
`Trooper_*_Regiment` forces), so the rule is derived from how the Remake's own AI conquers planets:

* `Scripts/Ai/Conquer_Enemy_Space.lua` (category `GC_Conquer_Enemy_Space`) and `Conquer_Unrestricted_Space.lua` (category `GC_Annex_Neutral_Space`, `MainForce_Original_Target_Owner_Changed`) send a **space-only task force** (`SpaceForce`,
  categories `Fighter|Bomber|Corvette|Frigate|Capital`) to a target planet, call `Move_To`, and succeed when
  `EvaluatePerception("Planet_Has_Friendly_Space_Force_In_Orbit")` is true. The task then handles
  `SpaceForce_Original_Target_Owner_Changed`, i.e. the engine changes the planet's owner as a consequence of space control
  alone; no transport or ground unit is requested.
* Rule implemented: a planet changes owner after a single hostile faction has been the only party in orbit for
  `capture_delay_seconds` (5 s, estimate; the 1 s `Space_Capture_Allowed_Countdown_Seconds` is the tactical value) and
  the owner no longer has a star base. Consequently attackers must first destroy the star base, armed structures,
  garrison and defending fleets (a battle). Unowned planets are captured by any fleet arriving.
* On capture the star base, all structures and the build queues are removed (the real game destroys buildings too; some
  could survive in reality).
* Not modelled: `Planet_Surface_Accessible`, ground defenders, siege/bombardment (`Scripts/GameObject/ObjectScript_Bombardment*.lua`),
  planet shields, story scripts that flip owners (`Xml/SANDBOX_EVENT_PLANET_OWNERSHIP_FLIP.XML`).
* **Uncertainty:** I could not verify this against a running game; if the real engine requires a landing, `capture()` in
  `sim.rs` is the single place to change.

## 7. Starting state

* Planet owner: `GameData::campaign_planet_owners` (the faction that places a star base or structure there, see
  `alamo-data`); star base, structures and fleets come from the campaign's `Starting_Forces`. Ground units and heroes are ignored.
* Neutral planets (no starting structure) start unowned and undefended.

## 8. Victory and elimination

* `Human_Victory_Conditions`: `Galactic_Control_Named_Planets` (planets in `Human_Victory_Planet_Names`) and/or
  `Galactic_All_Planets_Controlled`, for the campaign's `Starting_Active_Player`. `AI_Victory_Conditions` apply to every other faction.
* A faction with no planets and no fleets is eliminated. The last non-eliminated faction wins.
* Victory can be switched off with `SimConfig::victory_enabled = false` (the headless example's `novictory`).

## 9. AI stub (`ai.rs`)

Every `ai_interval_seconds` (10 s) each AI faction, planned in parallel threads from an immutable snapshot and applied in
faction order: buys tech, builds/upgrades star bases, builds structures that unlock many ships (shipyards) or pay back, queues the
best durability x damage per credit ship, merges idle fleets per planet, and sends a fleet at the best-valued reachable enemy or
neutral planet whose defense (Lanchester estimate `sqrt(durability x dps)`) it exceeds by 1.5x.

## 7. Tactical results
A tactical battle (front end or `run_headless`) reports survivors per group; losses, the star base and structures are applied exactly like the
statistical result. Units that left the battle by `Retreat` count as survivors and, when their side did not win (or the battle was a draw and they
attack), their fleets escape along the escape route above (estimate: no attrition, `Space_Retreat_Attrition_Factor` is 0 in the data). Damage is
not carried over between battles (surviving ships are repaired), also an estimate.

## 8. Difficulty multipliers
`Sim::set_faction_difficulty(faction, Difficulty)` (from `alamo_data::DifficultyAdjustment`): positive income x `credit`, build times x `build_time`,
and in battles damage dealt x `damage`, damage taken divided by `health` (hull) / `shield`. Statistical results use them in `combat::resolve_allied`;
tactical battles get them as `TacticalSide::{damage,health,shield}_mult` and `BattleConfig::side_mults`. Default 1.0 for everyone.

## Heroes, discounts, capture bonus (see GC-GAPS.md)

* Hero respawn is the Remake's story chain (destroy -> 300 s -> SPAWN_HERO); nothing else in the sim respawns heroes.
* Production discount: per `Stacking_Category` the strongest ability, categories multiply (estimate); a hero's/ship's discount applies galaxy-wide while any fleet of the owner holds it (estimate); a structure's discount applies at its own planet. Time floor 1 % of the list time.
* Capture bonus: `Planet_Capture_Bonus_Reward` x `SimConfig::capture_bonus_scale` (default 1.0, estimate) once per planet and faction, when the capture completes.

## Auto-resolve calibration (estimates, `examples/calibrate.rs`)

The statistical resolver was compared with `run_headless` on random near-even battles (2-40+ ships a side, half at a star base, tactical 3 seeds, auto 40 seeds each).
Tactical results are cached in `target/calibrate_cache.txt`; `OLD=1` reverts to the uncalibrated formula, `BASE/EF/EC/EFR/FOCUS/CK/CG` override single knobs.
Calibrated knobs (all in `SimConfig`, named estimates): `combat_base_factor` 1.5 (star base, structures, garrison: hull, shield and damage; was 0.15 when the tactical bases were far weaker than their raw numbers, re-measured 2026-10-09 after the hardpoint damage model (U47) and the station hit radius: pirate base + garrison beats 1, 2, 3, 5 Nebulons and 3 corvettes in the tactical battle, and loses to an MC-80; `auto_resolve_agrees_with_the_tactical_battle_on_pirate_bases`),
`combat_efficiency` [0.5 fighters/bombers, 0.5 corvettes, 1 frigates, 1 capitals] (small craft lose time closing in), `combat_escape` 0.12 (share of a losing fleet that retreats alive when its faction
owns more than one planet; the tactical AI does this, the old resolver killed everything). Tried and left off: `combat_focus` (focus fire, no gain), `combat_crowd` (damage cut for huge fleets, no gain).
With 3 tactical seeds per scenario the comparison has a noise floor of about 0.2 in win share, so winner agreement above ~85 % cannot be demonstrated.

### Population cap and waves (data rule, calibrated on 40+ ship battles)
`Space_Tactical_Unit_Cap` (150 population) limits what a side has in play. In the tactical battle the ships over the cap wait in a reserve and are called in as losses make room
(`battle_sim::Battle::pop_in_play` now counts only what is alive or on its way, so losses free population; `run_headless` calls the reserve in once per second, arrival after the 5 s reinforcement delay).
The statistical resolver does the same (`SimConfig::combat_waves`, default on): the first ships up to the cap fight, the rest arrive 2 rounds after room appears; uncalled reserve survives.
On 40+ ship battles (60 scenarios, 8 tactical seeds each, not used for fitting) winner agreement went from 76 % (no waves) to 93 %, mean |attacker win share difference| 0.36 -> 0.19, bias -0.23 -> +0.02.
`combat_escape` is now 0.2 (loser keeps about 0.2 of its ships in tactical). Left over: tactical battles of 40+ ships time out as draws in 15-25 % of the runs; the statistical resolver never draws.

### Battle time limit
The data has no time limit for space battles; `tactical_max_seconds` (3600, was 900) and `battle_sim` `max_time` (3600, was 1800) only guard headless runs. The "draws" of 40+ ship battles were a mixture of
(a) real attrition wars of reserve waves (a capital ship against 150 population of defenders takes many minutes) and (b) a battle-sim bug: flight is planar, so height offsets of reinforcements (random +-800) and launch bays
never closed and fighters circled a target 650 units above them out of weapon range for ever. Mobile ships now climb towards their target's height and reinforcements spawn in the battle plane.
With the 3600 s guard none of 40 sampled big battles ends as a draw; the agreement on the 40+ bucket (8 tactical seeds, 45 battles) is 75 / 85 / 94 % by sample, mean |difference| 0.16, bias +0.08 (auto slightly favours the attacker); loser survival 0.12 both ways (`combat_escape` back to 0.12).

## Economy additions (checked by `galaxy-script/tests/economy.rs`)
* `Additional_Multiplier_On_Mining_Colonies` (mining facilities): on a planet with `Planet_Is_Mining_Colony` (3 planets) the facility's percentage and absolute modifiers count (1 + multiplier) times (data comment: "net result multiplied by 2.0 if the affected planet is a mining colony").
* `Planet_Income_Gambling_Ability` (Underworld cantina, scrapyard and the like): every `Evaluation_Interval_In_Secs` the planet's income changes by `Max_Income_Modifier` (probability `Chance_To_Win`) or `Min_Income_Modifier` times its base value until the next evaluation (**estimate**: the data has the numbers only). Uses the galaxy RNG, so it is deterministic.
* The cached `income_per_week`, `population_cap` and `population_used` are refreshed at the end of every `step`, so they always match the state.
* Checked by a scripted human (30 weeks each as Empire, Rebel, Underworld: builds income structures, star bases, tech, fleets, captures planets): cached income equals a from-scratch sum over planets, structures, star bases, ship upkeep and the faction factor every week; population cap equals base + planet + star base + structure capacities; credits earned follow the income integral; each tech level opens more units (Empire 16 -> 27 -> 36 units, Rebel 21 -> 27 -> 29, Underworld 19 -> 23).
* Not modelled (Remake GC): corruption, black market income (`Planet_Black_Market_Modifier`, hero abilities), siphon / slice abilities, trade route income (all zero in Web Way).

## Galaxy fog of war (`sight.rs`, estimate)
The data names who sees what (`Minimum_Visible_Base_Level` 1, `System_Spy_Ability`, `Galactic_Stealth_Ability` with `Evade_Detection_Chance` 0 on all 22, the `Spy_Center` with `Galactic_Influence_Range` 350) but gives no sight range for planets and fleets, so the model is an **estimate**. A faction (and its allies) is *present* at planets it owns or has a fleet at: `Sight::Detailed`. The planets one lane away from a presence are `Seen`; all others `Hidden`. Fleets: own and allied are always detailed; a fleet in orbit is seen like its planet; a fleet in hyperspace is `Seen` when either end of its lane is not hidden; a fleet that holds a `Galactic_Stealth_Ability` unit is found only by those present in its system and never in hyperspace (detection is certain there because the evade chance is 0). `Detailed` shows contents, `Seen` shows the fleet with unknown contents ("?"), `Hidden` is not drawn. `System_Spy_Ability` fleets are detailed at their planet through presence; the spy network of the Rebel `Spy_Center` (range 350) is not modelled (no Web Way campaign builds it). `Galaxy::{sight_map, planet_sight, fleet_sight, visible_fleets}`; `galaxy_play::Snapshot::sight(world, viewer)` bundles them. The AI keeps its own layer.

## The passive Pirates rearm with the player's tech (user rule, every campaign)
The Pirates are the passive faction in **every** game (`World::passive_faction`, no longer tied to `ground_up`): they never get commands (refused), are never run by the AI, never move, attack or expand, and spend nothing.
They do rearm, in `Sim::rearm_passive` (once at the start, and on every week tick), with the tech level of the human player (`victory.active_player`; without one the best playable main faction):
* **Star base:** every pirate-owned planet gets the star base of the pirates' own chain at level `clamp(player tech, 1, top level of the chain)`; raised, never lowered. (The pirate chain has 3 levels in the data, so tech 3+ gives level 3.)
* **Garrison:** every week a pirate planet holding fewer than `PASSIVE_GARRISON_PER_TECH` (2, estimate = the user's "2 x player tech") x tech units gets one more, picked deterministically (planet id + week, cycling) from the ship types
  the pirates field in the campaign (their own fleets: the data tags those ships with other factions' affiliations; fallback the neutral buildable ships), no heroes or planet destroyers. The units sit in a pirate fleet at that planet.
Tests: `the_pirates_rearm_with_the_players_tech_in_every_game_and_never_expand` (tests/ground_up.rs, normal GC and ground up; without enemies the pirate planet count stays the same). Other AIs still conquer pirate planets.

Fog of war, U89 correction: a corvette (or any unit with `System_Spy_Ability`) does **not** reveal the neighbouring systems. The data gives `System_Spy_Ability` the *system its unit is in* (`See_Base_Levels`, `See_Credit_Income`, `See_Num_Fleets` ...), which presence already covers; the only ranges in the XML are `Galactic_Influence_Range` of the Spy Network (350, Rebel `Spy_Center`, not modelled) and of a few hero abilities (200). The earlier rule "any corvette gives Detailed sight of all neighbours" had no source and was removed (test `a_corvette_next_door_does_not_reveal_the_neighbouring_system`).
