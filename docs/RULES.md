# Rules

The game rules the simulations implement, and where each one comes from. Source of truth is the Remake 4.0 data
(`Data/Xml`, `Data/Scripts` of Workshop item 1770851727) with the base game files layered below it. Paths are relative
to a mod's `Data` folder.

Every rule is one of three kinds:

* **Data**: read from the XML or Lua files at run time. The file and tag are named.
* **Estimate**: the engine is closed source and the data does not define the rule. The value is ours, named in
  `SimConfig` (galaxy) or `BattleConfig` / `defs.rs` (battle) so it can be tuned.
* **Project rule**: a deliberate design decision of this project, not Remake behaviour.

Gameplay numbers are never invented silently. A value without a data source is an estimate and says so in the code
and here. Known holes are in [KNOWN-GAPS.md](KNOWN-GAPS.md).

# Part 1: Galactic Conquest (`galaxy-sim`, `galaxy-script`, `galaxy-ai`)

## 1.1 Time

* Estimate: there are no weeks in the data. One week is `SimConfig::week_seconds` = 60 s, chosen so a Star Destroyer
  (`Build_Time_Seconds` 70) takes about a week. Income is quoted per week but accrues every tick (`tick_seconds` = 1 s).
* Data, not used by the galactic layer: `Xml/Gameconstants.xml` has `Object_Max_Speed_Multiplier_Galactic` (5.0),
  `Fleet_Maintenance_Update_Delay_Seconds`, `Space_Retreat_Allowed_Countdown_Seconds` (60) and
  `Space_Capture_Allowed_Countdown_Seconds` (1); the last two are tactical-mode values.

## 1.2 Economy

| Rule | Kind and source | Implementation |
|---|---|---|
| Planet income = `Planet_Credit_Value` | Data: `Xml/Planets_WW.xml` (Coruscant_WW 607) | per week per owned planet |
| Structures, star bases and units change income through `Planet_Income_Bonus_Ability` (`Absolute_Income_Modifier`, `Percentage_Income_Modifier`) | Data: `Buildings/Space/GC/Shipyard_*.xml` (upkeep -35), `Buildings/Ground/GC/**/Mining_*.xml` (+25), units (freighters +10% to +150%, Errant Venture +40%) | `World::planet_income`: `base * (1 + sum(pct)) + sum(abs)`; unit percentage modifiers apply while orbiting an owned planet |
| Ships cost upkeep through the same ability (Star Destroyer -800) | Data: `Units/Space/Units_Space_Empire_Star_Destroyer_Imperial.xml` | summed over all ships of a faction |
| Faction multiplier `Credits_Accumulation_Factor` | Data: `Xml/FACTIONS.XML` (Rebel 0.95) | multiplies the faction's total income |
| Credit cap `Credit_Cap_Per_Planet` 400000 | Data: `Xml/Gameconstants.xml` | Project rule: off by default (credits never expire); `SimConfig::credit_cap_per_planet` > 0 enables it |
| Credits never go below 0 | Estimate: the engine rule is unknown | clamped |
| `Political_Income_Curve`, `Income_Redistribution` | Data: `Xml/Gameconstants.xml` | ignored: political control is always 1 for owned planets |
| `Income_Stream_Ability` on mines | Data: `Buildings/Ground/GC/**/Mining_Facility*.xml` | ignored: its comment says it exists in tactical modes only |
| `Additional_Multiplier_On_Mining_Colonies` | Data: mining facilities | on a planet with `Planet_Is_Mining_Colony` the facility modifiers count (1 + multiplier) times |
| `Planet_Income_Gambling_Ability` (Underworld structures, heroes: Mon Mothma +-25%, Jabba, Tyber, Xizor -50%/+100%) | Data: `Evaluation_Interval_In_Secs`, `Chance_To_Win`, `Max/Min_Income_Modifier`; Estimate: when both a gambling structure and a hero are present, both roll independently on their own timers and stack additively | uses the galaxy RNG, so it is deterministic; heroes evaluate while orbiting an owned planet |
| Attrition credits (`Destruction_Survivors` spawning `N_Death_Penalty_Dummy` or `N_Reverse_Death_Penalty_Dummy`) | Data: `Units/Space/_Attrition.xml`, `DEATHPENALTY.lua` / `DEATHPENALTYUNOREVERSE.lua` | Craft and garrison losses cost 20 to 300 credits per craft lost; destroyed heroes and stations refund 2400 to 12860 credits to their owner; applied in tactical and auto-resolved battles |
| Starting credits, tech levels, home planets, forces | Data: `Xml/Campaigns_*.xml` (`Starting_Credits`, `Starting_Tech_Level`, `Home_Location`, `Starting_Forces`) | `FactionInfo` |
| Trade route `Credit_Gain_Factor`, `Political_Control_Gain`, black market, corruption | Data: `Xml/TradeRoutes_Web_Way.xml` | not modelled; every Web Way lane has both at 0 |

The cached `income_per_week`, `population_cap` and `population_used` are refreshed at the end of every `step`.
`galaxy-script/tests/economy.rs` and `galaxy-sim/examples/audit_economy.rs` recompute the start income of every main
faction from the XML and compare.

Population: units have `Population_Value` (data). The cap is `SimConfig::base_population_cap` (estimate, 40) plus, for
every owned planet, the planet's `Additional_Population_Capacity`, its star base's and its structures' (data).

## 1.3 Production

* Three lanes per planet, one item at a time per lane: ships, research items and structure level swaps (lane 0),
  structures (lane 1), star base and tech upgrades (lane 2). Estimate: cost is paid when queued (the real game
  charges progressively). Structure swaps share the ship queue: the structure lane only advances a non-structure
  when it is the first item, so an upgrade queued behind a real structure would never finish.
* Time = `Build_Time_Seconds` (data) x `build_time_scale`. `Production_Speed_Mod_Base_Vs_Tech_*` are all 1.0 in the
  Remake, so a higher star base level does not speed production up.
* A ship may be queued when all of these hold (`World::check_build_unit`, data tags): `Affiliation` contains the
  faction; `Tech_Level` <= faction tech; `Required_Star_Base_Level` <= the planet's base level; every
  `Required_Special_Structures` group is met (a structure also satisfies the requirement of any object in its
  `Variant_Of_Existing_Type` ancestry); the planet is one of `Required_Planets` (names not in the galaxy are ignored:
  estimate); the planet can hold a star base of the required level (`Max_Space_Base`, otherwise the item is hidden);
  the object is not `Build_Initially_Locked` unless unlocked; `Build_Limit_Current_Per_Player` and
  `Build_Limit_Lifetime_Per_Player`; population; credits. Objects with `Tech_Level` >= 90 (story-unlockable,
  terrain-specific templates) are never offered.
* `Build_Initially_Locked` objects are unlocked by research items: completing `Research_<Name>`
  (`Research/Units/_Default_Base.xml`, mirrored by `Scripts/Library/libResearchFunctions.lua`) unlocks `<Name>`.
* Structures: slot limits come from the planet's `Special_Structures_Space` (orbital) and `Special_Structures_Land`
  (ground); queued items already take a slot. Also `Build_Max_Instances_Per_Planet`, `Required_Star_Base_Level`
  (shipyards need level 2), `Required_Special_Structures`, `Required_Planets`. Terrain variants (`*_Urban`, `*_Barren`,
  `*_Sector`) and one-planet buildings (Starforge, refineries, fortresses) are selected by `Required_Planets`.
  Ground structures appear only if they matter for the galactic game (`StructureType::galactic`).
* `Required_Special_Structures` syntax (data, checked against the XML): names separated by whitespace or commas are all
  required, names joined by `|` are alternatives (`Empire_Outpost_Urban | Imperial_Palace_Urban`), and both mix.
  Refusal texts use the game's strings (`TEXT_REQUIRED_TECH_LEVEL`, `TEXT_REQUIRED_STAR_BASE_LEVEL`,
  `TEXT_REQUIRED_SPECIAL_STRUCTURE`, `TEXT_REQUIRED_PLANET`, `TEXT_BUILD_N_LIMIT_*`).
* One star base per planet. A planet's cap is its `Max_Space_Base` (data: all real planets of `Planets_WW.xml` say 3,
  dummy and icon planets 0 or 1); nothing is classified or guessed. A unit whose required structures all need a
  higher level than the cap is hidden from that planet's menu. Star base levels chain through `Prev_Level_Base` /
  `Next_Level_Base`; minor factions define unlinked levels (`Minor_Starbase_X`, `_2`, `_3`) that are chained by name.
* Heroes: the hero ship itself costs 1 credit in the data (`Piett_Procursator`); the real cost is the follow-up
  `Vessel_Upgrade_Template` item (25000 and more) that replaces a ship in orbit (`Required_Orbiting_Units`, enforced).
  The menu shows the data value. Super ships with `Required_Planets` or `E_Shipyard_3` follow the data
  (`galaxy-sim/tests/super_ships.rs`).
* The build bar lists items in object load order (`GameObjectFiles.XML`) with `GUI_Row` 0 (upgrade items) before 1.
* Tech levels in the Remake and Clone Wars data: upgrade objects with `Build_Advances_Tech_Level`
  (`Research/Units/Upgrade_Dummies.xml`). Data: time 300 / 450 / 600 / 750 s, star base level 1, and the faction's
  `<F>_Research_Institute` on the planet where it is bought. Data: the price is the object's `Build_Cost_Credits`
  (20000 / 30000 / 40000 / 50000 for levels 2 to 5, 1000 for `CIS_Tech-1` and `Republic_Tech-1`). The institute
  requirement binds only the human player because the Remake scripts lock the institute for the AI; the built-in AI
  buys levels at any star base planet (estimate). The `Research_<F>_Shipyard_N` units have empty names and no effect.
* Technology price discounts: `Reduce_Technology_Price_Ability` on planets (The Maw 33%, Scarif 25% in Remake 4.0 data) reduces tech level upgrade prices for the owner of the planet. Stacking follows `Stacking_Category` (strongest in category wins, categories multiply); both Maw and Scarif share category 0 so controlling both gives 33% discount.
* Tech levels in vanilla data: `*_Level_N_Tech_Upgrade` objects (`Tactical_Build_Increments_Tech_Level`, cost
  `Tactical_Build_Cost_Campaign`), star base only. Estimate: factions without them get upgrades at 3000 x level.
* New ships join the idle fleet of that faction at the planet, or create one.
* Structure level upgrades and downgrades (`Scripts/GameObject/Upgrade_*.lua`, `Upgrade_Structure` /
  `Downgrade_Structure` in `Scripts/Library/libUpgradeFunctions.lua`). The build item is the space unit the script
  names, or the `GroundCompany` whose `Company_Units` is that ground dummy. Price, time, tech, star base level,
  affiliation, build tab and `Required_Special_Structures` come from that object. The item is offered only while the
  planet has the `from` structure (a `Variant_Of_Existing_Type` spawner of it counts; that is what the build menu
  places). Completing it replaces every exact `from` in place, or one spawner stand-in when no exact `from` is
  present. The slot stays taken. An upgrade refunds nothing when it replaced a structure, and
  `(to build cost - from build cost) * 0.8` once when it did not. A downgrade refunds
  `(from build cost - to build cost) * 0.5 + 1` per structure replaced, or 1 credit when none was. Those costs are
  the structure objects' `Build_Cost_Credits`, not the item price (already paid at queue). The script does not copy
  health, and structures have no per-instance health here. `Build_Max_Instances_Per_Planet` caps how many of that
  item sit in the planet's ship queue. `Required_Ground_Base_Level` is not enforced. `*_ORBITAL_DUMMY` objects are
  not spawned, so the script's despawn of them does nothing.
* Selling (`Command::SellStructure`): Data: refund = build cost / 2 + 1, from `Delete_Building` in
  `Scripts/Library/libUpgradeFunctions.lua`. Ground structures use the same rule (estimate; no sell dummy exists).
  Ships (`Command::SellUnit`): Project rule: 35% of the build cost for fighters, bombers, transports and corvettes,
  55% for frigates, capitals and super capitals; the data has no rule to sell a ship. One ship per order, no heroes,
  not in hyperspace or at a planet with a pending battle. Some manifest sell texts are wrong or placeholders (Golan I
  "13K"); the script rule is the truth.
* Production discounts: per `Stacking_Category` the strongest ability applies, categories multiply (estimate). A
  hero's or ship's discount applies galaxy-wide while any fleet of the owner holds it (estimate); a structure's
  or planet's discount applies at its own planet, unless flagged `Galaxy_Wide` (e.g. Corellia's 25% corvette build
  time reduction) which applies to all planets owned by the faction (data). Time floor 1% of the list time (estimate).
* Capture bonus: `Planet_Capture_Bonus_Reward` (data) x `SimConfig::capture_bonus_scale` (1.0, estimate), once per
  planet and faction when the capture completes.

## 1.4 Movement

* Data: lanes are the campaign's `Trade_Routes` between real planets (`Xml/TradeRoutes_Web_Way.xml`: `Point_A`,
  `Point_B`, `HS_Speed_Factor` 1 to 3). Length is the Euclidean distance of `Galactic_Position` x/y.
* Fleet speed = slowest ship's `Hyperspace_Speed` (data, clamped 0.1 to 1.5) x lane `HS_Speed_Factor` (data) x
  `galactic_speed_scale` (6.0 map units per second, estimate). Paths minimise travel time (Dijkstra). A trip takes
  the sum of its lanes' travel times (passing a planet costs no time and gains none). A fleet that passes a planet holding
  hostile fleets or a hostile star base is stopped there (blockade; estimate).
* `Retreat` sends a fleet back to the planet it came from; a fleet in hyperspace turns around. A fleet already in
  hyperspace cannot be redirected, only retreated.
* Escape after a lost tactical battle (retreated losers, attackers after a draw): the fleet flies the fastest path to
  the nearest friendly planet, passing other planets without a battle, and blockades do not stop it. With no friendly
  planet left it takes the cheapest lane to the neighbouring system. Estimate; no attrition
  (`Space_Retreat_Attrition_Factor` is 0 in the data).

## 1.5 Battles and auto-resolve

* A battle starts when two hostile parties share a planet. Parties are the factions with fleets in orbit plus the
  planet's owner if it has a star base or armed structures. Data: all factions are hostile to each other
  (`Enemies` in `Xml/FACTIONS.XML` and `Xml/Factions_Minors.xml`).
* `Event::BattlePending` is emitted; the app plays a tactical battle or calls `resolve_battle_auto`.
* Auto-resolve (`combat.rs`) is a statistical placeholder. Every ship, star base, armed structure and garrison
  squadron is a combatant with shield and hull from `Shield_Points` / `Tactical_Health`, and weapons aggregated per
  damage type from `HardPoint` and `Projectile` data. The damage-type versus armor-type matrix (`Damage_To_Armor_Mod`,
  673 entries in `Xml/Gameconstants.xml`) scales hits. Friendly `Combat_Bonus_Ability` auras with `Space_Automatic` on heroes
  and fleet commanders apply their damage, health, shield, defense, range and speed bonuses (filtered by `Applicable_Unit_Categories`,
  `Applicable_Unit_Types`, and `Excluded_Unit_Types`, stacked by `Stacking_Category`). Hardpoint health, ranges, cones and
  speeds are not simulated. Estimates: rounds of 4 s; a fight with no damage or over 600 rounds is a draw and the attackers
  retreat. Units start with the hull and shield share they carry; only an abandoned tactical fight leaves damage, and survivors
  heal after every battle.
* Calibration (estimates in `SimConfig`, `galaxy-sim/examples/calibrate.rs` compares the resolver with the tactical
  sim): `combat_base_factor` 1.5 (star base, structures, garrison), `combat_efficiency` (0.5 fighters and bombers, 0.5
  corvettes, 1 frigates and capitals), `combat_escape` 0.12 (share of a losing fleet that retreats alive when its
  faction owns more than one planet). `combat_waves` (on): the first ships up to the population cap fight, the rest
  arrive 2 rounds after room appears. Data: `Space_Tactical_Unit_Cap` (150). With waves, winner agreement with the
  tactical sim on 40+ ship battles is 75 to 94% (noise floor about 0.2 in win share with 3 tactical seeds).
* A tactical battle reports survivors per group; losses, star base and structures are applied like the statistical
  result. Units that left by `Retreat` count as survivors and escape (see 1.4). Estimate: damage is not carried over
  between battles, surviving ships are repaired. `tactical_max_seconds` (3600) only guards headless runs.
* Difficulty multipliers (`Sim::set_faction_difficulty`, from `alamo_data::DifficultyAdjustment`, data): positive
  income x `credit`, build times x `build_time`, damage dealt x `damage`, damage taken divided by `health` (hull) and
  `shield`. Default 1.0.

## 1.6 Planet capture without ground combat

The data contradicts itself on ground troops (the mod ships `GroundCompany` objects and `Trooper_*_Regiment` start
forces), so the rule is derived from the Remake's own AI.

* Data: `Scripts/Ai/Conquer_Enemy_Space.lua` and `Conquer_Unrestricted_Space.lua` send a space-only task force to a
  target, `Move_To`, and succeed when `Planet_Has_Friendly_Space_Force_In_Orbit` is true. The task then handles
  `SpaceForce_Original_Target_Owner_Changed`: the engine changes the owner from space control alone.
* Estimate: a planet changes owner after one hostile faction has been the only party in orbit for
  `capture_delay_seconds` (5 s) and the owner has no star base. Attackers must first destroy star base, armed
  structures, garrison and defending fleets. Unowned planets are captured by any arriving fleet. If the real engine
  requires a landing, `capture()` in `sim.rs` is the one place to change.
* On capture the star base, structures and build queues are removed. Not modelled: `Planet_Surface_Accessible`,
  ground defenders, siege and bombardment, planet shields, story scripts that flip owners.

## 1.7 Starting state, victory, elimination

* Data: planet owner is the faction that places a star base or structure there (`GameData::campaign_planet_owners`);
  star base, structures and fleets come from `Starting_Forces`; ground units and heroes are ignored. Neutral planets
  start unowned and undefended.
* Data: `Human_Victory_Conditions` (`Galactic_Control_Named_Planets` with `Human_Victory_Planet_Names`, and/or
  `Galactic_All_Planets_Controlled`) for the campaign's `Starting_Active_Player`; `AI_Victory_Conditions` for every
  other faction. A faction with no planets and no fleets is eliminated; the last one standing wins. Victory can be
  switched off with `SimConfig::victory_enabled`.

## 1.8 Galaxy fog of war (`sight.rs`)

The data names who sees what (`Minimum_Visible_Base_Level` 1, `System_Spy_Ability`, `Galactic_Stealth_Ability` with
`Evade_Detection_Chance` 0 on all 22, the Rebel `Spy_Center` with `Galactic_Influence_Range` 350) but gives no sight
range for planets and fleets, so the model is an estimate.

* A faction (and its allies) is present at planets it owns or has a fleet at: `Sight::Detailed`. Planets one lane away
  are `Seen`; all others `Hidden`.
* Own and allied fleets are always detailed. A fleet in orbit is seen like its planet; a fleet in hyperspace is `Seen`
  when either end of its lane is not hidden. A fleet with a `Galactic_Stealth_Ability` unit is found by those present in
  its system, elsewhere only through a spy that sees its kind (below), and never in hyperspace. `Detailed` shows
  contents, `Seen` shows the fleet with "?", `Hidden` is not drawn.
* Data, spies (`Galaxy::intel_map`): every `System_Spy_Ability` that lasts (`Duration_In_Secs` -1, no `Causes_Despawn`)
  on a unit in orbit or a structure on an own planet, and every running field agent report (1.14), reveals what its
  `See_*` flags name: `See_Base_Levels` (star base and ground base level), `See_Credit_Income`, `See_Special_Structures`,
  `See_Current_Production`, `See_Num_Fleets` (planet at least `Seen`), `See_Fleet_Contents` (`Detailed`),
  `See_Minor_Stealth_Heroes` (stealthy agents) and `See_Major_Stealth_Heroes` (other stealthy units). A missing flag is
  No. Own and allied planets are known in full.
* Data, range: the spy covers its own planet and every real planet within its `Galactic_Influence_Range` (galaxy map
  distance, x/y of `Galactic_Position`; on the ability or the object): `Spy_Center` 350, `MTC_Spy*` 180, Dodonna 50, the
  rest 0. Fleets in hyperspace within `Hyperspace_Fleet_Reveal_Range` of the spy's planet (`Spy_Center` 350, MTC 180)
  are `Seen`, `Detailed` with `See_Fleet_Contents`. Estimate: the moving fleet's position is interpolated along its lane.
  The `Spy_Center` and both `*_Orbital_Long_Range_Scanner` are `Neutral` in the Remake, so in 4.0 only units spy at range.
* The galaxy hover card of an enemy or neutral planet shows what the player's spies reveal: the planet's current income
  instead of its base value, star base and ground base level, structures and what it is building. Fleets follow the
  sight rules above. The AI's knowledge (1.12) counts spied planets as `Detailed`.

## 1.9 Pirates: the passive faction

Project rule: the Pirates are passive in every game (`World::passive_faction`). They get no commands, are never run
by the AI, never move, attack or expand and spend nothing. Other AIs still conquer Pirate planets. They rearm in
`Sim::rearm_passive` (at the start and every week) with the tech level of the human player (`victory.active_player`;
without one, the best playable main faction):

* Star base: every Pirate planet gets the star base of the Pirates' own chain at level `clamp(player tech, 1, top of
  the chain)` (3 levels in the data); raised, never lowered.
* Garrison: every week a Pirate planet holding fewer than `PASSIVE_GARRISON_PER_TECH` (2, estimate) x player tech units
  gets one more, picked deterministically (planet id and week) from the ship types that planet already fields (what
  the campaign data put there). A planet with none gets the three cheapest passive ship types. No heroes or planet
  destroyers.
* Data for context: the Pirates' strength is mostly the start spawn (`Spawn_Starting_Forces_Web_Way.lua` gives each
  Pirate planet `Free_Random(2000, 3000)` x 0.5 combat power against 500 to 750 x modifier for main factions);
  `Pirates_Template` has no infrastructure, defensive or offensive budget and `Maintenance_Cost` 0.

## 1.10 "From the Ground Up" (`ground_up.rs`)

Project rule: the Remake has no such mode. The name appears only in vanilla strings; the `UNIT_FILTER_*.LUA` scripts are
build-menu filters, not a start mode. The mode is this project's own:

* Main factions (`World::main_factions`, derived from the campaign): the human plus every faction whose
  `AI_Player_Control` is a full AI plot (`GC_RebelAi`, `GC_EmpireAi`, `GC_UnderworldAi`, `SandboxHuman`); not
  `GC_MinorAi` / `GC_PiratesAi`. In Clone Wars galaxies the sides are the factions the sibling campaigns of the same
  `Campaign_Set` are played as (`GameData::campaign_sets`).
* The human keeps 1 planet (its `Home_Location`); every other main faction keeps 3 (home plus the two nearest it
  owns). Released and unowned planets go to the passive Pirates. Minors keep theirs. Credits and tech are the
  campaign's values. The setup does not count as captured or lost planets in the statistics.
* Valid starts: start planets keep the campaign's star base level (clamped to `Max_Space_Base`; a planet without a base
  gets level 1). Then `Sim::drop_invalid_structures` removes, until stable, every structure the build rules refuse in
  that state (tech above the faction's, star base level above the planet's, `Required_Planets` not containing the
  planet, missing `Required_Special_Structures`); dependants go too. Objects with `Tech_Level` >= 90 are exempt. Start
  units are not touched. `ground_up_starts_are_valid_and_not_bankrupt` runs over every galactic campaign.
* Build rules are the normal ones, so a one-planet start offers only what that planet can build (Rebel at Yavin can
  queue no ship at the start, only structures, a star base upgrade or tech).
* The built-in AI saves for the cheapest shipyard while no owned planet can build a shipyard-built ship, and prefers
  strong ships once half the population cap is used, saving up to `SAVING_WEEKS` = 4 weeks of income (estimate). It
  picks only targets whose path crosses no blockade it would lose to and whose battle it wins in a fixed-seed
  `combat::resolve_allied` (`wins_battle`), judged with `World::tuned_strength` (estimate).
* Data worth knowing: some Clone Wars starts are negative or very large because the mod's own start structures say so
  (Underworld at Ryloth_WW is -5 per week in several Clone Wars campaigns; Separatist Holdouts at Mustafar earns
  about 1900 per week from the CIS headquarters and mines). The Ground Up mode trims planets, not start structures.

## 1.11 Clone Wars stack (Republic, CIS)

* Content is an explicit choice (new-game screen "Content" row or `--mod fft`); the default is Remake 4.0. The sim has
  no faction-specific code: Republic and CIS are ordinary data factions (`galaxy-sim/tests/clone_wars.rs`).
* Data: besides the three `*_Story_Name` tags, every playable faction's `Story_Name` entry of the campaign is started.
* Data-derived, unverified against the submod: UI skin, announcer and HUD voice set follow the faction icon named in
  `FACTIONS.XML` (Republic `i_icon_rebel.tga` uses the Rebel set, CIS `i_icon_pirate.tga` the Underworld set).
* Data: Clone Wars plots `LOCK_UNIT` the GCW-era heroes at `Universal_Story_Start`. A `STORY_TRIGGER` fires when its
  prerequisites have, whatever the reward (inferred in `alamo-lua` `story.rs` from the Web Way plots: nothing triggers
  the hero spawns, locks, tech followers, shield grants or the 181st chain by name). A `STORY_TRIGGER` with no
  prerequisite waits until a `TRIGGER_EVENT` names it. `story_locked` makes the build checks refuse the object until a
  story `UNLOCK_UNIT`. Rosters otherwise stay the data `Affiliation`.
* Data, deliberately unchanged: the Shatterpoint campaigns give the full-AI opposing side 500000 credits against the
  human's 50000; a Republic human starts at about 497 credits per week.
* Estimate: history chart colours of non-classic factions are the XML faction colour lifted (`0.35 + 0.65 * c`).

## 1.12 The galactic AI

* Default AI = the Remake's AI (goal functions, perceptual equations and Lua plans, `galaxy-ai`) plus a strategy layer
  at difficulty Normal. The older `ai.rs` of `galaxy-sim` is the "classic" option: every `ai_interval_seconds` (10 s)
  each AI faction buys tech, builds star bases and structures, queues the best durability x damage per credit ship,
  merges idle fleets and sends a fleet at the best-valued reachable planet whose defence (Lanchester estimate
  `sqrt(durability x dps)`) it exceeds by 1.5x.
* Difficulty: Easy, Normal and Hard are the `Xml/DifficultyAdjustments.xml` entries (data: credit, build time, battle
  multipliers, `Galactic_AI_Contrast_Multiplier`, `Galactic_AI_Goal_Cycle_Sleep_Duration`; the unit of the sleep is an
  estimate). Brutal is a project tier (estimate): Hard plus AI credits x4 (`BRUTAL_CREDIT_MULTIPLIER`, Hard is 3.0),
  build time x0.8 (`BRUTAL_BUILD_TIME_MULTIPLIER`, Hard 0.9) and full map knowledge. Fair is a project tier: no
  multipliers, Normal contrast.
* AI knowledge. Data: `AIUsesFogOfWarGalactic` False (`Xml/Gameconstants.xml`, Remake 4.0 and Clone Wars), so the AI
  of every difficulty knows every planet and fleet, tierless factions too. The player's fog (1.8) is unchanged. Only
  if the data turns the AI fog on, every tier but Brutal is limited: the AI knows enemy forces
  exactly only at planets within `AI_FOG_SIGHT_HOPS` = 2 lanes (estimate) of any planet it owns or any fleet it has,
  plus every planet it sees `Detailed` through 1.8; outside that it assumes only what the map shows (the owner's star
  base with garrison, no fleets, no armed structures; estimate).
* Strategy layer (`strategy.rs`, all weights are estimates): see [ARCHITECTURE.md](ARCHITECTURE.md), galaxy-ai.
* Data: the 4.0 AI budget gives the `Hero` category 0% and the only hero goals are `Remove_Corruption` and the Death
  Star, so the AI keeps heroes at home.
* Estimate: AI tech research. The Remake has no galactic research plan; with no human player the engine queues levels
  on a timer.
* Data: AI map hints from `Data/Xml/AI/GalacticMarkup` (`RemakeHints.xml`: 17 chokepoint planets, 7 sectors at 1.0, 7 systems at 0.75, 3 gas giants at 0.5) are loaded and used by perceptual equations (`Hints.Chokepoint`, `Hints.PriorityTarget`).

## 1.13 Story ability grants

* Data: `NEW_POWER_FOR_ALL` (`Reward_Param1` the unit type, `Reward_Param2` the ability) turns that ability on for
  every unit of that type the plot's faction fields, including ships built later, for the rest of the campaign.
  `Rebel_Shield_Upgrade` (75000 credits, tech 4, 600 s) grants `Reflector_Shield` to the 25 types in
  `Story_SandBox_Rebel_Shield_Upgrade.xml`. The grant is saved with the faction and is not part of `Galaxy::state_hash`.
* Data: `Rebel_Shield_Upgrade` is a `DUMMY_STARSHIP` variant of `Dummy_R` (`Build_Tab_Space_Units`, no `Space_Model_Name`).
  The Rebel space build tab offers it at tech 4, with a level-1 star base, `R_Research_Institute`, and a build limit of 1
  current and 1 lifetime. Finishing it spawns no ship; the construct notify is what grants the shields.

## 1.14 Agents (`agents.rs`)

* Data: agents are the `HeroCompany` objects with `CategoryMask` `NonCombatHero` and `Build_Tab_Heroes` whose company units
  carry a `Siphon_Credits_Ability` or a `System_Spy_Ability` (`Units/Ground/Non-Combat_Heroes.xml`): `Rebel_Smuggler_Team`
  (Rebel), `Underboss_Team` (Underworld), `Rebel_Spy_Team` (Rebel); `ISB_Agent_Team` and `Underworld_Saboteur_Team` are
  `Neutral`. Cost 500, 60 s, tech 0, population 1, `Required_Star_Base_Level` 0, `Required_Ground_Base_Level` 1, limits and
  locks from the company; hyperspace speed from the `Company_Transport_Unit` (`Alliance_Shuttle`, `Shuttle_Tyderium`: 0.75);
  `Galactic_Stealth_Ability` from the company unit. Smuggler: a Rebel outpost or Base One on the planet; underboss: a cantina
  or criminal palace; field agent: the `Spy_Center`.
* Data: the `Spy_Center` has `Affiliation` `<!--Rebel-->,Neutral`, so no Rebel can build it and the field agent is never
  offered in the Remake unless a Spy Center stands on a Rebel planet.
* Data: ground base level. `GroundBases.xml` says ground bases are no longer built and their level is kept in step with the
  number of special structures; `World::ground_base_level` is the number of ground (non-orbital) structures on the planet,
  capped at the highest `GroundBase` `Base_Level` (5). The build checks refuse a unit below its `Required_Ground_Base_Level`
  ("Required : Ground Base Level"). The only structure with the tag (`Space_Mode_Ground_Rebel_Ion_Cannon`) has `Tech_Level`
  99, so structures do not check it.
* Estimate: agents are queued in the ship lane (the game has a ground queue this project does not model) and join the
  idle fleet at the planet like ships. They move like any fleet.
* Estimate: agents never fight (`Combat_Power_Value` 0, `Is_Stealth_Company`). A fleet of agents only is no party of a
  battle, does not contest or capture a planet and does not blockade; in a mixed fleet the agents are not battle members
  and stay when the warships are lost. A faction whose fleets are all agents and that owns no planet is eliminated and
  its agents leave the map.
* Credit siphon (data: `Min_Siphon_Percentage` 0.20, `Max_Siphon_Percentage` 0.30, `Siphon_Duration_In_Cycles` 5,
  `Causes_Despawn`, `Activation_Style` `Ground_Activated`; the cycle is `Fiscal_Cycle_Time_In_Secs` 45 s of
  `Gameconstants.xml`; text: "Siphons 20% to 30% credit income of an Imperial or Criminal system for 5 months").
  `Command::ActivateAgent` (the galaxy's "Siphon credits at ..." button) at a planet owned by a hostile faction uses up
  the agent; for 5 cycles of 45 s a share of the planet's income goes to the agent's faction instead of the owner.
  Estimates: the share is drawn from the galaxy RNG at the start and at every new cycle; the siphon runs on without the
  agent and ends early when the planet is no longer a hostile faction's; one siphon per planet (the Remake AI's
  `Suitable_Smuggler_Target`: "Can't have multiple smugglers on a target"). "Ground_Activated" (landing the agent) is the
  order, as there is no ground layer.
* Field agent (data: `Probe_Droid_System_Spy`, `Galactic_Automatic`, `Duration_In_Secs` 60, `Causes_Despawn`, `See_*`
  flags): when it orbits a planet of a hostile faction it is used up and its faction gets a spy report on that planet for
  60 s (`Galaxy::spy_reports`, saved). Estimate: one report per planet and faction at a time.
* Not modelled: the AI does not recruit or use agents; agents cannot be caught (`AntiStealthEffectiveness` in the AI
  equations); `Neutralize_Hero_Ability` of the neutral ISB agent.

## 1.15 Super ship damage phases on the galaxy map

* Data: in a space battle the `Set_SSD_{Executor,Eclipse,Piett,Viscount,Subjugator}_State` object scripts set
  `GlobalValue` `<Name>_Level` to the phase of each phase object as it appears (1 to 5, Subjugator 1 to 4) and to 0 when
  the last phase's `Death_Clone` (`Death_Clone_*_Phase`) appears. `battle-sim` reads the branches of those scripts
  (`object_name == "TYPE"` then `GlobalValue.Set("KEY", N)`) and reports the sets in order (`BattleResult::globals`,
  `TacticalResult::globals`, `Event::TacticalGlobals`); `galaxy-script` writes them into the store before the battle's
  story events. The galactic `Story_SSD_*_Damage.lua` plots then run as in the Remake on `STORY_WIN_BATTLES` /
  `STORY_LOSE_BATTLES`: with a level other than 0 they spawn that phase (if none of that type exists) at the planet of
  the most recent space battle, or at the owner's nearest planet when an enemy holds its orbit
  (`Should_Despawn_Orbital_Unit`), and reset the level to 0. The phase that was destroyed is removed by the battle (and
  the plot's `REMOVE_UNIT`).
* Data: `Time_Since_Space_Conflict` (`Remake_SSD_PhaseEquations.xml`, `Variable_Target.TimeSinceSpaceConflict`) is the
  game seconds since the last space battle at the planet was decided (`PlanetState::last_space_battle`, saved).
  Estimate: a planet without one counts 1e9 s (the plots start their search at 10000).
* Battles the player settles (tactical result, auto-resolve, abandon) reach the story like those the sim settles in
  its step (before, their `STORY_WIN_BATTLES` / `STORY_LOSE_BATTLES` / `STORY_TACTICAL_DESTROY` were not reported).
* Data, repair (`Scripts/GameObject/Upgrade_Units.lua`): `Repair_Phase_<n>_<ship>` (Executor, Eclipse, Piett and Viscount 2
  to 5, Subjugator 2 to 4; `Vessel_Upgrade_Template`, `Required_Orbiting_Units` the damaged phase, 140000 credits and
  200 s for the Viscount) turns phase n of the ship in orbit into phase n - 1, like the hero upgrades.
* Data, auto-resolve: the object scripts exit outside space battles (`Get_Game_Mode() ~= "Space"`), so the Remake has no
  phase outcome without a tactical battle: an auto-resolved battle sets no level and a destroyed phase is lost (the
  plots run on its `STORY_LOSE_BATTLES` and find level 0).

# Part 2: Space battles (`battle-sim`)

Everything is derived from the Remake data plus the `.alo` models for hardpoint positions. Values the engine defines
but the data does not show are estimates, kept in `BattleConfig` or `defs.rs`.

## 2.1 Units, space and time

* Fixed step `dt` = 1/30 s. Battle space: X forward at yaw 0, Y left, Z up; ships move in the XY plane (heights are
  followed at half top speed towards the target's height).
* Data: XML speeds (`Max_Speed` of ships and projectiles) are per frame and multiplied by `Catalog::speed_scale` = 30
  (the 30 Hz logic tick, estimate) to get units per second: a Star Destroyer 90 u/s, laser bolts about 1000 u/s,
  turbolasers 1350 u/s. `Max_Rate_Of_Turn` is used as radians per second. All `*_Seconds` fields are real seconds.
* Estimate: acceleration times by size class (fighter 1.5 s, corvette 4 s, frigate 6 s, capital 10 s, super 16 s to
  full speed).
* Model space (`.alo`) is Z-up with the bow along -Y and maps to battle space with `(x, y, z) -> (-y, x, z)`. Verified
  with the Star Destroyer: the bow turbolaser `HP_ISD_1_New_TL_05` ends up ahead (+X), the shield generator dome
  behind (-X).

## 2.2 Ships, hardpoints, projectiles

| Rule | Source |
|---|---|
| Hull `Tactical_Health`, shield `Shield_Points`. `Shield_Refresh_Rate` is gained once per `ShieldRechargeIntervalInSecs` (2 s) as a continuous `rate/2` per second. A shield drained to zero is offline for `Depleted_Shield_Disable_Time` (4 s). Estimate: `BattleConfig::shield_regen_delay` (default 0, regenerate even under fire) can pause regeneration after a hit; see 2.12 | Data: unit XML, star bases `Buildings/Space/GC/**`, `Xml/Gameconstants.xml` |
| `Armor_Type` / `Shield_Armor_Type` and the 673-entry `Damage_To_Armor_Mod` matrix scale hull and shield damage | Data: `Xml/Gameconstants.xml` |
| Hardpoint position = `Fire_Bone_A` (else `Attachment_Bone`) in the ship `.alo`. Fixed weapons fire along that bone's local +X. Turrets (`Is_Turret`, `Model_To_Attach`) take their rest direction from the attached turret model's barrel (`FP_01` minus `Barrel` bone, rotated by the mount bone; the mount axes alone are authored arbitrarily) | Data: `Units/Space/Hardpoints/*.xml`, `Art/Models/*.ALO`; Estimate: the rest-direction derivation |
| Arc = larger of the fire cone (`Fire_Cone_Width/Height`, full angles) and the turret extents (`Turret_Rotate/Elevate_Extent_Degrees`) in the mount frame. A mount whose arc cannot reach the battle plane is re-centred on the bow | Data; Estimate: the re-centring |
| Range `Fire_Range_Distance`, `Fire_Min_Range_Distance` (measured to the target's bounding sphere) | Data: hardpoint XML |
| Volley: `Fire_Pulse_Count` pulses `Fire_Pulse_Delay_Seconds` apart, then a random recharge between `Fire_Min/Max_Recharge_Seconds`. `Fire_Inaccuracy_Distance` per target class is added as aim error | Data: hardpoint XML |
| `Fire_Category_Restrictions` lists the classes the weapon must not fire at (only ion cannons: no fighters, bombers, gunships, transports, corvettes) | Data |
| Hardpoint `Health`, `Is_Destroyable`, `Is_Targetable` | Data |
| Projectile: `Projectile_Damage`, `Damage_Type`, `Max_Speed`, `Max_Rate_Of_Turn` (homing), `Projectile_Max_Flight_Distance`, `Projectile_Does_Shield/Hitpoint_Damage`, blast range, damage and victims, ion stun duration and speed/rate reduction | Data: `Xml/PROJECTILES.XML` |
| Fighters fire a hull weapon: `Projectile_Types`, `Projectile_Fire_*`, range `Targeting_Max_Attack_Distance`, cone `Turret_Rotate_Extent_Degrees` | Data: unit XML |
| A "facing" hardpoint (`Proj_Facing_Dummy`, damage 1) makes the ship turn its bow towards its target | Data |
| Ships with more than 64 hardpoints exist (Subjugator 213, Super Star Destroyers 185, Executor 158); the alive mask is `HpMask`, not a `u64` | Data |

Hit resolution (estimate for the geometry): a projectile is a point moving along a segment; it hits the first enemy
bounding sphere it crosses (capital and frigate spheres shrunk to 75%). Shields absorb first (`damage x shield
matrix`); if a hit depletes the shield the remainder goes to the hull (`damage x armor matrix`). Projectiles without
shield damage bypass shields; ion cannons (no hull damage) only hurt shields.

Destroyed hardpoints (estimate): weapons stop firing; all shield generators gone collapse the shield and stop
regeneration; engines scale speed by `0.20 + 0.80 x alive/total` (see 2.13); fighter bays stop squadron replacement.
Ion stun slows the target and its fire rate for `Projectile_Ion_Stun_Duration` (data). Data: `Projectile_Ion_Stun_Speed_Reduction_Percent`
and `_Shot_Rate_Reduction_Percent` are fractions (0.2 to 0.999): speed x `(1 - cut)`, recharge / `(1 - cut)` (estimate: at most 10x;
the data's comment says -50% also halves the pulse count, which is not modelled). Numbers may carry a C float suffix (`0.999f`).

## 2.3 Hit points are the hardpoints

Project rule, not game data. A unit with destroyable hardpoints (`Is_Destroyable` and health above 0) has no hull pool
of its own: its hit points are the sum of its hardpoints and it is destroyed when the last one is. `Ship::hull`
stays as a derived figure for bars and decisions (`ShipDef::hull` times the share of hardpoint health left).

* The shield is a second health bar over the whole unit. Projectiles that do shield damage hit it first (the part of a
  hit that breaks it carries over to the hardpoints); projectiles without shield damage (missiles, proton torpedoes)
  bypass it (data: `Projectile_Does_Shield_Damage`).
* Targeted fire (`AttackHardpoint`) hits the chosen hardpoint; untargeted fire hits the living destroyable hardpoint
  nearest to the impact point.
* Overkill spills to the next nearest hardpoints, halving at every step, in whole numbers (a hit that does not destroy
  its hardpoint spills nothing). Easy to change in `damage_ship`.
* Units without destroyable hardpoints (fighters, bombers, small craft) keep a plain hull that dies at 0.
* Estimate: with shields down every untargeted hull hit also lands at full amount on the destroyable hardpoint nearest
  to the impact.
* Estimate: ships with destroyable hardpoints and none of type engine get a synthesised engine hardpoint 0.75 radii
  astern (health 8% of their destroyable hardpoint health, clamped to 1000 to 3000), because some capitals have none
  in the XML.
* Estimate, healing (`Force_Healing_Ability`, the healing harmonic bomb): a heal of N hull points mends N / `ShipDef::hull`
  of the unit's total hardpoint health, the most damaged living hardpoint (by share of its health) first, each up to its
  maximum; destroyed hardpoints stay destroyed (the data has no rule that rebuilds them). The hull figure follows.

## 2.4 Squadrons, carriers, stations

* Data: a `Squadron` object expands into `Squadron_Units` fighters placed with `Squadron_Offsets`
  (`Starfighter_Formations.xml`); every `Squadron_Units` line adds its craft (`Red_Squadron` 6 X-wings on 3 lines,
  `Darth_Vader_TIE_Fighter_Squadron` 9 fighters, `Hutt_V-Wing_Squadron` 5). `Starting_Spawned_Units_Tech_0` of ships and
  star bases launch at battle start; a wiped-out squadron is replaced from `Reserve_Spawned_Units_Tech_0` after
  `Spawned_Squadron_Delay_Seconds`, while a fighter-bay hardpoint (if the ship has any) lives.
* Project rule (the EaW behaviour; the data has no rule): launched squadrons fight on when their carrier or station is
  destroyed. They stay spawned units (2.8): they do not keep their side in the battle, are removed with the loser and
  cannot jump on their own. Squadrons whose carrier jumped out without them are lost.
* Data: star bases, shipyards and defence structures are static combatants with their own hardpoints. Golan I to III
  stand in a battle as their `*_Golan_N_Orbital` form and the Uplink Command as its control station, linked through
  `Place_Other_Type_At_Every_Specific_Marker_Position` and `Tactical_Additional_Structure_Type`.
* Estimate: fighters stay with their squad leader or carrier when idle and fly strafing runs (approach, break away 1 to
  2.5 s).
* Launch out of the hangar (estimate): fighters leave along the bay bone's X axis (data: the opening) until 40 u clear
  of the hull (`LAUNCH_CLEARANCE`), the rest of `Fighter_Bay_Flyout_Distance` (data) level; an axis that does not lead
  out within 150 u drops them below the hull first.

## 2.5 Targeting and movement

* Data: every 0.4 s (estimate) a ship scans enemies and keeps a candidate list sorted by `Targeting_Priority_Set` weight
  (lower first, then distance). Data: `Targeting_Stickiness_Time_Threshold` (5 s on `Vessel_Base` and `Spacecraft_Base`
  in `Units/Space/_Default_Base.xml`): a target taken up less than that long ago stays the target while it is still a
  candidate, even when a better one appears; a dead, unseen or out-of-range target is dropped at the next scan. Estimate:
  the AI side's priority target (`FREE_STORE_ATTACK_RANGE`, the free store's `Service_Attack` order) takes over at once.
  Sets come from `Xml/TARGETINGPRIORITYSETFILES.XML`: `Attack_Priorities`,
  `Category_Exclusions`, `Hard_Point_Priorities`. A ship uses its own set (the A-Wing has `Fighter`, so interceptors
  prefer bombers). The `<set>_Attack_Move` variants (hardpoint sniping order: shield generator, engines, hangar,
  weapons) exist but the Remake scripts only assign them to ground task forces; `BattleConfig::attack_move_sets`
  (default off) enables them for AI attack-move orders. Classes missing from a set are opportunity targets.
* Each weapon fires at the first candidate in range, inside its arc and allowed (estimate). Heavy guns (damage >= 150)
  and shield-only weapons never fire at fighters or bombers (estimate). Data: a hardpoint with
  `Allow_Opportunity_Fire_When_Targeting` False fires only at the ship's own target while it has one, one with
  `_When_Idle` False not at all while it has none (both true where the data gives none). Not modelled:
  `Hardpoint_Recharge_Cutoff_For_Opportunity_Fire` 0.5 (meaning unknown).
* Data: `Fire_When_In_Normal_Attack_Mode` No = the hardpoint fires only while `ROCKET_ATTACK` is on,
  `Fire_When_In_Rocket_Attack_Mode` No = only while it is off, `Fire_When_In_Power_To_Weapons_Mode` No = not during
  `POWER_TO_WEAPONS` (all default to Yes; the data only ever writes Yes for the last one). Hardpoints a unit can only
  use in rocket mode stay silent when the unit has no `ROCKET_ATTACK` (`Broadside_Cruiser_DB`, whose ability is
  commented out).
* Estimate: large ships close to the range that puts 80% of their weapon damage in reach (x0.95) and hold there; a
  long-range specialist (e.g. a missile ship) holds at 0.8 of its longest range plus half the target radius. They turn
  to the bearing (bow, broadside or stern) at which the largest share of their hardpoint damage bears
  (`ShipDef::bearing_dps`, from the arcs; a damaged ship counts only the guns it still has), stop with an arrival speed
  profile (no overshoot) and restart at 1.12 x the standoff. Fighters and bombers are fired at but not chased unless
  ordered.
* Data, chase ranges (`Units/Space/_Default_Base.xml`: `Vessel_Base` `Idle_Chase_Range` 0, `Guard_Chase_Range` 1000,
  `Attack_Move_Response_Range` 400; the Remake squadrons 1500 / 1500 / 1500, which their craft fly by). The data gives
  only the numbers; what each measures from is an estimate:
  * `Idle_Chase_Range`: an idle aggressive unit takes up only enemies within that range plus its own reach
    (`max(weapon range, Targeting_Max_Attack_Distance)`) of where it last rested (its carrier for a launched craft) and
    pursues them; 0 (all corvettes, frigates and capitals) holds the place as before. An idle squadron now attacks what
    comes near instead of waiting by its carrier.
  * `Guard_Chase_Range`: a guard closes on its target but never goes further than that from the guarded place; at the
    limit it stops and fires at what it reaches, with nothing to fight it returns. Replaces the estimate "engage within
    max(1.3 x weapon range, 2500) of the ship or 4500 of the place".
  * `Attack_Move_Response_Range`: a player's attack-move takes up enemies within that range plus its own reach (before:
    any enemy the side saw). The Remake's own tooltip (`TEXT_TOOLTIP_ATTACK_MOVE_01`) says its attack-move button
    issues a plain move. The AI plan's attack-moves (`ai.rs`, the free store's `Service_Attack`) keep taking up every
    enemy they see (estimate: they are not that order).
  * Not modelled: `Autonomous_Move_Extension_Vs_Attacker` (only the squadrons have it, 1500, the same as their idle
    chase; no ship records who attacked it).
* Orders: `Move`, `AttackMove`, `Attack(ship)`, `AttackHardpoint(ship, hardpoint)`, `Guard(place)`, `GuardShip(ship)`,
  `Stop`, `Retreat`, stances Aggressive / Defensive / HoldFire.
* Guard (`Command::Guard`; the command bar's `c_button05`, `TEXT_TOOLTIP_GUARD_BUTTON` "Follow unit or guard location",
  `_01` "Guarding units will only pursue enemies to a limited range and then return"): the button arms the guard
  pointer (`POINTER_GUARD`); the next left click on an own ship guards (follows) it, elsewhere guards the place; the
  ordered unit says its `SFXEvent_Guard` line. A player's guard takes up what it reaches and every enemy within
  `Space_Guard_Range` 2000 (`Gameconstants.xml`; 750 in the base game) of the guarded place or ship (estimate of what
  the range measures), pursues within `Guard_Chase_Range` (above) and with nothing to fight stays at the place or
  beside the ship (its radius + own radius + 150, like the AI's corvette escort). A ship cannot guard itself or an
  enemy. When the guarded ship is lost the guards hold where it was. The AI's `Service_Guard` maps onto `Guard(place)`
  (a defender at its station, a corvette beside its capital ship, re-planned every second); those guards keep the
  pick-up ranges of before (estimate: 5000 from the ship when aggressive, 4000 when defensive) and the
  `Guard_Chase_Range` leash. Not shown: `GUI_Guard_Move_Command_Ack_Effect` (the view draws no order acknowledgement
  particle of `Gameconstants.xml`; it marks orders with its own rings).
* Estimate: a broadside approach curve starts at 2.2 x standoff with a tangential offset of 0.45 x standoff; stop
  distance `max(2.5 x braking + 1.5 x radius, 400)`.
* Estimate, bomber attack run: a bomber that reaches a frigate-or-larger target flies on 3 to 5 s (turn at most 0.15
  rad/s) before the next pass, with x1.35 speed and x1.5 acceleration while leaving and a climb of 0.7 x speed; at a
  ship with a hull it peels off like a fighter. Hardpoint order: shield generator, engine, fighter bay, weapon.
* Estimate, dogfights: squadrons leave formation within 650 u of fighters or bombers; close-run distance `max(70 +
  slack, 180)`; slot blend radius 80 u; speed catch-up 1.5 u/s per u behind, 2.0 per u ahead.
* Estimate, escort avoidance: corvettes and frigates keep capital hull + own radius + 150 u away; capitals yield to
  escorts only when their circles overlap (0.85 of the radii; an escort on a rock's edge or between two capitals cannot
  make way), then the pair splits the overlap.
* Estimate, carrier screen: fighters near their carrier prefer bombers within 3500 u of it (weight x0.3).
* Estimate, flanking: corvettes and units faster than 250 u/s swing 3200 + 600 x (id % 3) u to the side on an
  attack-move (3500 + 500 x (id % 4) on a move). The Remake has flanking but no distances in the XML.
* Estimate, squadron flight: squadrons fly the data's `Squadron_Offsets` at the craft's model scale, led by the member
  nearest the middle; a leader slows while a member lags (`SQUADRON_LAG_START` 50 u, half speed at
  `SQUADRON_LAG_HALF` 250 u); a fallback offset `(-20 x i, 25 x (i % 2))` is used only when the data has none.

## 2.6 Abilities

`Unit_Abilities_Data` entries with an `Expiration_Seconds` become abilities with the `Mod_Multiplier` values of the
data: `SPEED_MULTIPLIER`, `SHIELD_REGEN_MULTIPLIER` (negative values clamped to 0), `WEAPON_DELAY_MULTIPLIER` (applied
to the recharge, below 1 is faster), `TAKE_DAMAGE_MULTIPLIER`, `CAUSE_DAMAGE_MULTIPLIER`. `Recharge_Seconds` starts after
the effect ends. `Supports_Autofire` (true on 89 of 96 entries) and `Active_By_Default` are data; every ability starts
with autocast off (the `Active_By_Default` entries are all in unused `_BU` files). A player ship with autocast on uses
the ability under the AI's rules (`Command::SetAutocast`).

| Type | Kind and rule |
|---|---|
| `TURBO`, `SPRINT`, `SPOILER_LOCK`, `DEFEND`, `POWER_TO_WEAPONS`, `INVULNERABILITY`, `MAXIMUM_FIREPOWER` and any entry with `Expiration_Seconds` | Data: the `Mod_Multiplier` values. Estimate: defaults where the data has none (e.g. `POWER_TO_WEAPONS` damage x1.35, delay x0.6, shield regen 0, speed x0.75; `DEFEND` damage taken x0.65, shield regen x1.6, speed x0.65, only for a `DEFEND` without any `Mod_Multiplier` in the data; `TURBO` speed x1.8). Data: `SHIELD_REGEN_INTERVAL_MULTIPLIER` divides the regeneration rate (Home One 0.5 = x2, TF-24 0.27 = x3.7; estimate: at least 0.05) |
| `FULL_SALVO` | Data: per-hardpoint `Full_Salvo_Weapon_Delay_Multiplier`; `POWER_TO_WEAPONS` also uses `Power_To_Weapons_Mode_Weapon_Delay_Multiplier` |
| `TRACTOR_BEAM` | Data: `Tractor_Beam_Attack_Ability` range, categories and `Target_Speed_Decrease_Percent`. Needs a living tractor-beam hardpoint; picks the current or nearest enemy in range and slows it while active and in range |
| `MISSILE_SHIELD`, `LASER_DEFENSE` | Data: `Effective_Radius` (750 default), `Defense_Duration_In_Secs` (30). Destroys enemy missiles, torpedoes and rockets inside the radius; the missile shield pins the ship (`Disable_Movement`), laser defence does not. AI sides switch the missile shield on under missile fire |
| `INTERDICT` | Data: a toggle that pins the ship and, while on (or for `Is_Interdictor` units), blocks the enemy `Retreat` command. A field that comes up during the jump countdown cancels the retreat. AI interdictors switch it on while the enemy retreats and off after 20 s (estimate) |
| `STEALTH` | Data: toggle or timed. Estimate: hidden from enemies unless within `stealth_detection` (500) |
| `FOW_REVEAL_PING` | Data: `Effective_Radius`. Estimate: reveals the area for 5 s |
| `CORRUPT_SYSTEMS` | Data: `Activation_Max_Range`, `Duration_In_Seconds`, `Shield_Drain_Per_Second`, `Inaccuracy_Radius`. Drains the shield of an enemy ship at the chosen point; a drained shield stays down the usual 4 s. The AI uses it on the enemy with the most shield in range |
| `Force_Healing_Ability` | Data: hull `Heal_Amount` + `Heal_Percent` of max per `Heal_Interval_In_Secs`, on the ship or friends within `Heal_Range` (single target = most damaged), never on `Excluded_Unit_Types`; ships with destroyable hardpoints heal those (2.3) |
| `Combat_Bonus_Ability` | Data: `Damage_Bonus_Percentage`, `Health/Shield/Defense_Bonus_Percentage` (damage taken divided by 1 + bonus), `Fire_Range_Bonus_Percentage`, `Movement_Speed_Bonus_Percentage`, `Applicable_Unit_Categories` or `Applicable_Unit_Types` (either matches; neither = every ship), never `Excluded_Unit_Types`. Estimate: the data has no radius, so the aura covers the whole side. Per `Stacking_Category` the strongest bonus applies, categories add; negative values (Gorgon -40% shields) are maluses. Applied in tactical battles and auto-resolve |
| `Redirect_Blaster_Ability` | Data: `Reaction_Arc_In_Degrees`, `Block_Chance`, `Redirect_Chance`. Estimate: a redirected shot deals its damage to the shooter instead of re-aiming the bolt. An ability that starts disabled stays off until a story grant (`NEW_POWER_FOR_ALL`) names that unit and ability; `Reflector_Shield` is block 0.15, redirect 0, arc 350 |
| `ROCKET_ATTACK`, `SWAP_WEAPONS` | Data: for the duration (10 s where the data has none, estimate) the ship fires its rocket-mode hardpoint set instead of the normal one (2.5). Estimate, only for ships whose data names no such sets: missile, rocket and torpedo hardpoints reload at 0.4x delay |
| `SENSOR_JAMMING` | Data: `Expiration_Seconds` 15. Enemy homing missiles aimed at the ship lose their lock |
| `BUZZ_DROIDS` | Data: `Damage_Radius`, `Enemy_Damage_Per_Second`, `Activation_Max_Range` (Eclipse lightning storm: 5000, 250 dps). A damage zone at the chosen point for the ability duration (20 s where the data has none, estimate) |
| `SUPER_LASER` | Data: `Fire_Frame` 50, `Damage_Per_Frame` 10, categories. Estimate: destroys the nearest enemy of those categories within 600 units outright after `Fire_Frame`/30 s (10 damage per frame cannot be the one-shot kill). Estimate: a super laser of at most `ION_SUPER_LASER_DAMAGE_PER_FRAME` = 1 (the CIS Subjugator's ion shot, `TEXT_TOOLTIP_ABILITY_SBJ_ION`) does not kill: it hits the target with the ship's longest-stunning ion projectile (`Proj_SBJ_Ion`: 99900 shield damage, no hull damage) and stuns it on detonation (900 s, speed and fire rate -75%). `Projectile_Disable_Engines_Duration` 900 is not modelled (it needs the energy pool) |
| `WEAKEN_ENEMY`, `LURE`, `HARMONIC_BOMB`, `CLUSTER_BOMB`, `ENERGY_WEAPON` | Data: the numbers on the ability and its `Spawned_Object_Type` / `Bomb_Type`. Estimate: `WEAKEN_ENEMY` radius 2000 for 60 s at 25% less damage and speed where the data has none; `HARMONIC_BOMB` radius 750 / 300 and 200 damage; cluster ring placement; energy weapon 500 per frame x 30 fps for 1.5 s |
| `HUNT`, `REPLENISH_WINGMEN` | Estimate: `HUNT` searches an 8x8 grid, 60 s per cell; replenish once per battle |
| `BLAST` | Data: `Blast_Ability` `Charge_Up_Seconds` (Krayt and Merciless 5, Savage 1), `Damage_Multiplier` (Savage 1), `Recharge_Seconds` 90. After the charge every hardpoint with a `Blast_Ability_Fire_Projectile_Type` fires that projectile once (Krayt and Merciless: the mega plasma gun, `Proj_Ship_Krayt_Special_Megaweapon_Damage`, 5000 blast damage instead of 2500; Savage: mass driver and 4 ion cannons); it is reloaded when the charge ends. Those hardpoints fire their `Fire_Projectile_Type` the rest of the time; on a ship without `BLAST` (Death Star) the tag marks a weapon only the super laser uses. Estimates: the blast waits 10 s for those shots; the recharge runs from the end of the charge. AI (`Default_Unit_Target_In_Range`, `PGEvents.lua`): used whenever the target is in weapon range |
| `SHIELD_FLARE` | Data: `Shield_Flare_Ability` (`Viscount_Star_Defender_P4`, `Jury_Rig_Shield`): `Charge_Up_Seconds` 8, `Damage_Radius` 12500, `Max_Damage_Per_Victim` 10000, `Recharge_Seconds` 180; the `Spawned_Object_Type` `Proj_Viscount_Jury_Rig_Shield_Blast` (blast 10000, 40 victims, 5 dropoff tiers, ion stun 900 s without speed or fire-rate cut). After the charge every enemy within the radius (nearest first, at most 40) takes the blast damage of its tier through the normal hit. Estimates: the ship's own shield drops to 0 (the text: "drains the shield for a powerful blast"); the AI (no rule in the Remake scripts) uses it when enemies worth 3 or more (a capital 3) are within the radius |
| `LEECH_SHIELDS` | Data: `Leech_Shields_Ability` (`Ravager`, `Nihilus_Eat_You_Now`): `Activation_Min_Range` 10, `Activation_Max_Range` 2000, `Applicable_Unit_Categories` Corvette, Frigate, Capital plus `Applicable_Unit_Types` `Millennium_Falcon`, `Shield_Damage_Per_Second` 400, `Duration_In_Secs` 30, `Damage_Multiplier` 3, `Recharge_Seconds` 75, `Supports_Autofire`. A beam drains the shield of the current target (or the nearest allowed enemy, or the one nearest to the chosen point) within range; while it runs the Ravager deals 3 times its damage (the text: the drained power "is re-routed to the attacker's weapons"). Estimates: the beam and the bonus end when the target is more than 1.2 times the range away; `Effective_Radius` 1500 on the entry is not used; the AI drains an allowed enemy in range that still has shield |
| `CONCENTRATE_FIRE` | Data: `TF-24_Requiem`: `Effective_Radius` 3000, `Expiration_Seconds` 15, `Recharge_Seconds` 20; its `Concentrate_Fire_Attack_Ability` (`Requiem_Home_One_Ability`; the entry names `Akbar_Home_One_Ability`, which the ship does not have, so the ship's only concentrate-fire sub-ability is used): `Applicable_Unit_Categories` Corvette, Frigate, Capital, `Target_Damage_Increase_Percent` 0.5, `Target_Speed_Decrease_Percent` 0. The caster reloads its weapons and turns them on the current target (or the enemy nearest to the chosen point, or the nearest allowed one) within the radius; for 15 s that ship takes 1.5 times the damage from everyone. No self buff. AI (`Default_Unit_Target_In_Range`, `PGEvents.lua`): at a capital or super ship within the radius |
| `LUCKY_SHOT` | Data: the squadron's `Create_Team_Type` container holds the numbers its craft only name (`Red_Squadron_Container`: `Recharge_Seconds` 60, `CAUSE_DAMAGE_MULTIPLIER` 600, `Supports_Autofire`); a craft's own `Projectile_Types_Override` wins (`Red_Leader_XWing`: `Proj_Ship_Laser_Heavy_R`). The next shot of the craft at its current target (or the enemy nearest to the chosen point) uses that projectile and does 600 times the damage. Estimates: the ability waits 10 s for the shot (`SHOT_WINDOW_SECONDS`, the data has no expiration) and recharges from the use; each craft that names the type has its own (the text names Luke); the AI and autofire use it on a frigate, capital ship, super ship or station whose hull is in weapon range (the Remake scripts have no rule for it) |
| `ION_CANNON_SHOT` | Data: as `LUCKY_SHOT`, from `Hutt_V-Wing_Squadron_Container`: `Recharge_Seconds` 20, `Projectile_Types_Override` `Proj_Ship_Ion_Cannon_Small` (25 shield damage, no hull damage, no stun), no multiplier; each of the 5 V-wings has its own. The tooltip promises a 30 s stun at 50% and 250 shield damage, the projectile data does not; the data is used. AI (`Default_Unit_Target_In_Range`, `PGEvents.lua`): at a frigate, capital or super ship in weapon range |

Not implemented: every other ability type (see [KNOWN-GAPS.md](KNOWN-GAPS.md)). Galactic abilities
(`Planet_Income_Bonus_Ability`, `Galactic_Stealth_Ability`, `System_Spy_Ability`, `Planet_Destruction_Ability`,
`Reduce_Production_*`) belong to `galaxy-sim`.

AI use of abilities (estimate): weapon boosts when the target is in range, after `Try_Offensive_Ability_Space`
(`PGCommands.lua`, data): a full salvo only at a capital ship or a station with more than 20% hull, a faster-fire boost
only at a frigate or bigger, rockets only at a frigate or bigger. Beyond the scripts, AI ships use expiring abilities
with `speed_mult > 1.05` when fleeing or when the target is over twice the weapon range away, `shield_regen_mult >
1.05` when the shield is below 50% and was hit in the last 3 s, and `take_damage_mult < 0.95` when the hull is below
60% under fire. The Remake script lists its ships one by one; we apply its common rule to every ship with the ability.

## 2.7 Projectile combat mods, missiles, rockets

* Data: `Projectile_Combat_Mod` (`COMBAT_MOD_SPEED`, `_FIRE_RATE`, `_DEFENSE`, `_HEALTH`, `_SHIELD` with `_Value`,
  `_Duration`, `_Category_Mask`): the ship hit and every ship within `Projectile_Blast_Area_Range` of the impact get the
  change for the duration. Negative values hit enemies, positive ones help friends. Speed and fire rate are multiplied
  by `1 + value`; damage taken is divided by `1 + value`. Estimate: the strongest value of a kind counts and a new
  hit refreshes the time; they do not stack. Example: the proton torpedo burn, -25% health for 14 s on fighters,
  gunships, bombers, transports and corvettes.
* Estimate, missile arcs: homing missiles (`Projectile_Category` MISSILE with `Max_Rate_Of_Turn` > 0) leave the
  launcher rotated left or right (drawn per shot from the battle seed) by `min(0.6 rad, 0.17 x Max_Rate_Of_Turn)` x
  0.6 to 1.0; the homing turn brings the missile in. The data has no launch angle (`MISSILE_ARC_PER_TURN_RATE`,
  `MISSILE_ARC_MAX`).
* Estimate, rocket weave: `ROCKET` projectiles with `Projectile_Rocket_Curve_Distance` > 0 deviate by
  `Projectile_Rocket_Curve_Offset` degrees following one sine over the curve distance after
  `Projectile_Rocket_Straight_Distance` (data: the three numbers; the shape and the side per shooter are ours).
  Offset 0 (flak, Executor barrage) flies straight.

## 2.8 Reinforcements, unit cap, retreat, victory

* Data: `Command::Reinforce` adds ships at the side's spawn after `reinforce_delay` (5 s = `Skirmish_Reinforcement_Delay_Frames`
  150 at 30 fps) if the population of everything in play plus the request is within `unit_cap` (150, `Space_Tactical_Unit_Cap`
  of the faction XML). Units over the cap wait in a reserve and are called in as losses make room.
* Positional reinforcements (`Command::ReinforceAt`): allowed when the position is inside the battle area, outside the
  no-spawn zone of every enemy structure and inside an area the side sees (`placement.rs`, shared with the UI). Zone
  radius: the structure's `Reinforcement_Prevention_Radius` where the data has it, else `no_spawn_radius` 2000
  (estimate, the number the Remake AI equations use for "near"). Ships arrive at free space (nothing within
  `Space_Reinforcement_Collision_Check_Distance` 150, data), moving out in rings when the spot is taken, facing the
  nearest enemy. Enemy mobile ships do not block a spot.
* Data: retreat becomes available after `Space_Retreat_Allowed_Countdown_Seconds` (60 s) and is blocked by enemy
  interdictors; `Space_Retreat_Attrition_Factor` is 0. Estimate: on the order every mobile unit heads for its own spawn
  point. Data: the jump timer is the faction's `Space_Retreat_Countdown_Seconds` (`FACTIONS.XML`, 30 s for every Remake
  faction; `BattleConfig::side_retreat_jump`, 30 s where a faction has none); at zero every living unit with a
  living engine hardpoint (ships without engine hardpoints count as having drives) escapes and counts as a survivor,
  the others are lost. Units spawned in the battle do not escape on their own: a carrier's launched squadrons escape
  only with the carrier; called-in reinforcements are lost. Pressing Retreat again while the timer runs calls it off
  (`Command::CancelRetreat`).
* A side stays in the fight while any unit it brought lives (ships, squadrons, star base and structures of the setup,
  and called-in reinforcements). Units spawned during the battle (a star base's garrison, squadrons a carrier
  launches from its reserve) do not count and are removed with the loser. A side whose brought units are all
  destroyed or retreated loses. A hand-made scenario without setup units counts any ship. Estimate: a battle is a
  draw after `max_time` (60 min; the data has no limit, this guards headless runs).
* Fog of war (`BattleConfig::fog_of_war`, default on): a ship sees `Space_FOW_Reveal_Range` (data, `ShipDef::sensor_range`,
  1250 where missing); an enemy it saw stays visible for `fog_linger` = `SpaceFOWRegrowTime` (15 s, data). Targeting
  only considers visible enemies. Data: `AIUsesFogOfWarSpace` False (`Xml/Gameconstants.xml`, `Catalog::ai_ignores_fog`),
  so an AI-controlled side plans, hunts and uses its abilities against every enemy, and its ships may engage an unseen
  enemy within their own weapon reach (an aggressive order's unbounded range stays limited to what the side sees,
  estimate); what it sees (the view, the player's fog, reinforcement placement) is unchanged. With fog-bound data the AI
  knows what it sees, and an enemy that damaged a unit of the side in the last `fog_linger` s (data:
  `Default_Unit_Damaged` hands the AI the attacker).
* Arena: `BattleConfig::arena` (from `Arena::load` and the `.ted` map) gives the playable bounds, `Attacker Entry
  Position` markers (attackers at -Y), `Defending Forces Position` markers (defenders at +Y), the `Space Station
  Position` and structure marker spots. Only a prop with `SPACE_OBSTACLE` in a `Behavior` or `SpaceBehavior` tag
  (any copy) and `Space_Obstacle_Radius` greater than 0 is a rock or an effect zone. It is a round obstacle that
  absorbs projectiles and pushes ships away when `Is_Impassable_Asteroid` is true and `Is_Asteroid_Field` is not
  (`Xml/SpaceProps.xml`: `Asteroid_Huge` 280, `Asteroid_Super_Huge` 350, `Star_Destroyer_Construction` 300). The
  centre is the map position plus `Space_Obstacle_Offset` rotated by yaw only (estimate: the full Euler of a tumbling
  prop is unverified). `Is_Asteroid_Field` with a radius is a damage field, not a rock
  (`Asteroid_Field_Huge_Obstacle_Damage_8000` radius 3500). `Is_Nebula` and `Is_Ion_Storm` with a radius are effect
  zones, not rocks (`Nebula_Small` 135; `Nebula_Small_Ion` is both). `Nebula_Ultra_Huge` has both flags and radius
  40000 but `Behavior` `IDLE` only and `In_Background`: a backdrop, no zone. A name that contains "asteroid"
  is not enough: `Asteroid Field Small` has the field flag but its radius is commented out, so it is neither a rock
  nor a field. `Is_Impassable_Asteroid` without `Space_Obstacle_Radius` (`Destroyable_Asteroid_Huge`, footprint 320)
  is not a nav rock. Without an arena fleets start in rows by size class in an empty square of `half_size` = 10000.
  The planet and backdrop props are not obstacles.
* Asteroid fields: a ship is inside an `Is_Asteroid_Field` zone when its centre lies within the radius on the map
  plane. A ship with `ASTEROID_FIELD_DAMAGE` in a `Behavior` / `SpaceBehavior` copy (`Vessel_Base` in
  `Units/Space/_Default_Base.xml` gives it to corvettes, frigates and capitals; `Spacecraft_Base` fighters have none)
  takes `Asteroid_Field_Damage` 200 with probability `Asteroid_Field_Damage_Rate` 0.05 per 30 Hz frame
  (`Gameconstants.xml`: "Probability the damage is applied in each frame"), 1.5 hits or 300 points a second on
  average. The damage type is `Damage_Asteroid` (no armor modifier in the data, so 1.0), shield first, then the
  hardpoints like any hit. Estimate: the hit lands half the ship's radius from its centre in a random direction,
  which picks the hardpoint. View: the unit's `Asteroid_Damage_Hit_Particles` (`Large_Damage_Space` for vessels) and
  `SFXEvent_Damaged_By_Asteroid` (`SFX_Asteroid_Detonation`) where it lands; an own unit entering a field says its
  `SFXEvent_Move_Into_Asteroid_Field` line (estimate: on entering, once per unit type per 12 s; the data does not
  say when the engine plays it).
* Nebulae: a ship with `NEBULA` in a `Behavior` / `SpaceBehavior` copy (`Vessel_Base` and `Spacecraft_Base` give it)
  cannot start a special ability while its centre lies within an `Is_Nebula` zone (map plane) and for
  `Nebula_Ability_Disable_Time` 5 s after it left. Estimate: an ability that already runs keeps running and a running
  toggle can still be switched off (the data names only the disable time). The ability button shows the wait like a
  recharge. An own unit entering a nebula says its `SFXEvent_Move_Into_Nebula` line (same rule as the asteroid line).
  No Remake GC map places a nebula zone (see the asteroid bullet above: `Nebula_Ultra_Huge` is a backdrop).
* Ion storms: a ship whose centre lies within an `Is_Ion_Storm` zone (map plane; the `Nebula_*_Ion` props are also
  nebulae) loses its whole shield, and the shield stays down for `Ion_Storm_Shield_Disable_Time` 5 s after it left,
  then recharges as usual. Estimate: only ships with `NEBULA` are affected (units have no ion-storm behaviour; `NEBULA`
  is their only space-field one, so stations without it keep their shields). The view shows the empty shield bar; the
  data names no ion-storm sound or particle. No Remake GC map places an ion-storm zone.
* Super ship phases: a destroyed ship whose `Destruction_Survivors` names a space unit with a model and hit points (the
  phases of the Executor, Eclipse, Piett's Executor, the Viscount P1 to P5 and the Subjugator Zann P1 to P4, e.g.
  `Viscount_Star_Defender_P2, 1.0`) gives way to that ship in the same battle: at full `Tactical_Health`, where it died,
  facing the same way, with its orders and stance, launching the squadrons it starts with (estimate: the data gives
  only the object and the count). Penalty dummies, map reveals, debuff spawners and destroyable asteroids are not
  ships and are not spawned. The new ship is not a unit the galaxy sent (results group `PHASE_TAG`); the last phase
  leaves its `Death_Clone`.

## 2.9 Navigation, hulls, formations

All values in this section are estimates; the data has no movement-doctrine numbers.

* Static obstacles: rocks and stations form one distance field per battle (250 u cells). A big ship (corvette and up)
  plans an A* path over it, pulls the string, keeps `0.75 x radius + 40` from every rock and station and replans
  every 0.5 s or when the destination moves by more than 300 u. `BattleConfig::navigation = false` restores the old
  behaviour for comparisons.
* Other hulls: big ships look 4 s ahead (at least two radii), steer sideways around hulls in their way (head-on meetings
  pass on the right) and slow down when one is close; a push-out (85% of the two radii, up to 2 x top speed) is the
  hard limit.
* Model hulls: the collision radius is clamped per class (capital 150 to 700), so `ShipDef::hull_shape` is the
  plan-view capsule of the model's visible full-detail meshes (reach ahead and behind, half width) and
  `ShipDef::footprint` falls back to a circle. Big ships keep both apart (`nav::hull_overlap`); escorts keep
  `ESCORT_GAP` 150 from capital hulls.
* Craft and hulls: the volume of every ship (`nav::HullVolume`: the capsule plus the `hull_z` range, grown by `HULL_GAP`
  = `LAUNCH_CLEARANCE` 40 u; a ship without a model shape counts as a circle of 0.45 radius) is closed to every
  fighter, bomber and hangar-launched craft, its own carrier included (they leave it by the hangar flyout). A craft
  inside is moved out the shortest way. Attack runs aim at the point where the line to the aim point meets the grown
  hull (grown by `STRAFE_GAP` 80 u) and start the pass within 180 u of the surface; range to such a target is measured
  to the surface (`target_reach`). A formation slot inside a hull volume is held at the nearest place outside it.
* Formations: each fleet unit remembers its lateral position in the start line (`Ship::lane`, +-1500). Under
  attack-move orders, while the enemy is over 2500 u away, the aim point shifts sideways by that lane so the fleet
  advances line abreast; a big ship does not outrun slower big ships within 2500 u (never below 40% of its speed).
* Group moves (`formation.rs`), project rule: once a move order is given every ship plans its own route to its place
  without collisions and flies there; 20 Star Destroyers stand in a row, tight, in 2 or 3 rows if needed.
  * Slots: a `Move` or `AttackMove` of two or more units (a squadron is one unit; static units and `FormationGrouping`
    `Solo` units stay out) lays out one formation centred on the ordered point, facing the move
    (`Rotate_Formation_Facing_Moves`, data). Bands front to back: `FormationOrder` (data), then escorts (fighters,
    bombers, corvettes, transports) ahead of frigates and capitals (estimate). A band wider than 6000 splits into
    rows of equal count, filled by `Formation_Priority` (data) and by how far ahead units already are.
  * Spacing: neighbours stand `BetweenFormationSpacing` (data, 50; a unit's `FormationSpacing` multiplies it) beyond
    what keeps their hulls apart; Star Destroyers stand 589 u apart, Victorys 383, corvettes 101. A squadron takes the
    room of its members' places around its leader, each member within 50 u of its place, at least 90 u every way.
  * Routes: every ship flies its own path (A*, hull separation). On the last straight leg a big ship or squadron leader
    steers at the point of its slot's line halfway between its place and the slot, so it lines up behind the slot and
    comes in straight. A ship facing too far from where it steers first turns on the spot. It stops 20 u from its
    slot, counts as parked within 30 u and stays parked within 120 u.
  * Facing: a parked ship more than `FinalFormationFacingMinimumAngle` (data, 5 degrees) off the formation's facing
    turns to it. `FinalFormationFacingDeltaCoefficient` and `Max_Formation_Area` are not used (meaning unknown).
  * Pace: space units wait for each other (`WaitsForFormationFormup`, data): each flies the share of its top speed it
    needs to arrive when the last one can, never below 25% (estimate); a fighting or parked unit does not hold the
    group up.
  * Orders: attack-moving units fight what they meet and then go on to their slots. Any other order takes a unit out
    of its formation.

## 2.10 Tactical AI

Source: `XML/AI/GoalFunctions/Remake_SpaceCombat.xml`, `PerceptualEquations/Remake_SpaceCombatEquations.xml`,
`Scripts/AI/Spacemode/*.lua`. The Remake tactical AI is a set of goals (Destroy, Flank, Bomb, Strike, Focus_Weak,
Escort, Reinforce, Retreat); the engine part that matters here is implemented as follows.

* **Retreat** (`Retreat_From_Space`, data thresholds): an AI side orders `Command::Retreat` (check every 2 s) when
  clearly outgunned. Attacker: enemy force >= 2 x own and battle age > 120 s. Defender: enemy >= 3 x own (4 x with a
  living station) and age > 120 s (> 30 s with no station). A side made mostly of transports leaves whatever the odds
  (attacker after 30 s). Also required: `BattleConfig::retreat_allowed` (`Allowed_To_Retreat_Space`: campaign only,
  faction holds more than one planet, never Pirates), the countdown (60 s) passed, no enemy interdiction.
  * Force is the units' `AI_Combat_Power` (data: `Player_AI_Combat_Power_Space_Total`; Executor 65000, Star Destroyer
    1800, MC-80 1750, Victory 1200, Nebulon-B 300, X-wing 10, TIE 4, Rebel star base 3 3750). Estimate: each unit counts
    with the share of hull and shield it has left; units not yet called in count at 0.8
    (`ReinforcementsUnnormalized {Parameter_Attenuator = 0.8}`). Units without the tag count 0.
  * Project rule, emergency retreat: after the countdown, without waiting for 120 s, a side whose survival chance is
    below 25% (enemy force > 3 x own) leaves while capital ships or units of 15+ population are left, or once its
    ships without fighters and bombers have lost half of their peak strength in this battle. A side of fighters and
    bombers only keeps the data rules. A defender with a living station is the exception: it holds until the station
    falls or 120 s have passed; only the half-peak-strength clause sends it away earlier.
  * Not modelled: the Normal/Hard checks of `RetreatPlan.lua` (abort when engines are down; Hard finishes enemies below
    20% first) and `AI_CAN_NOT_RETREAT` while `FocusWeak.lua` runs.
* **Focus weak** (`Prevent_Retreat_On_Priority_Target`, data threshold): a non-fighter enemy with less than 33% of hull
  and shield left inside weapon reach has its priority weight halved (factor 0.5 is an estimate).
* **Fleet plan** (once a second, after `MoveToLocation.lua`, `SpaceTacticalFreeStore.lua`, `ReinforceSpace.lua`):
  * Data: `FREE_STORE_ATTACK_RANGE` 3600, `FREE_STORE_DEFEND_RANGE` 6200, `FIGHTER_AGGRO_RANGE` 3000; the task force moves
    until 5000 from the enemy location; a unit with an enemy warship or structure in weapon range leaves the task
    force; the priority enemy is picked from the classes station/capital, frigate, corvette in that order by the
    weakest-and-nearest scan; aggressive mode (`Allowed_As_Defender_Space_Untargeted`: attackers, defenders without
    structures, defenders whose mobile force is 2x the enemy's after 120 s, a station below 70%, or an own big ship
    near the enemy); the reserve is called in one unit at a time while the cap has room (`Want_To_Reinforce_Units`).
  * Estimate, formation: free big ships form a line with corvettes and transports 700 u ahead, rows ordered by where
    ships stand across the line of advance (room `radius x 2.2 + 60`); the anchor advances at the slowest big ship's
    speed (slowed to 25% while a ship is 500 to 2000 u behind its slot); fighters escort the formation. Once any member
    has an enemy warship within 1.5 x its standoff the whole formation is released and attack-moves on the priority
    target.
  * Estimate, priority target: ships within 3600 u of the side's priority target weigh it at half its priority weight,
    so the priority sets still decide between classes. A defender that holds back focuses the enemy nearest its
    station; a non-aggressive defender guards its most damaged structure.
  * Estimate, search and reserve: with nothing in sight the fleet sweeps search points (90 s, then 45 s each).
    `BattleConfig::ai_reserve` units arrive at the side's reinforcement point, or 1500 u behind the fleet when a
    visible enemy is within 3000 u of it.
  * Data, corvettes: at contact a corvette of an attacking side follows `Service_Guard`: it attacks fighters, bombers
    and transports within its aggro range (longest weapon range, at least `SHIP_AGGRO_RANGE` 3500), else the enemy
    nearest its own frigate, capital ship or station, else stays beside that ship (150 u of room, estimate).
  * Data, free-store targets (`Service_Attack`, Normal): the side's priority target when it is of the kind the unit
    prefers (`Is_Preferred_Target`) and within 3600; else the nearest enemy of the preferred kind in aggro range
    (capital ships prefer frigates, frigates corvettes, corvettes and `AntiFighter` units fighters, bombers and
    transports, fighters fighters and bombers); else the priority target within 3600.
* **Separation** (estimate): mobile non-fighter ships push each other apart when closer than 0.85 x the sum of their
  radii; stations are excluded.
* Not implemented: Flank, Escort and Bombing-run goals as separate behaviours (bombers use their priority sets); kiting
  (`Service_Kite`) and the pull-back of a dying task-force unit; the defensive abilities of `Default_Unit_Damaged`;
  `Determine_Targeting_Priority_Set` (hardpoint sets, tried with no measurable effect); `Default_Space_Targeting_Priority`;
  the map-wide `Find_All_Objects_Of_Type` for the priority target (the priority target is chosen from the enemies the
  AI knows, see 2.8 fog of war).

## 2.11 Death Star II and ground-to-space weapons

* Data: `Death_Star_II` (`SPACEUNITSSUPERS.XML`, `Hardpoints_Empire_Death_Star.xml`) is `In_Background`, has a
  `SUPER_LASER` ability (`Recharge_Seconds` 45, `Expiration_Seconds` 25) and a hardpoint `HP_Death_Star_Superlaser` that
  fires `Proj_Death_Star_SRI` (99999 damage). The hardpoint is the ability's weapon and never fires on its own.
  `In_Background` ships are not targeted and projectiles pass through them.
* Estimate, countdown: the first shot waits for the approach, `TSW_Start_Distance_From_Target` 80000 to
  `TSW_Attack_Distance_From_Target` 50000 at `Max_Speed` 5. The data gives no scale; `TSW_SPEED_SCALE` = 10 (about 600 s)
  was fitted to a Remake gameplay recording. It moves only the first shot.
* Data: `Hypervelocity_Gun` (Empire, `Proj_GroundToSpace_Empire_Turbolaser`, 60000 damage, shield and hull),
  `Seismic_Cannon` (Rebel, nuke blast 9000, range 1750, 2 shots 1 s apart, hull only) and `Nuke_Launcher` (Underworld,
  blast 35000, range 300, hull only) have `Is_Special_Weapon_In_Space`; targets Frigate, Capital, Super; recharge 80 s;
  cost 20000, tech 3. There is no ion cannon in the 4.0 galactic build bar; `Planet_Shield_*` is ground-only.
* Estimate: the first volley comes after 15 s; the blast falls off linearly to 50% at the range edge; nothing disables
  the weapons; ground structures are never lost in a space battle.

## 2.12 Balance: shield regeneration under fire

The outcome of capital duels hinges on one mechanic the data cannot settle: whether shields recharge while under fire.
The Remake comments say the regeneration was boosted 2.5x deliberately. MC-80 (shield 30000, regen 300/s, hull 20000,
no shield-generator hardpoint) versus Star Destroyer (shield 15000, regen 150/s, hull 40000):

| regen while hit | ISD vs MC-80 | 2 v 2 | 5 v 5 | 10 v 10 |
|---|---|---|---|---|
| continuous (default; an MC-80 with shields up often beats an ISD) | MC-80 wins | MC-80, 2:0 | draw | ISD wins, 4 of 10 left |
| paused 2 s after a hit | ISD wins | ISD 2:0 | ISD 4:0 | ISD 6:0 |

A single ISD deals about 290 effective shield DPS to an MC-80 (ideal 827; ion cannons only fire at frigates and larger,
many guns are out of arc or range) against 300/s regeneration, so it cannot break the MC-80 shield alone; the MC-80
(about 210 DPS) does break the ISD shield. With the 4 s `Depleted_Shield_Disable_Time` (data) the 1v1 duel takes about
270 s. Focus fire of several ships flips the result. Hardpoint sniping is off by default; with `attack_move_sets` on,
MC-80 fleets snipe the ISD shield generator and win every size. `cargo run --release -p battle-sim --example
battle_duel` reproduces these numbers.

## 2.13 Estimate constants

Kept in `sim.rs` and `defs.rs`; they shape how ships move, never damage, health, range or speed from the data.

| Item | Value | Why it is an estimate |
|---|---|---|
| Engine hardpoint effect | speed x `0.20 + 0.80 x alive/total engines` | the data has `HARD_POINT_ENGINE` but no rule for losing them |
| Hull gap and keep-out | `HULL_GAP` 40 u, `STRAFE_GAP` 80 u, `SLOT_MARGIN` 10 u, `ESCORT_GAP` 150 u | half the size a fighter is drawn at; collision handling is not in the data |
| Launch | `LAUNCH_CLEARANCE` 40 u, `LAUNCH_EXIT_MAX` 150 u | the data gives bay bones and flyout distance only |
| Missile arc | `MISSILE_ARC_PER_TURN_RATE` 0.17, `MISSILE_ARC_MAX` 0.6 rad | no launch angle in the data |
| Death Star countdown | `TSW_SPEED_SCALE` 10 | no scale from `Max_Speed` to units per second |
| No-spawn radius | `no_spawn_radius` 2000 u | taken from the AI equations' "near" |

The hold of the victory/defeat banner before the summary (`OUTCOME_HOLD_SECONDS` in `battle-view`, 5 s) is a project
rule, not an engine value. Visual-only effects (ribbons, debris, glow, ripples, sensor rings, hyperspace tunnel) do not
change simulation state.

## 2.14 Performance and determinism

Per step: a snapshot of positions and a uniform grid (500 u cells, ship boxes inserted in all cells they overlap), then
rayon-parallel target scans, movement, weapon updates and projectile flight; hit application, upkeep and spawning are
sequential in index order. Randomness comes from per-entity generators seeded by `(seed, tick, id)`, so the result is
identical with or without threads (tested). Energy (`Energy_Capacity`) is not modelled.

# Part 3: Presentation rules

Estimates and data that shape what the player sees and hears. Behaviour is in `battle-view`, `galaxy-play`,
`alamo-audio` and `eaw-ui`.

## 3.1 Audio

* Data: sound events, sample lists, `Max_Instances` (on 90 of 94 presets), `Probability`, `Overlap_Test`, priorities,
  saturation distance (death sounds of corvettes and Star Destroyers carry 450000 to 500000, audible map-wide) and the
  music events per faction (`Strategic_Map_Music_Event`; the Death Star battle theme plays while a `SUPER_LASER` ship
  lives; victory and defeat stingers use `_Vs_Faction` variants). `Unit_Ship_Hyperspace_Enter` plays for reinforcement
  arrivals only. The data has no bus volumes, no space ambience bed and no planet-selected music.
* Estimate: voice limit 48 (the least important or oldest voice is stolen); at an event's `Max_Instances` the quietest
  voice at the listener goes and a request more than 3 dB quieter is dropped; a cut voice fades over 60 ms; the death
  end sound plays 1.5 s after the start sound; falloff `(saturation/distance)^rolloff`; bus gains (weapon shots 0.02 to
  0.07, speech 0.9, music 0.6). Music chain: Galactic, BattleLoading (loop), Space ambient (2 s fade-in), Space battle
  (0.5 s, at the first hostile shot), result stinger (0.35 s) with the 15 s peace delay from `Audio.xml`.
* `alamo_audio::hooks` scans every `Xml/**/*.xml` for tags containing `SFX` or `Music` (39,047 references in 223 tags);
  `hooks::WIRED_TAGS` lists the ones the game plays; `tests/hook_audit.rs` fails on any new unplayable event, with the
  Remake's own broken references in `tests/known_audio_gaps.txt`.
* `SFXEvent_Turret_Rotating_Loop` (86 opening tags in the mod `Xml`, 85 name an event and 1 is empty; none are under
  `Units\Space`) loops while a turret of that unit is turning. Estimate `turret_rotating_epsilon`: the aim yaw or pitch
  moved by more than 0.001 radian since the last sample. The data says the loop plays while the turret rotates and gives
  no dead zone.
* `SFXEvent_Engine_Cinematic_Focus_Loop` (272 opening tags once comments are removed, 266 name an event and 6 are
  empty; 193 of the opening tags are under `Units\Space`) loops while cinematic camera focus is on that unit (C; the
  Space battle cam is not this).
  Estimate `cinematic_focus_stacks`: the loop plays as well as the idle or moving engine loop. The data does not say
  that it replaces that loop. An empty value or `SFX_NA` stays silent.
* `Kills_Previous_Object_SFX` is set on 15 events once XML comments are removed (4 fighter deaths, 3 ground slams,
  3 detonations, 4 deploy or build stops, and `Unit_Magnepulse_Fire`). Starting one stops every live voice whose source
  is that object. A battle shot or death of such an event is sent with the ship entity as its source.
* Ability buttons `special_button_00` through `special_button_23` (24 components) each name `Click_SFX`
  `GUI_Button_Press_2_SFX`. A click plays that event. The battle minimap `radar` (169 by 169) names
  `Mouse_Over_SFX` `GUI_Highlight_Mouse_Over` and no `Click_SFX`. `radar_galactic` and `radar_galactic_adv_map` name
  no click either. Hardpoint reticles are not command-bar components. Those clicks stay silent. `GUI_Zoom` is an event
  and is not a click of the radar.

## 3.2 Cameras and interaction

* Data, tactical camera: pitch 60, fov 60 (`TacticalCameras.xml`); `Tactical_Min/Max_Scroll_Speed` 1000/4000,
  `Push_Scroll_Speed_Modifier` 2.0, `Tactical_Edge_Scroll_Region` 2 (4.0 `Gameconstants.xml`); zoom-out cap 6000.
* Data, galaxy camera: `GMC_InitialPullbackDistance` 470, `GMC_ZoomedPitchAngleDegrees` -20, `GMC_ZoomTime` 1.0 s,
  `GMC_ZoomedPullbackPlanetRadiusFraction` 6.0 (the app does not read all of them yet, see KNOWN-GAPS).
* Estimate: wheel zoom keeps the point under the cursor fixed (factor 0.3 per notch); fleet carry pops 28% in 0.3 s,
  lifts 1.1 pad radii, grows 20%, follows at 22 per second and glides on drop at 14 per second
  (`galaxy-play/src/carry.rs`).
* Data: dying craft with `Spin_Away_On_Death` roll against `Spin_Away_On_Death_Chance` to spin out of control for `Spin_Away_On_Death_Time` seconds with `Spin_Away_On_Death_SFXEvent_Start_Die` before exploding into `Spin_Away_On_Death_Explosion`.
* Data: destroyed hardpoints with `Death_Breakoff_Prop` spawn their break-off prop model, drifting along the prop's `Debris_Movement_Vector` (30 Hz tick scale to units/s) and tumbling along `Debris_Facing_Rotate_Vector` for their configured debris lifetime.
* Data: dying ships without a death clone play the unit's own `Death_Explosions` particle effect when defined, falling back to the class table when absent (`ship_events`).
* Estimate: the hyperspace arrival flash is additive at 0.6 r (no tactical hyperspace effect exists in the data); fighters are drawn
  at their real model scale with a size floor of 3.6% of screen height.
