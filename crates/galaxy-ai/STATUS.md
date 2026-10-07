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
