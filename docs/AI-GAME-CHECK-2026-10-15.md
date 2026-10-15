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
