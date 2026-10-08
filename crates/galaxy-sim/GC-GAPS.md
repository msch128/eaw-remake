# GC feature gaps (space only, Remake 4.0 Web Way GC)

What the Remake data defines for the galactic layer, what `galaxy-sim`/`galaxy-script` model, and what is missing.
"Data" = `Data/XML` of the Remake mod. Numbers are never invented here; rules the data does not state are
named estimates in `RULES.md`.

## Done in this pass (top 3 by gameplay impact)

1. **Hero death and respawn** (data: `Story_Sandbox_Hero_Timers_*.xml`, 18 chains per faction group).
   Chain per hero: `STORY_TACTICAL_DESTROY <hero>` (perpetual) -> `STORY_ELAPSED 300` -> `SPAWN_HERO <hero> <planet>` ->
   bookkeeping triggers (`REMOVE_STORY_GOAL`, `RESET_BRANCH`). `Gameconstants.xml` has `Default_Hero_Respawn_Time -1`,
   so the timers are the only respawn rule. Before: unit losses never reached the story, so heroes stayed dead forever.
   Now: `Event::UnitsDestroyed` -> story `STORY_TACTICAL_DESTROY`; `Event::HeroDestroyed` (from `Is_Named_Hero`) for the UI;
   the story engine counts a perpetual event as done for its dependants, resets whole branches by `<Branch>` tag (and
   everything downstream), and runs prerequisite-only `RESET_BRANCH`/`REMOVE_STORY_GOAL` triggers by themselves
   (inferred: nothing triggers them by name). Respawn happens at the data's planet, or the faction home if the faction
   lost it. galaxy-play shows "X has returned at P" and "X was lost at P".
2. **Production discounts** (data: `Reduce_Production_Price_Ability` / `Reduce_Production_Time_Ability`):
   Mon Mothma (-20 % price, all ships except listed types), the Modular Taskforce Cruiser (-15/-10/-5 % price and
   time by size class), the Starforge special structure (-50 % price, -60..-99 % time by size class, at its own
   planet). `World::unit_cost` applies them to the charge, the credit check and the build time. Same
   `Stacking_Category` takes the strongest, different categories multiply (estimate), heroes work from any fleet
   (estimate). Before: all costs were the list price.
3. **Capture bonus** (data: `Planet_Capture_Bonus_Reward`, 147 planets, 900..2400 credits):
   first conquest of a planet by a faction pays the amount (`Event::CaptureBonus`, `SimConfig::capture_bonus_scale`,
   once per planet and faction; the "once" and the exact trigger are estimates). Before: ignored.

## Heroes

| Topic | Data | State |
|---|---|---|
| Galactic movement | heroes are `UniqueUnit` space ships; they travel in fleets like any ship | works (they are units) |
| Death / respawn | see above | done |
| Respawn dialog | `Dialog_Respawn_*` | shown through the story event |
| Abilities | `Unit_Abilities_Data` (STEALTH, FULL_SALVO, DEFEND, TURBO, ...) | tactical only: battle-sim has generic ability rules |
| Galactic abilities | `Mothma_Remove_Corruption`, `*_Neutralize_Hero` (Boba, Bossk, IG-88, Mara), `Han_Solo_Siphon_Credits`, `Corruptor_Corrupt_Planet_1` in `Gameconstants.xml` | missing, see Underworld |
| Hero cost modifiers | `General_Minor/Major_Hero_Cost_Mod`, `Owner/Target_Respawn_Time_In_Secs` (240, 300) in `GENERICHEROUNITS.XML` | unused: the Remake's story timers replace them |

## Underworld mechanics (FoC)

Present in the engine constants (`Gameconstants.xml`: corruption, bribery, siphon, neutralise, black market) and in
the Underworld AI templates, but the Web Way GC runs `Planet_Black_Market_Modifier` = 0 for most planets and the Remake
story plots contain no corruption/bribery reward types (reward types in the plots: RESET_BRANCH, SPAWN_HERO, REMOVE_UNIT, LOCK_UNIT,
NEW_POWER_FOR_ALL, SET_TECH_LEVEL, REMOVE_STORY_GOAL, STORY_GOAL_COMPLETED, FORCE_CLICK_GUI, TRIGGER_AI, DISABLE_BRANCH, UNIQUE_UNIT,
PLANET_FACTION, BUILDABLE_UNIT). 8 planets define `Planet_Corruption_Removal_Base_Cost`, 6 `Planet_Black_Market_Influences`.
Status: **not modelled** (corruption, bribery, smuggling/siphon, raids, neutralise). Checked against the loaded Remake data (space-only play): none of these is reachable. `Black_Market_Ability`, `Neutralize_Hero_Ability`, `Siphon_Credits_Ability`, `Corrupt_Planet_Ability` and `Remove_Corruption_Ability` exist only on ground hero objects (`Ground_Vanilla/Heroes`, `Non-Combat_Heroes`, the named ground heroes), never on a space hero or ship, so the fleet special actions of `Gameconstants` (`Activated_*_Ability_Names`) cannot be triggered; the 10 black market items need the black market ability; corruption only arises from the corrupt ability. The space heroes of the Underworld (Boba, IG-88, Bossk, Karrde, Urai, Tyber Zann, Xizor, Noira) carry `Planet_Income_Bonus_Ability` and `Planet_Income_Gambling_Ability` (modelled), `Galactic_Stealth_Ability` (22 abilities on 12 units, Underworld: Xizor; `Evade_Detection_Chance` 0 everywhere) and `System_Spy_Ability` (Boba, Bossk, IG-88, Karrde and others: what is known about the planet the fleet is at) which are information features: there is no galactic fog in the sim, so nothing to hide or reveal yet. `Planet_Starbase_Stealth` is `No` in every one of the 45 planet definitions, so it has no effect. Tactical: `CORRUPT_SYSTEMS` (shield drain, 2 units) is modelled in battle-sim. They are ground/agent systems driven by
GUI abilities of agent units; gameplay impact in a space-only GC is low. Next candidate: Siphon/Corrupt as
fleet-less agent commands if the Underworld faction becomes playable.

## Planet bonuses and structures

* `Planet_Ability_Name` (INCOME 7, PRODUCTION_BOOST 2, ACADEMY 2, BM 2, REPUBLIC/CIS 3 each, plus ~25 flavour keys): text only, the
  effect is carried by the planet's structures and `Planet_Income_Bonus_Ability` (modelled since the first rules pass).
* `Planet_Credit_Value`, `Planet_Destroyed_Credit_Value`, `Absolute/Percentage_Income_Modifier`: modelled.
* Production discounts of structures (Starforge): done. Other `Reduce_Production_*` are ground structures (barracks, factories): out of scope.
* `Combat_Bonus_Ability` / `Defense_Bonus_Percentage` (4 uses: ground power generator, rebel asteroid bases): not modelled in galaxy-sim; space bases carry only the fighter bonus (+2.5 %).
* `Planet_Tech_Availability_Rating`, `Planet_Slice_*`: ground-map generation, irrelevant.
* `Special_Structures_Space/Land` slot counts: modelled.

## Space stations

Star base levels (`Buildings/Space/GC/**`, Golan_1..3, etc.) are modelled as a chain with cost, build time, health and weapons,
used by the tactical resolver (base fights, repair per level). Not modelled: stealth (`Planet_Starbase_Stealth`, 10 planets),
station-specific unit unlocks beyond `Required_Star_Base_Level`.

## Trade routes / supply

`Xml/TradeRoutes_Web_Way.xml` fields are all zero/ignored (`Political_Control`, `Credit_Gain`): no supply system exists in the data
(RULES.md section 2). Nothing to add.

## Death Star / superweapons

`Death_Star_Planet_Destruction` ability (named in `Gameconstants.xml` `Activated_Destroy_Planet_Ability_Names`) and
`Planet_Is_Destroyable_By_TSW` (10 planets Yes, 7 No), `Planet_Destroyed_Credit_Value`, Galaxy Gun unit. Not modelled: the weapon is
an activated ability on a mobile unit that destroys a planet (planet becomes an asteroid field, owner loses its income, destroyed-credit
value remains). **Next best candidate** (small: owner None, structures cleared, income `Planet_Destroyed_Credit_Value`, a `Command::DestroyPlanet` gated on a Death Star unit being present).

## Galactic events / notifications

* Story events (STORY_ELAPSED, TACTICAL_DESTROY, CONQUER, CONSTRUCT, TECH_LEVEL, WIN/LOSE_BATTLES, AI_NOTIFICATION, GENERIC, CLICK_GUI) are run by `alamo-lua`;
  the host feeds CONQUER, CONSTRUCT, TECH_LEVEL and now TACTICAL_DESTROY. `STORY_WIN_BATTLES`/`STORY_LOSE_BATTLES` (ship phase upgrades
  in `Story_SSD_*_Damage.xml`, Pirate interventions) are **not fed** yet: next small step (`Event::BattleWon/BattleLost` exist).
* The `INTERVENTION_*.XML` random events (pirate ambush, smuggler, revolt, refugee escort, ...) are not part of the Web Way plot list that
  the campaigns load, so they are not triggered.
* Notifications: galaxy-play describes construction, battles, captures, hero loss/return and the capture bonus.

## Update: superweapons and battle story events

* **Planet destruction** (done): `Command::DestroyPlanet` for a fleet holding a unit with `Planet_Destruction_Ability` (Death_Star, Death_Star_II, Galaxy_Gun_*),
  at a planet that is not the firer's own/allied, `Planet_Is_Destroyable_By_TSW` not `No` (planets without the tag count as destroyable: estimate),
  not already destroyed. Effects: planet neutral, buildings and queues gone, `PlanetState::destroyed` (nobody can capture it), all other ships in orbit lost,
  `Event::PlanetDestroyed`. The weapon is not consumed; it recharges `SimConfig::superweapon_cooldown_seconds` (120 s = 2 weeks, estimate). Death Star II costs
  1,000,000 and needs tech 5 / star base 3 in the data. AI: the Remake's galactic `Use_Death_Star` goal is commented out in `GC_Invade_Planets.xml`
  (only the tactical `Fire_Death_Star` exists), so the galactic use is player-only (`PlayAction::DestroyPlanet`). The tactical super laser is not modelled.
* **STORY_WIN_BATTLES / STORY_LOSE_BATTLES** (done): counted per plot faction for space battles (`FILTER_FRIENDLY_ONLY`, `Event_Param1` = number needed), fed from `BattleWon`/`BattleLost`.
