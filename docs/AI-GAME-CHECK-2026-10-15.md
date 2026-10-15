# AI game check, 2026-10-15

Headless full games on Remake 4.0 with the Remake AI (`ScriptedSim`, strategy layer on, fog on), checked like a player would feel them.
Tool: `crates/galaxy-script/examples/ai_check.rs` (new): `ai_check <weeks> <campaign> <human faction|none> <easy|normal|hard|brutal> [groundup]`;
env `BATTLES=1` (every battle), `PENDING=1` (sides of every pending battle), `STRUCT=<name> FACTION=<f>` (why a structure cannot be built),
`HEROES=1` (buildable heroes). Existing tools used: `ai_audit` (80 weeks, seed 1) and `ai_match` (plan statistics).

Runs (80 weeks each): Rebel campaign with Rebel as passive human, Normal; all factions AI, Normal; Underworld Ground Up with Underworld as passive human.
Not run: Hard and the Empire campaign (the campaign list has `GC_Full_Web_Way_Imperial`, not `..._Empire`, which my first Empire run tripped over);
the findings below do not depend on difficulty, but Hard was not measured.

## Ranked findings (worst for the player first)

| # | Finding | Evidence | Status |
|---|---|---|---|
| 1 | **The Rebel AI never wins an attack.** It sends fleets (often 3 separate ones in the same week) into Pirate planets that hold a Lv1/Lv2 star base, 8 capital ships and 9 garrison squadrons, and loses everything; the Pirates lose 0 to 2 ships. Same planets again and again (Ord Trasi 5 times, Vjun 4 times). | all-AI 80 weeks: Rebel 0 won / 37 battles, 593 ships lost, still 5 planets (start 5). `ai_audit` buckets: even "overwhelming (>3x ships)" attacks against a star base won 0 of 12; across all factions no attack on a star base planet was won in the Rebel/Underworld runs. Ground Up Underworld game: Rebel 2 won / 40, Empire 5 / 32. | **Partly fixed**: lost-attack memory (below) halves the repeated suicide attacks (51 -> 27 battles) but the wins stay 0. Root cause not found: the AI's defence estimate for Pirate planets (`defense_strength`, fog rule, plain `strength` without the resolver's `base_factor` 1.5 and 0.5 small-craft efficiency) is too low; switching to `tuned_strength` alone did not change the record. Needs a decision on the fog rule / a calibration of the AI strength estimate against the resolver. |
| 2 | **Main AIs hardly expand.** Empire 18 -> 20 planets in 80 weeks, Rebel 5 -> 5, Underworld 8 -> 17 (the only one that grows). | all-AI table below | open (follows from 1: all conquest goals end at the same walls) |
| 3 | **Credits pile up in one budget category.** Empire reached 285k credits (income 9k/week) with 243k earmarked for "infrastructure" that cannot be spent: `Build_Income_Gases` is started 294 times and fails every time (`E_Gas_Refinery/Depot` need a gas-giant planet; none of the 20 Empire planets takes one) but its desire is `500 + ...` by the data. | `ai_audit` "unspent earmarked per category: infrastructure 242677" | **Mitigated**: an account may hold at most 20 weeks of income, the excess goes to the other categories (`TaskForces::rebalance`, estimate). Empire ends with 208k instead of 285k, 22 planets and 8 won attacks instead of 20 / 3. A cap of 10 weeks was tried and failed the strict test `strategic_layer_captures_more_enemy_planets_over_several_seeds` (27 vs 34 captures), so 20 stays. |
| 4 | **Collapse and no rebuilding in Ground Up.** The Empire AI in the Underworld Ground Up game ends with 15 ships (peak 146), pop 48/975, a single unit type (Arc-170 squadron 93%), after 27 lost battles. | `ai_check 80 GC_Full_Web_Way_Underworld Underworld normal groundup` | open (same cause as 1: it feeds ships to Pirate walls; Ground Up has 117 to 135 Pirate planets) |
| 5 | **Minor factions hoard and stand still** (Mandalorians, Hapan, CSA, Sith: 2 to 5 planets, 124k to 212k credits at week 80 with a passive human). | human-passive run: credit pile weeks 28 to 49 | open, probably by design (minor AIs do not expand in the data); not changed |
| 6 | **Pirates grow without limit.** 615 -> 967 ships, pop 5470 -> 14606 against a cap of 2690 (Ground Up: 9600 against 4256), income -85780/week. | all-AI run | open: this is the passive faction's rearm rule (user rule in `ground_up.rs`/RULES.md); it is why 1 happens. Not changed. |
| 7 | **Heroes are never used.** No AI faction owned a hero ship at any time (hero weeks 0 for all). | `HEROES=1`: 10 to 13 hero ships are buildable for Rebel/Empire/Underworld | **Faithful to the data**: the 4.0 AI budget template gives the `Hero` category 0% for every faction and the only `Hero` goals are `Remove_Corruption` and the Death Star; the unit tables are not used for hero purchase. Not changed. |
| 8 | **Idle fleets.** Empire: 139 of 1235 fleet-weeks idle for 20+ weeks (11%), Warlords 16%, Cartels 9%; worst single fleet idle 69 weeks. | all-AI run | open, moderate |
| 9 | **Unit spam is mild.** Top unit type 19% to 34% for the big factions; 56% to 73% for factions with 2 to 4 buildable types (Hapan 2 types, CIS Geonosian cruiser 66%). | `ai_check` composition | not a bug (limited unit lists) |
| 10 | Bankruptcy: none (no faction below 0 credits in any run). | | ok |
| 11 | Tech levels: AI factions reach tech 4 to 5 in 80 weeks (Rebel 4, Empire 5, Underworld 5); a passive human stays at 1. | | ok |

## All-AI table (Normal, 80 weeks, seed 1, before the fixes)

| Faction | planets (start -> end) | battles won / lost | credits end | note |
|---|---|---|---|---|
| Rebel | 5 -> 5 | 0 / 37 | 72k | 593 ships lost |
| Empire | 19 -> 20 | 53 / 19 | 285k | 243k unspendable infrastructure budget |
| Underworld | 10 -> 17 | 12 / 18 | 50k | 21 unit types |
| Pirates (passive) | 68 -> 68 | 90 / 1 | 0 | 967 ships |

## What was changed (not committed)

* `crates/galaxy-ai/src/taskforce.rs`: `TaskForces::rebalance` + test `unspendable_money_moves_to_the_other_categories`.
* `crates/galaxy-ai/src/engine.rs`: calls `rebalance` after funding (`AI_ACCOUNT_CAP_WEEKS` = 20, estimate); `note_lost_attack` / `note_won_attack`:
  offensive goals avoid a planet where the player just lost an attack, 6 weeks per loss in a row, at most 24 (`lost_attack_cooldown`, estimate, only with the
  strategy layer on so the plain baseline of the strict strategy test is unchanged); test `cooldown_grows_with_every_lost_attack_and_is_capped`.
* `crates/galaxy-script/src/lib.rs`: `note_attack_results` feeds `BattleResolved` into the AI engine.
* `crates/galaxy-script/examples/ai_check.rs`: the check tool.

Tests: `galaxy-ai` and `galaxy-script` suites pass, including `ai_match` (the strict strategy-vs-plain capture test, difficulty tiers, determinism).

## Not verified

* Hard / Brutal behaviour, Empire as the human faction, Clone Wars content.
* Whether the real Remake AI loses as often against Pirates; the data (goal/equation XML and Lua) is run unchanged, only the estimates of enemy strength come from our side.

## Follow-up: why the Rebel AI lost every attack (systematic check)

Method: `ai_check` with `ATTACKS=<faction|1>` turns auto-resolve off, prints at every pending battle both sides (ships, plain and resolver-tuned
strength, star base / garrison), the winners of 9 resolver previews with different seeds, then resolves it and prints the result.

* **The resolver is fine.** Outcomes follow the strengths: Underworld 59 ships (tuned 25551) vs Cartels (23914) won with 46 losses; Rebel 68 ships (30105) vs Pirates
  (42686) lost 61 ships and killed 3, which is what the square law gives for a 0.7 ratio. All 9 previews agree with the real result.
* **The AI's belief about the defence is right** (believed 11.7k for Ord Trasi; real 12.6k plain). Three things were wrong on the attacking side:
  1. **Piecemeal arrival.** The plan assembled about 19 ships (strength 12.5k, equal to the defence) but they reached Ord Trasi as three fleets 593 s, 598 s and 600 s
     into the game (2, 12 and 5 ships); the sim opens and resolves a battle at the first arrival, so each fleet fought alone (preview: 9 of 9 Pirate wins each).
     **Open**: needs a short gather window before a battle is resolved (`detect_battles`) plus joining of later arrivals; not done (touches galaxy-sim and galaxy-play).
  2. **Force cap.** `compose` stopped at 24 ships (the plans set no `MaximumTotalSize`, 24 was our default) even when the strength it wanted (54k) was far above
     what 24 ships reach (24k), and then launched anyway. Fixed: default cap 60 (`DEFAULT_MAX_FORCE`), and a force below `MIN_VIABLE_FRACTION` (0.9) of the believed defence
     is not sent (the plan fails with "no ship to build").
  3. **Own strength overestimated.** The AI measured its fleets with the plain estimate, but the resolver halves the damage of fighters and bombers
     (`combat_efficiency` 0.5), and Rebel fleets are mostly squadrons (plain 30105 = tuned 21896). `galaxy-script/src/ai.rs` now uses `tuned_strength` for
     force and defence estimates.
* **Result (ai_audit, 80 weeks, seed 1):** Underworld 17 attacks, 17 won (was 29 / 11); Rebel 0 attacks (was 37 / 0), Empire 4 / 1 (was 21 / 3). The Rebel AI now stays home and
  hoards (credits 376k): it cannot assemble a force of the strength the Pirate planets have by week 30 (60 squadron ships reach ~26k against 70k), so the AI that was
  throwing its fleet away now does nothing. Making it build capital-heavy forces (composition by strength, not by role weights) is the next step and is not done.
* **Pirates (item 2).** Per the 4.0 data the Pirates are fully passive: `GC_PiratesAi` / `Pirates_Template` turn off the Infrastructure, Defensive and Offensive goal categories
  (no budget), `Maintenance_Cost` 0, `Credits_Accumulation_Factor` 1.0, `Space_Tactical_Unit_Cap` 180 (a limit per battle; the statistical resolver already uses it for
  waves). Nothing in the data lets them grow. The growth is our user rule (`rearm_passive`: one more ship per planet and week up to 2 x the main factions' tech, picked from every ship type
  the Pirates field, capital ships included): 68 planets x up to 10 ships = the 967 ships and the pop 14.6k (the faction pop cap does not apply to a faction that never builds).
  It is bounded (per planet, by tech), so "unbounded" was wrong. What makes Pirate planets walls is the mix: a random pick from a pool with Lucrehulk battleships and cruisers
  gives 35 to 65k strength per planet by week 30 to 50. **No change made**; a design decision for the owner of the rule (for example pick by the planet's start garrison types, or cap a planet's passive
  garrison at the faction's tactical cap in pop and at a strength bound).

### Correction to the Pirates paragraph (measured afterwards)

Most of the Pirate fleet is the data's own start spawn, not our rearm rule: `ai_check 0/1/2 ... Rebel normal` shows 58 Pirate ships at week 0, 565 after week 1 and 617 from week 2 on
(tech 1, so `rearm_passive` adds nothing at 2 ships per planet). The jump is `Scripts/Story/Spawn_Starting_Forces_Web_Way.lua`: every Pirate planet gets
`Free_Random(2000, 3000) x planetSpaceCombatPowerModifier` (0.5 for a neutral owner) of combat power from the Pirate unit table, against 500 to 750 x modifier for Rebel, Empire and Underworld planets
(CIS 2000 to 3000 like the Pirates): about 1000 to 1500 power, 8 to 9 ships per planet, which is why the Pirate planets are 4 times as strong as the main factions' own. Only the growth from 617 to 967 ships
(all-AI games, where the main factions reach tech 4 to 5) is the user rule. So the Pirates' strength is the data's; no cap is applied.

## Update after the three follow-up fixes (80 weeks, Normal, all-AI, seed 1)

1. **Piecemeal arrival fixed on the AI side.** Tracing every AI move command showed the cause: the Lua plans ask for a staging planet (`Ground_Staging_Planet`, "enemy undefended") and
   `Produce_Force` took it literally, so the staging planet was the target itself and each ship of the force travelled there as its own fleet. The staging planet must now be an own planet
   (else the nearest own planet to the target), so the force merges there and moves as one fleet; pincers are off (`PINCER_ENABLED`) and only one offensive plan per target runs at a time.
2. **Pirate rearm from the planet's own types** (RULES.md, "Passive faction rearm"): weekly ship from the types the planet already fields, else the 3 cheapest.
3. **Strength-driven top-up** in `compose`: when the role-weighted fill is short of the strength the target needs, free slots are filled and, at the force size cap, the least
   cost-effective ships are swapped for the ship type with the most strength per credit (`force_strength / cost`) until the force is strong enough. (Population cap is not modelled in the
   selection; the force cap of 60 stands in for it.)

| | before (first check) | after |
|---|---|---|
| Rebel planets / attacks won | 5 / 0 of 37 | 11 / 12 of 18 |
| Empire planets / attacks won | 20 / 3 of 21 | 23 / 8 of 8 |
| Underworld planets / attacks won | 17 / 11 of 29 | 22 / 17 of 31 |
| Rebel credits at week 80 | 72k (376k after the first fixes) | 32k (spent 98% of earned) |
| Pirate ships (week 80) | 967 | about 820 |

Still open: Empire fleets idle for 20+ weeks (409 of 1169 fleet-weeks, worst 55 weeks); heroes are not bought by the 4.0 AI (data); minor factions still hoard when the human is passive.

## Items 3b/4 and the Clone Wars stack (80 weeks, Normal, all-AI, seed 1, ai_check)

What changed (all in `crates/galaxy-ai/src/taskforce.rs` unless noted):

- **Faction doctrine from the roster** (`Doctrine`): role shares, pick weight and the price exponent of the top-up come from the faction's own buildable ship costs (median and top quarter, measured against the other full rosters of the game). Empire comes out heavy (few big ships, ranks by raw strength), Rebels light (many cheap ships, ranks by strength per credit), Underworld in between. Standing fleets still follow the percentages of the data's `Build_Units_*` lists, so the class mix below is mostly the data's; the doctrine shapes attack forces and the top-up.
- **Clone Wars conquer plans** (`alamo-lua/src/runtime.rs`): the submod's `Conquer_Enemy_Planet.lua` has a stray comma and a missing comma and did not compile, so no Clone Wars AI could ever attack (0 attacks in 80 weeks, 1 million credits banked). The loader now repairs those two slips (RULES-level workaround, labelled in the code).
- **Elastic ship lists** (`A | B = 0, 10`): the submod lists ships as min/max lines. The old code took the maximum (up to 8) of every line, hundreds of ships the queue then refused. Now the minimum is certain, the maximum a cap, and the force is filled to the strength the target needs (estimate).
- **Population** (`AiSim::population_room`): a force never asks for more ships than the faction's population cap leaves room for. Ships already built do not count.
- **Force sizing floor**: the strength a force aims for was lower than the strength below which a force is dropped as too weak (difficulty factor under 1), so many attacks were assembled and then discarded. Both now agree.
- **Front defence** (`update_garrisons`, estimate): the Remake data has no active galactic defence (`Generate_Magic_Space_Defense` exists but no goal function selects it, the Defensive budget is 0%). It keeps the data's idea of a front (`Function_Is_Connected_To_Enemy`): a planet next to an active enemy's planet, or with enemy fleets one or two jumps away, keeps fleets worth 0.8 times that threat (at least 2% of the owner's fleets on a front planet, at most 15% on one planet and 30% overall) out of the pool attack forces draw from; free fleets move to a front planet that misses its garrison; when a front planet falls the front is recomputed, so the line re-forms one jump back. Pirates count as no threat. Fleets reserved for an attack stay free.
- **Second battle at the same planet** (`galaxy-sim/src/tactical.rs`): not part of the AI work; see the bug fix: reserve units never called in no longer survive for the loser.

Per faction (planets week 0 / 40 / 80, attacks won / lost, credits banked, fleet-weeks idle 20+ weeks of all fleet-weeks, front planet-weeks without a fleet of all front planet-weeks; "lost in a row" = most planets lost within 8 weeks, 0-1 everywhere):

### Campaign GC_Full_Web_Way_Rebel
```
faction | planets wk0 / mid / end | attacks won / lost | credits | idle fleet-weeks (20+) of fleet-weeks | max planets lost within 8 weeks | front planet-weeks bare / all |
Rebel | 3 / 7 / 11 | 7 / 2 | 17k | 0 of 954 | 0 | 56 / 192 |
Empire | 18 / 19 / 26 | 10 / 1 | 80k | 77 of 1278 | 0 | 534 / 1033 |
Underworld | 8 / 13 / 17 | 8 / 3 | 52k | 51 of 1023 | 0 | 233 / 685 |
CIS | 3 / 3 / 3 | 0 / 0 | 28k | 44 of 352 | 0 | 0 / 53 |
Republic | 9 / 9 / 9 | 2 / 0 | 37k | 87 of 677 | 1 | 93 / 524 |
MIX: faction | ships | capital % | frigate % | corvette % | fighter+bomber % | other % | mean price | share of price in capitals %
MIX: Rebel | 394 | 0 | 5 | 10 | 85 | 0 | 695 | 0 |
MIX: Empire | 554 | 0 | 8 | 39 | 53 | 0 | 1252 | 3 |
MIX: Underworld | 505 | 0 | 13 | 19 | 68 | 0 | 1201 | 10 |
MIX: CIS | 39 | 3 | 72 | 26 | 0 | 0 | 5628 | 36 |
MIX: Republic | 146 | 5 | 27 | 20 | 48 | 0 | 3153 | 37 |
```
### Campaign GC_Full_Web_Way_Imperial
```
faction | planets wk0 / mid / end | attacks won / lost | credits | idle fleet-weeks (20+) of fleet-weeks | max planets lost within 8 weeks | front planet-weeks bare / all |
Rebel | 5 / 9 / 15 | 12 / 3 | 39k | 19 of 1023 | 0 | 42 / 251 |
Empire | 18 / 18 / 19 | 4 / 2 | 76k | 67 of 719 | 1 | 699 / 1095 |
Underworld | 8 / 12 / 18 | 10 / 2 | 53k | 48 of 1052 | 0 | 230 / 625 |
CIS | 3 / 3 / 3 | 0 / 0 | 27k | 93 of 352 | 0 | 0 / 64 |
Republic | 9 / 10 / 10 | 0 / 0 | 64k | 81 of 617 | 0 | 84 / 549 |
MIX: faction | ships | capital % | frigate % | corvette % | fighter+bomber % | other % | mean price | share of price in capitals %
MIX: Rebel | 509 | 0 | 3 | 17 | 79 | 0 | 686 | 0 |
MIX: Empire | 291 | 0 | 13 | 27 | 59 | 0 | 1517 | 3 |
MIX: Underworld | 460 | 0 | 15 | 18 | 67 | 0 | 1223 | 4 |
MIX: CIS | 40 | 2 | 72 | 25 | 0 | 0 | 5588 | 36 |
MIX: Republic | 167 | 6 | 28 | 18 | 48 | 0 | 3240 | 38 |
```
### Campaign GC_Full_Web_Way_Underworld
```
faction | planets wk0 / mid / end | attacks won / lost | credits | idle fleet-weeks (20+) of fleet-weeks | max planets lost within 8 weeks | front planet-weeks bare / all |
Rebel | 5 / 10 / 14 | 7 / 1 | 52k | 12 of 936 | 0 | 5 / 280 |
Empire | 18 / 19 / 23 | 4 / 1 | 74k | 93 of 1094 | 0 | 630 / 1081 |
Underworld | 7 / 10 / 13 | 5 / 6 | 154k | 38 of 973 | 0 | 267 / 633 |
CIS | 3 / 3 / 3 | 0 / 0 | 24k | 44 of 366 | 0 | 0 / 52 |
Republic | 9 / 10 / 10 | 1 / 0 | 39k | 139 of 644 | 1 | 76 / 546 |
MIX: faction | ships | capital % | frigate % | corvette % | fighter+bomber % | other % | mean price | share of price in capitals %
MIX: Rebel | 495 | 0 | 6 | 13 | 81 | 0 | 825 | 0 |
MIX: Empire | 586 | 0 | 6 | 43 | 50 | 0 | 1159 | 3 |
MIX: Underworld | 403 | 0 | 18 | 18 | 64 | 0 | 1365 | 5 |
MIX: CIS | 41 | 2 | 73 | 24 | 0 | 0 | 5549 | 35 |
MIX: Republic | 171 | 6 | 28 | 18 | 49 | 0 | 3125 | 38 |
```
### Campaign GC_Full_Web_Way_Republic
```
faction | planets wk0 / mid / end | attacks won / lost | credits | idle fleet-weeks (20+) of fleet-weeks | max planets lost within 8 weeks | front planet-weeks bare / all |
Rebel | 6 / 13 / 17 | 8 / 2 | 333k | 39 of 1494 | 0 | 86 / 431 |
Empire | 18 / 21 / 26 | 11 / 2 | 230k | 149 of 1215 | 0 | 697 / 1113 |
Underworld | 7 / 9 / 12 | 3 / 0 | 28k | 60 of 902 | 0 | 289 / 682 |
CIS | 3 / 10 / 14 | 15 / 3 | 263k | 32 of 1003 | 0 | 16 / 249 |
Republic | 8 / 14 / 20 | 11 / 15 | 81k | 156 of 1670 | 0 | 143 / 800 |
MIX: faction | ships | capital % | frigate % | corvette % | fighter+bomber % | other % | mean price | share of price in capitals %
MIX: Rebel | 472 | 1 | 11 | 32 | 45 | 11 | 1691 | 24 |
MIX: Empire | 350 | 1 | 36 | 10 | 41 | 12 | 3714 | 11 |
MIX: Underworld | 386 | 0 | 5 | 27 | 57 | 11 | 1139 | 0 |
MIX: CIS | 190 | 3 | 9 | 65 | 22 | 0 | 2943 | 17 |
MIX: Republic | 588 | 2 | 11 | 10 | 68 | 10 | 1665 | 27 |
```
### Campaign GC_Full_Web_Way_CIS
```
faction | planets wk0 / mid / end | attacks won / lost | credits | idle fleet-weeks (20+) of fleet-weeks | max planets lost within 8 weeks | front planet-weeks bare / all |
Rebel | 6 / 10 / 21 | 16 / 5 | 135k | 25 of 1582 | 0 | 62 / 444 |
Empire | 18 / 21 / 28 | 14 / 1 | 241k | 117 of 1357 | 0 | 723 / 1209 |
Underworld | 7 / 9 / 12 | 4 / 0 | 113k | 56 of 742 | 0 | 283 / 642 |
CIS | 3 / 8 / 11 | 15 / 6 | 264k | 69 of 973 | 0 | 21 / 242 |
Republic | 8 / 13 / 14 | 7 / 1 | 258k | 127 of 1086 | 1 | 96 / 665 |
MIX: faction | ships | capital % | frigate % | corvette % | fighter+bomber % | other % | mean price | share of price in capitals %
MIX: Rebel | 627 | 1 | 14 | 28 | 44 | 12 | 1529 | 10 |
MIX: Empire | 343 | 1 | 20 | 21 | 41 | 18 | 2808 | 12 |
MIX: Underworld | 423 | 0 | 2 | 37 | 55 | 7 | 1013 | 0 |
MIX: CIS | 193 | 6 | 9 | 68 | 17 | 0 | 3591 | 33 |
MIX: Republic | 393 | 2 | 10 | 21 | 47 | 19 | 2106 | 31 |
```

Reading the tables:

- Before the work: Rebel 0/37 attacks (first pass), then 12/18; Clone Wars Republic/CIS/Rebel/Empire 0 attacks. Now every main faction of both stacks attacks and wins most of them; Clone Wars Republic 8 to 14-20 planets and CIS 3 to 11-14 planets in their own campaign (seed 1, one seed).
- Planets lost in a row stay 0-1: the AIs do not collapse through the core. Front planets without any fleet are common for Empire (about 60% of front planet-weeks): its large star bases carry the defence value (the garrison target subtracts half of the fixed defence), so most of its fronts need no fleet.
- Class mix: the standing fleets are shaped by the data's build lists, so every Remake main faction is 50-85% squadrons and owns no capital ship in 80 weeks (population cap 97% full from about week 40, capital ships cost 75-100k). Rebel is the most squadron-heavy (79-85%), Empire has the most corvettes (27-43%) and the highest mean price. The Clone Wars factions own capitals (CIS 2-6%, Republic 2-6%) and more frigates. A heavier Empire needs the build lists weighed by doctrine and a scrap rule for a full population cap; neither is in the data, not done.
- Not verified: Hard/Brutal, Empire as the human, more than one seed per campaign.

## Capital ships and saving for expensive items (80 weeks, Normal, all-AI, seed 1)

Cause of "Empire never owns a capital ship": the Remake's standing-fleet plan (`GC_Build_Units_Space`) only runs when the player is "economically rich" (data), and the conquer plan lists every ship class as one 100% group, so composition is the engine's call, not the data's. Every plan of a goal category drew on one shared account, and cheap items spent each credit the moment it arrived, so a 17-25k shipyard or an 18-50k capital ship was never paid: Empire had 1 capital shipyard at week 40 (data wants a third of its planets) and its fleet of 554 ships held no capital.

Fix (`taskforce.rs`, labelled estimates): a plan that cannot pay for a structure makes its category save for it (`wait_for_money`; other plans of the category leave that price in the account, the claim lapses 60 s after the plan stops waiting, a price above the account cap is never saved for). The same applies to costly ships, scaled by the faction's doctrine: a fully heavy roster saves for a ship up to 0.4 of the account cap, a swarm roster (Rebels) never, so Rebels keep spending as money arrives. Structure plans that only find the structure already standing now fail (so the goal's failure learning moves to another planet).

Class mix of owned ships at week 80 (capital / frigate / corvette / fighter+bomber %, mean price, share of fleet price in capitals):

| Campaign | Faction | capital | frigate | corvette | squadrons | mean price | price in capitals |
|---|---|---|---|---|---|---|---|
| Rebel | Rebel | 0 | 3 | 5 | 91 | 563 | 0% |
| Rebel | Empire | 3 | 29 | 40 | 29 | 3027 | 18% |
| Rebel | Underworld | 0 | 15 | 22 | 63 | 1116 | 0% |
| Republic (CW) | CIS | 15 | 29 | 49 | 6 | 7058 | 52% |
| Republic (CW) | Republic | 1 | 2 | 15 | 67 | 1203 | 32% |
| Republic (CW) | Empire | 5 | 25 | 7 | 55 | 5340 | 34% |

Rebels are a squadron swarm at a mean price of 563 credits, Empire fields 5 times the price per ship with capitals; CIS is the heaviest of the Clone Wars stack. Cost: planets at week 80 over Rebel+Empire+Underworld went from about 54 to about 49 in the Rebel campaign (one seed, the runs diverge chaotically, treat as noise-level to -10%). Empire's capital count is still low (3% of ships): the standing-fleet plan is data-gated and there is no scrapping in EaW, so a full population cap is refilled only as losses free room. Not measured: Hard/Brutal, Empire as the human.
