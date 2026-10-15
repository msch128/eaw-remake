# Audit: space structures, roster and build limits (Remake 4.0), 2026-10-15

Scope: the four questions of the lead. Every line says "verified" (data line plus our code plus a test or a headless run) or "not verified".
Data paths are relative to the workshop item 1770851727 (`Data/XML/...`); the Clone Wars stack is the `fft` alias (3229239424 + 2794270450 + 1770851727).

Tools used (all headless, offscreen, no window, no audio), kept in the repo as examples:

| Tool | What it prints |
|---|---|
| `crates/galaxy-sim/examples/audit_roster.rs <mod> <campaign> [filter]` | per faction: every ship with requirements, planets that can ever build it, structures, star bases; `*` lists every structure with space combat |
| `crates/galaxy-sim/examples/audit_catalog.rs <mod> <campaign>` | per faction: every ship of the build bar, does the battle catalog know it, model file present, hull, shield, hardpoints, dps |
| `crates/galaxy-sim/examples/audit_battle.rs <defender> <attacker> <unit> <n> <star base level> [structures]` | runs a headless tactical battle and prints what spawned, special weapon cooldowns, attacker hull over time, result |

## Verdicts

| # | Question | Verdict |
|---|---|---|
| 1a | Hypervelocity Gun, Seismic Cannon (the Rebel ion cannon of 4.0), Nuke Launcher work in space battles | **Verified** in battle-sim with data numbers. |
| 1b | They work in auto-resolve | **Not working.** The structures have no combat numbers, so the statistical resolver skips them (gap G4). |
| 1c | Planetary shield | **Not a space structure.** Ground-only (`BASE_SHIELD`, radius 1600, ground reinforcement block), `Tech_Level` 99, never offered. Nothing to do. |
| 1d | Any other structure with a space effect | Fortresses, watchtowers, shipyards, satellite grids, Starforge, trade hub: **verified** in battle. **Golan platforms and the Uplink Command control station do not appear in any battle** (gap G1). |
| 2 | Star base levels, defence satellites, shipyards in battle with data stats | Star base (all 3 levels), satellite defence grid, shipyards: **verified**. Golan I/II/III: **missing** (G1). |
| 3 | Roster coverage | Every ship of the three build bars spawns with model, hull, shield and weapons (0 problems of 298 ships). Missing: all hero-ship upgrade items (G2), Death Stars are scripted only, Executor/Eclipse phase ships (P2 to P5) are not modelled (G6). |
| 4 | Build limits per planet | **Verified**: all 145 real planets take star base level 3 and shipyards 1 to 3. The user is **partly right**: some super ships are tied to few planets, but not all. Exact list below. Enforced by `check_build_unit`; test added (`tests/super_ships.rs`). |

## 1. Ground structures that act in space battles

### 1.1 What the data says (4.0 Galactic Conquest objects, loaded by `GameObjectFiles.XML` lines 36 to 105)

| Structure | Faction | File | Req. | Cost | Tech | Weapon | Valid targets | Recharge | Per volley |
|---|---|---|---|---|---|---|---|---|---|
| `Hypervelocity_Gun` | Empire | `Buildings/Ground/GC/Empire/Hypervelocity_Gun.xml:3-60` | `Empire_Base_Core_*`, 1 per planet | 20000 | 3 | `Proj_GroundToSpace_Empire_Turbolaser` (`PROJECTILES.XML:780`): `Projectile_Damage` 60000, `Damage_Default`, shield and hull | Frigate, Capital, Super | 80 s | 1 shot |
| `Seismic_Cannon` | Rebel | `Buildings/Ground/GC/Rebel/Seismic_Cannon.xml:42-51` | `Rebel_Base_Core_*` | 20000 | 3 | `Proj_GroundToSpace_Rebel_Nuke` (`PROJECTILES.XML:700`): blast 9000, range 1750, up to 50 victims, hull only | Frigate, Capital, Super | 80 s | 2 shots, 1 s apart |
| `Nuke_Launcher` | Underworld | `Buildings/Ground/GC/Underworld/Nuke_Launcher.xml:42-51` | `Hutt_Palace_*` | 20000 | 3 | `Proj_GroundToSpace_Consortium_AP_Missile` (`PROJECTILES.XML:742`): blast 35000, range 300, 30 victims, hull only | Frigate, Capital, Super | 80 s | 1 shot |
| `Planet_Shield_E/R/U` | all | `Buildings/Ground/GC/Planet_Shield.xml` | base core | 2000 | **99** | `BASE_SHIELD`, ground only | n/a | n/a | n/a |
| `Ground_Ion_Cannon`, `Ground_Empire_Hypervelocity_Gun`, `Space_Mode_Ground_*` | old | `SpecialStructures.xml:4, 1853, 5507, 5586` | | | **99** | old skirmish/space-mode copies | | | never offered |

There is no "Ion Cannon" in the 4.0 galactic build bar: the Rebel ground-to-space weapon is the Seismic Cannon (the old `Ground_Ion_Cannon` is a Tech 99 neutral dummy). `Is_Special_Weapon_In_Space` is set on exactly these three (plus the old copies).
`Planet_Rocket_Tower` and `Tower_Defense_Grid` are ground anti-air and ground turrets (`TURRET`, `Turret_Targets_Air_Vehicles`), Tech 99 or Required_Planets only, no space effect.

### 1.2 Our code

* `alamo-data` reads the fields; `battle-sim/src/defs.rs:1348-1380` builds `SpecialWeaponDef` (projectile, recharge, pulse count and delay, valid target mask, `Space_Victory_Relevant`).
* `battle-sim/src/sim.rs:260` places the structure behind the station, `sim.rs:2958-3010` fires it (target score: class first, then health), `sim.rs:2985` makes it untargetable (`!other.special`), `sim.rs:1286-1298` applies the blast (linear falloff to 50 percent at the range edge, an **estimate**).
* `galaxy-sim/src/catalog.rs:220` marks them `space_reach`, so they join the battle (`sim.rs:954`).
* The first volley comes after 15 s (`sim.rs:445`, `special_cd = min(15, recharge)`, an **estimate**: the data has no start delay).
* Ground structures are never lost in a space battle (`galaxy-sim/src/sim.rs:1067`).

### 1.3 Headless runs (`audit_battle`, 3 attacking capital ships, planet without star base)

| Weapon | Command | Observed | Data expectation | Result |
|---|---|---|---|---|
| Hypervelocity Gun (Empire, Anaxes) | `audit_battle Empire Rebel MC-80_Star_Cruiser 3 0 Hypervelocity_Gun` | first shot at 15 s, one MC-80 shield 30000 to 12216 (-17784); next shot 80 s later (`cd75` after 5 s) | 60000 x 0.33 (`Damage_Default` vs `Shield_Starship`, `Gameconstants.xml:1012`) = 19800 | verified (the rest is shield regeneration) |
| Seismic Cannon (Rebel, Dantooine) | `audit_battle Rebel Empire Star_Destroyer 3 0 Seismic_Cannon` | volleys at 15, 95, 175 s (80 s apart); about 9 to 11k hull per volley on the Star Destroyers in the blast | 9000 blast, 2 shots | verified |
| Nuke Launcher (Underworld, Atzerri) | `audit_battle Underworld Empire Star_Destroyer 3 0 Nuke_Launcher` | volleys every 80 s; one Star Destroyer loses 4438 hull | 35000 x 0.33 = 11550 at the centre | works, lower than the data: the hit goes through the hardpoint model (the same for every weapon), blast range 300 rarely reaches more than one ship |

The existing test `galaxy-sim/tests/special_weapon.rs` covers the Hypervelocity Gun and the old `Ground_Ion_Cannon` only. **Seismic Cannon and Nuke Launcher had no test** (G7).

### 1.4 What disables them, balance

* Data: `POWERED`, `Requires_Base_Power`, `Energy_Capacity` 1000 / `Energy_Refresh_Rate` 6 on the Hypervelocity Gun, `ION_STUN_EFFECT`, `STUNNABLE`. All are ground-battle rules; `Projectile_Energy_Per_Shot` of the three shots is 0, so power never limits the space shot.
* Our sim: nothing disables them (they cannot be targeted, they need no power). That matches the space-battle side of the data. **Not verified against the real engine** (closed source).
* Player control: the special weapon fires on its own for both sides; there is no button to aim it (the real game has `b_special_weapon`). Low impact (G8).

## 2. Space structures in battle

Headless run `audit_battle Rebel Empire Star_Destroyer 3 3 R_Shipyard_3,R_GOLAN_1_SPAWNER,R_Satellite_Defence_Grid,Seismic_Cannon`:

| Object | In the battle? | Hull / shield / hardpoints in battle | Data |
|---|---|---|---|
| `Rebel_Star_Base_3` | yes | 65500 / 30000 / 83 | `Buildings/Space/GC/Rebel/Colonies.xml:267-268` Tactical_Health 65500, Shield_Points 30000 (levels 1 and 2: 24000 and 36000) |
| star base garrison squadrons | yes (X-Wing_G, Y-Wing_G, Z-95_R ... as groups of 12 to 24) | per squadron member | `Starting_Spawned_Units_Tech_0` |
| `R_Shipyard_3` (also 1, 2) | yes | 12500 / 10000 / 10 | `Buildings/Space/GC/Shipyard_3.xml` Tactical_Health 12500, Shield_Points 10000 |
| `R_Satellite_Defence_Grid` | yes | 22500 / 0 / 15 | `Satellite_Defence_Grid.xml` |
| fortresses (15000 hull, 3 weapons), watchtowers (3000, 2), Starforge (762000 / 300000), trade hub, housing, research institute | yes (`audit_roster '*'`) | from the hardpoint data | |
| **`R_GOLAN_1_SPAWNER` / `E_Golan_1..3`** | **no** | Golan I: 90000 / 30000 / 3 weapons, II: 135000 / 50000 / 4, III: 148500 / 70000 / 3 (the `*_Golan_N_Orbital` objects) | `Golan_N.xml`: `E_Golan_N` has `Place_Other_Type_At_Every_Specific_Marker_Position` = `E_Golan_N_Orbital`; only the Orbital object has hardpoints |
| **`E_Uplink_Command`** | **no** | control station 18000 / 15000 with a garrison | `Uplink_Command.xml:166-176`: `Tactical_Additional_Structure_Type` = `E_Control_Station` |

Cause: `galaxy-sim/src/catalog.rs:200-260` builds combat numbers only from the structure's own hardpoints. The structure that stands in the galaxy (`E_Golan_1`) has none; its battle form (`E_Golan_1_Orbital`) is `Tech_Level` 99, cost 0, not buildable, and nothing links the two. Golan platforms stand at Coruscant, Endor, Empress Teta, Kuat, Loronar from the start (`Campaigns_Imperial_Web_Way_4Grand.xml:437-539`, `Starting_Forces ... E_Golan_1`), for all minor factions too, and `E_GOLAN_1_SPAWNER` (Tech 2, 52000, star base 1, 17 planets) is in the build bar. A player who builds one gets nothing in battle.

Golan planets (`Golan_N.xml:12-15`): SJWorlds, Hapes, Csilla, Kaas, Etti_IV, Endor, MonCalamari, Yavin, Bespin, Coruscant, Corellia, Kuat, Kashyyyk, Kamino, NalHutta, The_Maw, Onderon (Coruscant2 is not in this galaxy). These are exactly the planets with 7 or 9 orbital slots.

## 3. Roster coverage

### 3.1 Build bar by faction (4.0, `audit_catalog`)

| Faction | Ships in bar | Tech 99 (MP, skirmish, story) not offered | Problems found (no model, no hull, no weapons) |
|---|---|---|---|
| Rebel | 102 | 188 | 0 |
| Empire | 100 | 206 | 0 |
| Underworld | 96 | 176 | 0 |
| Minors (Mandalorians 8, Hapan 5, CIS 11, CSA 14, Republic 17, Warlords 23, Cartels 12, Sith 8) | 98 | n/a | 0 |
| Clone Wars stack (`fft`): Republic 90, CIS 89, Rebel 124, Empire 109, Underworld 125 | 537 | n/a | 0 |

"0 problems" means: the battle catalog knows the name, the `.alo` model exists in the VFS, hull above 0, and the ship has weapons, fighters or a hull weapon. Two ships have no direct damage by design (`Broadside_Cruiser_DB` fires Magpulse missiles, `Corona_Corvette`); hero fighters (Luke X-wing, Boba Fett, Katarn) shoot with a hull weapon.
One odd value: `Subjugator_*` reports a bearing damage sum of 3.5e8, probably from its `Super_Laser_Ability` (`Units_Space_Neutral_CIS_Subjugator_Battlecruiser.xml:175`). A headless run of a Subjugator against the Dantooine fleet crashed at 55 s (G3), so the superlaser itself is **not verified**.

### 3.2 Super capital and heavy ships (4.0, Rebel campaign)

"Hosts" = real planets where `check_build_unit` can ever pass (with the needed shipyard). All also need the shipyard chain (level 3 shipyard = star base 2 + shipyards 1, 2, 3) and the tech level.

| Ship | Faction | Tech | Cost | Build limit | Data `Required_Planets` | Hosts | Spawns in battle with model and weapons |
|---|---|---|---|---|---|---|---|
| `Executor_Star_Dreadnought_P1` | Empire | 5 | 1,250,000 | 1 alive, 1 lifetime | **none** (`Units_Space_Empire_Star_Destroyer_Executor.xml:51-65`: only `E_Shipyard_3`) | **145 (any)** | yes (hull 150000, 158 hardpoints) |
| `Eclipse_Star_Dreadnought_P1` | Empire | 5 | 1,500,000 | 1 / 1 | **Byss, Kuat, Fondor** | **3** | yes (500000 hull, 151 hardpoints) |
| `Annihilator_Super_Star_Destroyer`, `Puifier_Super_Star_Destroyer` | Empire | 5 | 400,000 | 1 / 1 | none | 145 | yes |
| `Titan_Star_Destroyer`, `Torpedo_Sphere` | Empire | 4 / 5 | 125,000 / 100,000 | 8 / none | none | 145 | yes |
| `Arc_Hammer` | Empire | 4 | 150,000 | 1 / unlimited | Kuat, Fondor (and `E_Uplink_Command`) | 2 | yes |
| `Viscount_Star_Defender_P1` | Rebel | 5 | 1,000,000 | 1 / 1 | none | 145 | yes (hull 150000, 111 hardpoints) |
| `Subjugator_E/R/U` (CIS: `Subjugator_CIS`) | Empire, Rebel, Underworld, CIS | 4 (CIS 3) | 600,000 | 1 / 1 | **Mustafar, Muunilinst, Cato Neimoidia, MonCalamari** | **4** | yes (hull 375000, 213 hardpoints) |
| `Subjugator_Zann_P1` | Underworld | 5 | 600,000 | 1 / 1 | none | 145 | yes |
| `Praetor_Battlecruiser_E/R/U`, `Mandator_II_E/R/U` | Empire, Rebel, Underworld | 4 | 500,000 / 300,000 | 1 / 1 | **Ringo Vinda, Rendili, Rothana, Allanteen** | **4** | yes (137 and 95 hardpoints) |
| `Lucrehulk_Battleship_*` | E, R, U | 4 | 100,000 | none | Mustafar, Muunilinst, Cato Neimoidia, MonCalamari | 4 | yes |
| `Death_Star_II` | Empire | 5 | 1,000,000 | 1 / 1 | none, but **not in any build bar** (`Build_Tab_Space_Units` off; scripted by the Death Star story) | 0 | hull 500 (an under-construction dummy) |
| `Death_Star`, `Executor_Star_Dreadnought`, `Eclipse_*_MP`, `Viscount_Star_Defender`, `Subjugator_Zann`, `Lusankya` | | 99 | | | | | story or multiplayer variants, never offered |
| `Galaxy_Gun_E/R/U` | | 99 | | | empty template (`Units_Space_Empire_Galaxy_Gun.xml`: no model, no hardpoints, no cost) | | not a unit in 4.0 |

Clone Wars stack differences (`audit_roster fft GC_Full_Web_Way_Republic`): `Eclipse_Star_Dreadnought_P1` is Tech 99 (not offered); `Viscount_Star_Defender` (900000 hull) only at **MonCalamari**; `Executor_P1` any planet with `E_Shipyard_3`; `Annihilator` 750,000 any planet; `Praetor_Battlecruiser_E` 8 planets (Kuat, Fondor, Rendili, Rothana, Corellia, Allanteen, Coruscant, Ringo Vinda); `Subjugator_CIS/II` 5 planets (4.0 four plus Geonosis); Republic and CIS have no super ship in the bar.

### 3.3 Who can build what in groups (4.0 build bar, planets per `Required_Planets`)

| Planets | Rebel | Empire | Underworld |
|---|---|---|---|
| any (145) | 49 (MC-80, Nebula, Liberator and Quasar loadouts, Endurance, Dauntless, Viscount P1 ...) | 46 (Star Destroyers I/II/Ion, Allegiance, Dominator, Titan, Annihilator, Puifier, Executor P1 ...) | 41 (Hutt ships, Sabaoth, Recusant II, Zann Subjugator P1 ...) |
| Ringo Vinda, Rendili, Rothana, Allanteen | 9 (Venator, Secutor, Praetor, Mandator ...) | 12 | 11 |
| Mustafar, Muunilinst, Cato Neimoidia, MonCalamari | 8 (Subjugator, Recusant, Providence, Lucrehulk ...) | 7 | 7 |
| Kuat, Fondor | 8 (Star Destroyer, Victory, Ton Falk, Immobilizer, Gladiator ...) | Arc Hammer | 8 |
| Mandalore / Malachor V / Lehon | 13 | 13 | 11 |
| Sullust, Hapes, Naboo, Kaas, Etti IV, Bothawui, Corellia ... (single-planet specials) | 12 | 14 | 14 |
| Byss, Kuat, Fondor | | Eclipse | |
| MonCalamari only | Profundity, Liberty, Independence, Ackbar_Wingless | | |
| NalHutta, Yag Dhul | | | Tyber Zann the Merciless, End of Days |

### 3.4 Heroes' ships

* In the bar (hero flag, needs `Uplink_Command`): Rebel `Rebel_One`, `Liberation`, `Fortressa`, `Independence`, `Liberty`, `Profundity`, `Ackbar_Wingless`, `Errant_Venture`; Empire `Piett_Procursator`, `Strikefast`, `Vensenor`, `Singularity`, `Gorgon`, `Predominant`, `Admiral_Hoytt`, `Arc_Hammer`; Underworld `Sykes_Venator`, `Tyber_Zann_The_Merciless`, `End_of_Days`.
* **Not reachable**: the upgraded forms, which are bought as `Vessel_Upgrade_Template` items (`Units/Space/_Default_Base.xml:317`, handled by `Scripts/GameObject/Upgrade_Units.lua`): `Piett_Procursator_Upgrade` (25000, Kuat, needs `Required_Orbiting_Units` Piett) to `Accuser`, then `Piett_Accuser_Upgrade` to `Piett_Executor_P1`; `Black_Squadron_Upgrade` to `Devastator`; `Strikefast_Upgrade` to `Admonitor`, `Admonitor_Upgrade` to `Admonitor_II`; `Executrix_Upgrade` (Havelon); `Ackbar_Upgrade` to `Home_One`; `Jabba_Upgrade` to `Jabba_Destroyer`; `Jared_Sykes_Upgrade`; `Tyber_Zann_Upgrade`; the 181st and Defender squadron upgrades. These items have no `Space_Model_Name`, and `alamo-data/src/space.rs:954-960` (`space_units`) drops every unit without a space model, so they are not in the catalog at all (`audit_roster` shows none; `galaxy-sim/RULES.md` line 66 describes them as if they existed). The upgraded hero ships are Tech 99 and not buildable, so **Home One, Devastator, Accuser, Piett's Executor, Admonitor, Executrix, Jabba's Destroyer never exist** (G2).

## 4. Build limits per planet

### 4.1 Data (`Planets_WW.xml`) and rules

| Planet group | Count | `Max_Space_Base` | Orbital / ground structure slots |
|---|---|---|---|
| normal worlds (Aargau ... Zeltros) | 119 | 3 | 6 / 4 |
| major worlds: Bespin, Corellia, Coruscant, Coruscant2, Kamino, Kashyyyk, Kuat, Mandalore, NalHutta, Onderon, Tatooine2, The_Maw, Yavin | 13 | 3 | 7 / 6 |
| big worlds: Csilla, Endor, Etti_IV, Hapes, Kaas, MonCalamari, SJWorlds | 7 | 3 | 9 / 8 |
| stations and asteroid fields: Bilbringi, Junkfort, MalachorV, Polis, Roche, Smugglers_Run, Syvris, Vergesso | 8 | 3 | 6 / 0 |
| faction dummy planets | 11 | 1 | 0 / 0 |

(`audit_roster` counts 145 real planets with cap 3 in the Rebel campaign: the campaign leaves some of these out.) **Every real planet can reach star base level 3 and can hold the three shipyard levels** (`Shipyard_N.xml`: `Required_Star_Base_Level` 2, no `Required_Planets`; hosts 145 for `E_Shipyard_1/2/3`). The user's belief that only some planets can upgrade a star base or shipyard is **not what the data says**. What is restricted:

| Structure | Where | Data |
|---|---|---|
| Golan I, II, III | 17 planets (list in section 2) | `Golan_N.xml:12-15` |
| Starforge (1,000,000) | Lehon only | `Starforge.xml` |
| Gas refinery, gas depot | Bespin, Yavin, Reltooine, Utegetu Nebula | `Gas_Refinery.xml:23-26` |
| Fortress (urban, temperate, barren, sector) | 2 + 1 + 2 + 8 planets | `Sectors/Fortress.xml` |
| Watchtower | 130 planets | `Sectors/Watchtower.xml` |
| Ships | see section 3 | `Required_Planets` of each unit |

### 4.2 Our enforcement

`galaxy-sim/src/production.rs:check_build_unit` (lines 175-243): `req_planets`, `Max_Space_Base` via `planet_takes_starbase`, `Required_Star_Base_Level`, `Required_Special_Structures` (groups and alternatives), limits, population, credits. Existing tests: `build_rules.rs::only_listed_planets_build_a_restricted_ship` (Star Destroyer: Kuat yes, Coruscant no), `small_planets_cannot_reach_high_levels`, `planet_star_base_caps_come_from_the_planet_data`.
New in this audit: `galaxy-sim/tests/super_ships.rs` checks, for every super and heavy ship above, on all real planets with a full shipyard chain, that the ship passes exactly on the planets of its `Required_Planets` (Eclipse: Byss, Kuat, Fondor only; Executor: everywhere; Subjugator: four planets; Praetor and Mandator: four planets).

## Gaps, ranked by how much a player notices

| Rank | Gap | Effect for the player | Where | Fix size |
|---|---|---|---|---|
| G1 | Golan platforms and the Uplink Command control station never appear in battle or auto-resolve | Coruscant, Kuat, Endor ... lose their signature defence; 52000 to 130000 credit buildings do nothing | `galaxy-sim/src/catalog.rs` (battle form of a structure), `tactical.rs` | small: give a structure the combat numbers and name of its `Place_Other_Type...`/`Tactical_Additional_Structure_Type` form |
| G2 | Hero ship upgrade items are missing from the catalog | Home One, Devastator, Accuser, Piett's Executor, Admonitor, Executrix, Jabba's Destroyer, End of Days upgrade cannot be reached; heroes stay at their weak first form | `alamo-data/src/space.rs:954` drops them; `galaxy-sim` has no "replace a unit in the fleet" item | medium: new item role + `Required_Orbiting_Units` check (galaxy-sim; menu in galaxy-play is not mine) |
| G3 | Ships with more than 64 hardpoints break hardpoint targeting: a bomber attack on a Subjugator (213), SSD (185), Executor (158), Eclipse (151), Praetor (137) or a star base (83) panics in debug builds (`attempt to shift left with overflow`, `battle-sim/src/sim.rs:2171`) and in release mixes up hardpoints 64+ with 0+ | Bombers aim at the wrong hardpoints of every super ship and station; debug builds and tests crash | `battle-sim/src/sim.rs:35, 1009, 2160-2171, 2262, 2290, 2731, 2814, 2885` (`hp_alive: u64`) | small: 256-bit mask |
| G4 | Special weapon structures are skipped by auto-resolve | An AI planet with a Hypervelocity Gun is as weak as an empty one when the player auto-resolves | `galaxy-sim/src/combat.rs` (`stats_of` returns none), `catalog.rs:178` | small to medium: weapon from the projectile, structure not targetable |
| G5 | Super ship phases not modelled: `Destruction_Survivors` (Executor P1 to P2 ...), `Repair_Phase_*` items, `Death_Star_II` is a 500-hull dummy | Losing an Executor ends it; the data lets it survive as a damaged hulk | `battle-sim`, story scripts | large, not planned |
| G6 | Subjugator superlaser (`Super_Laser_Ability`) not checked | the biggest ship of the Remake could be too weak or too strong | `battle-sim` abilities | needs its own run |
| G7 | No test for Seismic Cannon and Nuke Launcher; the existing test uses the Tech 99 `Ground_Ion_Cannon` | regression risk | `galaxy-sim/tests/special_weapon.rs` | tiny |
| G8 | The player cannot aim or trigger the ground-to-space weapon; the first shot comes after 15 s (estimate) | small | `battle-view` (not mine) | |

Next: the fixes are done in this order: G3, G1, G4, G7 (G2 needs a decision from the lead because it touches the build menu).
