# galaxy-ai status

The Remake's galactic AI on top of an abstract `AiSim` (implemented for galaxy-sim in `galaxy-script`). In progress.

## What works
- `AiData` loads `Data/Xml/AI` (player types, goal function sets, goals, perceptual equations, templates);
  `campaign_ai_players` picks the AI factions of a campaign.
- `equation.rs`: parser/evaluator for the perceptual-equation language; `all_remake_equations_parse` covers every Remake equation.
- `perception.rs`: equation variables computed from an `AiSim` snapshot (reconstructed definitions).
- `AiEngine`: budgets per goal category, goal/target scoring, plan selection; `discover_plans` finds plan scripts.
- `taskforce.rs`: space-relevant task force methods (`Produce_Force`, `Move_To`, `Invade`, `Release_Forces`);
  `requirements.rs` parses requirement strings. Example `ai_vocab`.

## Key decisions
- Variables and operator precedence are inferred from usage (engine definitions are unavailable); unknown variables read
  as 0 and are counted in `EvalStats`.
- Ground forces are "assembled" instantly; only space-relevant behaviour is real.

## Gaps / next steps
- `AI.md` is referenced by the sources but not written yet.
- `NBT*` variables are approximated by their plain versions; unmodelled variables need review against AI behaviour.
- Needs the galaxy-sim API additions listed in `galaxy-script/NEEDS.md`, then replacing the stub AI in the live game.
- No behaviour comparison against the original AI yet.

## Update (AI-vs-AI test)
- Force composition only picks ships some owned planet can build right now (`can_build_unit` Ok or NoCredits); before, plans asked for
  high-tier ships nobody could build and every conquer plan died with "Space Production Failed".
- `TaskForceStats::failure_reasons` (per unit and reason) and `AiStats::{tick_nanos, snapshot_nanos}` for diagnostics (`ai_match` prints them).
- With `ScriptedConfig::remake_ai = true` (galaxy-script) all AI factions build, expand to neutral/pirate planets via
  `GC_Conquer_To_Unite_Territory_Ground`, and capture planets; direct `GC_Conquer_Enemy_Planet` attacks are rare (the plans need ground `Regiment`s
  as well, which are "assembled" instantly, and the AI avoids fights it does not expect to win). The stub AI stays the default until the
  behaviour is compared against it over longer runs.
- Perf: ~240 ms AI per game week for 12 factions, mostly plan script start-up (new Lua VM per plan); a VM/chunk cache in alamo-lua would cut it.

## Update 2 (money budgets, attacks, perf)
- **Why enemy attacks were rare:** `Need_To_Conquer_Enemy_Planet/Space` multiply by `Is_Player_Economically_Wealthy` (credits > 25000 + 1750 x planets) and the
  engine did not reserve money per category, so build plans spent everything. `TaskForces::fund/set_plan_category` now keep a per-category account:
  the treasury at the first evaluation plus the net income since the last one is split by the template's budget fractions, plans only spend what their
  category holds, and unused earmarks stay in the treasury (estimate; the engine's bookkeeping is not in the data). Factions that save up now start
  `GC_Conquer_Enemy_Planet`; the ground part needed no change (ground forces are already instant).
- Plans run to the attack (forces ready -> `Move_To` -> orbit check); many attacks are lost against star bases/fleets ("SpaceForce lost battle"), which is
  the plan's own failure path. 20 weeks all-AI: 1-2 captures from enemy owners, ~9 captures in total (the stub AI takes ~500 in 30 weeks, which is
  unrealistically fast; do not use it as the target).
- Dead-code warnings removed.

## Strategic layer and difficulty (see STRATEGY.md)
- `strategy.rs`: influence maps, utility scoring (flank, pocket, expansion, consolidation, defend), fog of war with memory, personalities, tiers.
  `AiEngine::strategy` (enabled by default; `ScriptedConfig::strategy`). Force sizing by predicted square-law strength, pincer rally, free-store reuse
  and reservation. Tests: `tests/strategy.rs` (flank scenario, risk/distance/expansion, honest information, consolidation, personalities, cost) and
  `galaxy-script/tests/ai_match.rs` (tiers over 30 weeks, strategy vs plain over 4 seeds: 36 vs 19 enemy-owner captures, no faction gains more than +5
  planets in 10 weeks; same-seed determinism).
- Cost: strategy layer ~1 ms per game week; whole AI ~75-90 ms per game week.
- **Difficulty setting needs (for the app, space-peace):** pass `ScriptedConfig { difficulty: Some(DifficultyTier::{Easy,Normal,Hard,Brutal,Fair}), remake_ai: true, .. }`;
  show Brutal/Fair as "custom" (not part of the mod). The player's own faction gets no multipliers. For tactical battles the multipliers arrive in
  `TacticalRequest::sides[i].{damage,health,shield}_mult` and `request.battle_config(arena)` already puts them into `BattleConfig::side_mults`; a front end
  that builds its own `BattleConfig` must copy `side_mults` from the request. The galaxy-sim credit multiplier applies to income of non-human factions only
  once `Sim::set_faction_difficulty` is called for them (done by `ScriptedSim` when a tier is configured; the built-in stub AI does not use tiers).
- Known limits: the pincer only triggers when the single-force requirement lies between 50% and 90% of the player's strength; fog applies to the
  strategy layer and force sizing, not to the original equations; weights are estimates.

## App-side needs (save/load, defaults)
- Remake AI + strategy + Normal difficulty are the default of `galaxy_script::ScriptedConfig`; the menu should offer `DifficultyTier` (Easy/Normal/Hard from the mod, Brutal/Fair marked custom)
  and an option "classic AI" (`remake_ai: false, difficulty: None`).
- Save/load and the journaled input methods: see `galaxy-script/STATUS.md` (route every command through `ScriptedSim`, never through `with_sim_mut`).
- Persistent damage: the tactical front end passes `TacticalResult::health` through unchanged; the HUD can read `Galaxy::damage` / `PlanetState::base_health` (fractions) for fleet panels.
- Soak findings fixed: factions never advanced their tech level (the Remake AI has no galactic tech plan we run; now `AiEngine` queues the next level every 4 weeks for a faction with > 20000 credits, level L from week 10 x L on: an **estimate**; `AiStats::tech_upgrades`), `plan_category` and `reserved` entries are pruned.
  Open: the AI floats millions of credits late in the game (build plans refused by every planet / limited throughput), see galaxy-play/STATUS.md.

## Tech and hoarding: root causes (soak follow-up)
- **Tech:** the Remake has no galactic AI research plan. `Research/Story/Tech_Levels_*.xml` make the AI factions follow the tech level a player reaches: `STORY_TECH_LEVEL` events (`Rebel_Tech_2` ...) and `*_Auto` `STORY_TRIGGER` events with `SET_TECH_LEVEL`
  rewards for the other factions. Our story engine never fired the `*_Auto` events (it treated STORY_TRIGGER as "only when triggered by name"); fixed in `alamo-lua/story.rs` (`*_Auto` events with prerequisites fire when activated; inferred from the naming).
  With a human player the AIs now follow the human's tech (T1 -> T4 in the soak). The `AiEngine` tech heuristic (estimate) now runs only in games without any human.
- **Hoarding:** not a missing API. The AI spends 60-70% of its income (Fair tier: 250-560k credits at week 100 instead of 1-2M; the Normal/Hard credit multipliers of the data double/triple it). Spending is gated by the data's own desires: `Need_To_Build_Units_Space_GC` is
  `LowImportance (2) x Is_Player_Economically_Rich x shipyard`, loses to conquest goals for plan slots, and one plan builds 1-3 ships. Changes made: ships are queued at up to 40 planets (was 6, so units buildable only farther away were refused),
  `PLAN_LIMIT_FACTOR` named. Tried and rejected: shortening plan lifetime (no effect). Leave the rest to the data.

## Strategy vs plain AI parity (tuned)
Task force sizing margin `FORCE_MARGIN` (taskforce.rs) 1.25 -> 0.8 (estimate). Over 16 seeds of the all-AI match the strategic layer took 99 enemy-owner captures with 1.25 (plain AI: 150) and 141 with 0.8 (94%), at the same fleet size as the plain AI. The influence-map weights (risk 0.7, consolidation 0.7) were
insensitive (0 to 0.7 gave 49 to 51 over seeds 1-8). The old margin made the layer hoard ships (about 1030 vs 909 ships at week 30) instead of attacking. `ai_match` now compares over 8 seeds with the 80% bar.

## Plan failure "force had no fleet at launch" fixed; measured expansion (night audit)
- **Cause (traced with `AI_TRACE_FORCE=1`, per-plan notes in `TaskForces::produce_note`):** the ready check (`Are_All_Units_On_Free_Store`), `compose` and the claim loop counted free-store ships anywhere on the map, but `tick_units` only claims ships at planets that
  reach the stage without crossing an enemy star base (`safe_from`). Ships parked behind a blockade (e.g. two `Dreadnaught_Cruiser_E` stuck at one planet) made the check say READY, the claim skipped them, the queue refused the unit ("NotAvailable") and
  `Produce_Force` finished with no fleet while the Lua plan went on to `Move_To`. Now `free_ships`/`free_store_covers`/`compose` only count free ships at planets safe from the stage (`stage_safety`). 17-48 such failures per 30-week run -> 0-2.
  8 seeds x 30 weeks (ai_audit): attacks on enemy planets won 130 of 241 launched -> 123 of 156 (52% -> 79%); strategy captures over 8 seeds in `ai_match`: 203 -> 213 (enemy 68 -> 39, pirate/neutral 135 -> 174), i.e. more expansion, fewer wasted fights.
- **Measured expansion drive (`strategy.rs`, `LEADER_BRAKE` 0.6, estimate):** after the fix the Empire snowballed (18 -> 33 planets in 30 weeks on 2 of 8 seeds). `Strategy::share` is a faction's planets against the average of the established factions (4+ planets, no pirates);
  on neutral and pirate targets the utility gets `LEADER_BRAKE x expansion x clamp(1 - share, -1, 0.7)` (leader slows down, small factions catch up). 8 seeds, 30 weeks: largest faction 20-25 planets (start 18; unbraked 24-33), enemy captures strategy 46 vs plain 49 (94%), pirate/neutral captures 131 (total 177 vs 203 before the night).
- `ai_audit` prints `AUDIT` lines (planets of every faction at weeks 0/10/20/30, attacks launched/won, escape moves); `ai_match` test prints pirate/neutral captures and the largest faction, `AI_SEEDS=16` runs more seeds.
