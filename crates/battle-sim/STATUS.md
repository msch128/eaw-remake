# battle-sim status

Deterministic, rayon-parallel space-battle simulation (fixed dt 1/30). Rules and balance notes: `RULES.md`.

## What works
- `Catalog::build` from alamo-data + ALO models (hardpoints from bones, turret arcs, Model_To_Attach barrels).
- `Battle::{new, step, command, result, state_hash}`; arenas from .ted maps (`Arena`); abilities, tractor beam, missile shield,
  interdiction, ion stun, hangars/reinforcements, retreat, unit cap.
- Bench `battle_bench` (40v40 about 0.45 ms/step) and `battle_duel` (diagnostics: `delay=`, `snipe`, `range=`, env `TRACE`, `WINDOW`).
- Tests: `tests/{mechanics,real_data,features}.rs`; real-data tests skip without the game.

## Key decisions
- Speeds: XML `Max_Speed` (ships and projectiles) is per frame, scaled x30 (`Catalog::speed_scale`); all `*_Seconds` fields are real seconds.
- Shields: `Shield_Refresh_Rate` per 2 s interval, applied continuously (`shield_regen_delay` = 0); a shield drained to zero is offline for
  `Depleted_Shield_Disable_Time` (4 s). Attack-move priority sets are off by default (`attack_move_sets`).
- Results: 1v1 ISD vs MC-80 about 270 s (MC-80 wins), VSD vs Nebulon-B about 30 s, X-wing vs TIE squadrons about 2 s after contact.

## Gaps / next steps
- Fighter dogfights are very short: no evasion or aim error modelled; torpedo burn effect missing.
- Ships hold one standoff range and bring only one broadside to bear; no orbiting or facing optimisation.
- Energy and `Depleted_Shield_Regen_Cap` are not modelled; acceleration times by size class are estimates.
- The 30 Hz speed scale is an assumption; check against in-game ship speeds.

## Difficulty and carried damage (new)
`BattleConfig::side_mults` (damage dealt, hull and shield multipliers per side) and `BattleConfig::start_health` (ships start with a share of hull/shield);
`GroupResult::health` reports the survivors' hull/shield fractions of single ships (not squadrons) so galaxy-sim can keep the damage.

## Tactical AI review (see RULES.md section 9)
- New: data-based AI retreat (`Retreat_From_Space` thresholds, `BattleConfig::retreat_allowed`), focus on weak ships, generic ability use (speed / shield regen / damage reduction), separation of big ships.
- Tests `tests/tactical_ai.rs` (real data): outgunned AI retreats (and does not when forbidden), even fights are fought out, no ship stacking, real-map duels are decided and deterministic (threads included),
  defenders with a station do not flee. The ability rules have no dedicated test.
- Known gaps: strength estimate for the retreat rule is ours; retreat orders at age 120 s often come too late to save slow ships; AI never calls reinforcements; no flank/escort goals.
