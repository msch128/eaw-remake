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
