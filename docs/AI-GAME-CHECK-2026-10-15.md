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
