# Nightly runbook (coordinator)

How the coordinator runs unattended. The work list is `docs/ROADMAP-2026-10-09.md`. Findings and status go to
`docs/QA-2026-10-08.md`.

## Loop
1. **Read usage:** `(orca account list --json | ConvertFrom-Json).result.rateLimits.claude` gives
   `session.usedPercent`, `session.resetDescription` and `weekly.usedPercent`.
   - The value can freeze for hours. If it hasn't moved across two checks while agents work, treat it as stale
     and assume high usage.
   - **Session ≥ 85%:** start nothing new, let running agents finish, and wake again after `resetDescription`.
   - **Weekly ≥ 90%:** stop for the night and write the status into HANDOFF.md.
2. **Agents:** at most 3 at a time. Spawn them with the Agent tool on model `sonnet`, each with one roadmap item,
   using the brief template below.
3. **Commits:**
   - Commit only the files the agent names in its report.
   - Gate every commit on `cargo check --workspace --all-targets` (`CARGO_TARGET_DIR=target/check`), then push.
4. **Gate:** after the P0 items are in, run the release gate:
   - build in a fresh worktree with `CARGO_TARGET_DIR=target/gate`,
   - run `tools/smoke.ps1 -Exe <gate exe>`,
   - run `tools/soak.ps1`.
   Only then run `build.ps1` so that `target/release` (the user's exe) is updated.
5. **Morning:** write a short German summary for the user covering what's new, what was verified and how, what
   is still open, and the honest % estimate.

## Agent brief template
> You work on "Space Peace: Not at War", a Rust/Bevy 0.19.1 re-engine of Star Wars: Empire at War (Remake 4.0
> mod). Repo: C:\Users\schro\Documents\space-peace. Read CLAUDE.md and the STATUS.md of the crates you touch.
> Rules:
> - Never open visible windows and never take desktop screenshots. `--hidden` and `--offscreen` runs are fine.
> - Game and workshop folders are read-only.
> - Code and docs in English.
> - Do NOT commit.
> - Use your own `CARGO_TARGET_DIR=target/<area>`.
> - Gameplay numbers come from the data; name any estimate in RULES.md/STATUS.md.
> - References: `C:\Users\schro\Documents\space-peace-ref\` (screenshots `0N_*.png`; Remake video frames
>   `video\vNNNNs.jpg` every 10 s and `video\battle1s\bNNNNs.jpg` every 1 s for the space battle, 613–965 s).
>
> Task: roadmap item #N in docs/ROADMAP-2026-10-09.md (quote it).
> Prove it with a check that fails before the fix (unit test, smoke scenario or input_test). Run
> `cargo check --workspace --all-targets`.
> Final report, short: root cause, fix, verification (test names and results) and the exact list of changed files.

## Area ownership (to avoid two agents in one file)
| Area | Files | Target dir |
|---|---|---|
| tactical | `battle-view/*`, `space-peace/src/battle_play.rs` | `target/tactical` |
| sim | `battle-sim/*`, `galaxy-sim/*`, `galaxy-ai/*`, `galaxy-script/*`, `galaxy-play/src/driver.rs` | `target/battle` |
| galaxy UI | `eaw-ui/*`, `space-peace/src/{play_ui,galaxy_map,menu,input_test}.rs`, `galaxy-play/src/visuals.rs` | `target/ui` |
| render | `alamo-render/*`, `battle-view/src/scene.rs`, `space-peace/src/planet_view.rs` | `target/render` |

## Balance rule (user, 2026-10-09)
Every area at about 80% is worth more than a few areas at 100% and the rest at 50%. The goal is EaW plus the
Remake 4.0 in one, and it must FEEL like it: UI, feel, music and sound included. Always give one of the 3 agent
slots to the currently lowest area. No area may stay below 60% while another goes past 85%.

| Area | Evening 2026-10-08 | Morning 2026-10-09 | Next steps |
|---|---|---|---|
| Sound / music | 45 | 55 | Live listening pass with the user; then fix levels/spam/transitions |
| Battle look | 65 | 70 | Late-stage death clouds, bolt glow, Coruscant fill colour (user) |
| Battle controls | 65 | 72 | Real-mouse QA of abilities/reinforcements/retreat (driver window lookup needs user OK) |
| Battle depth | 65 | 72 | Balance check by playing; ability icons per section |
| Galaxy look | 75 | 78 | Remaining video diffs (banner frame) |
| Galaxy controls | 75 | 78 | Real-mouse QA of the new fleet picking / planet view |
| Stability / perf | 85 | 87 | Keep the gate green |
| Simulation / AI | 88 | 88 | Nothing urgent |

## Night plan (order)
1. **Verify #1:** run the Esc menu smoke scenario (`windowed-battle-esc-menu`); left buttons #5.
2. **Sim:** #3 retreat per user rules, then #4 escape route, then #2 auto-resolve result data.
3. **Galaxy UI:** #2 result dialog for auto-resolve, then #6 duplicate build entries plus the class filter.
4. **Tactical:** #9 Botajef HUD, then #11 attack lines, then #8 formations (with sim).
5. **Gate and user exe:** gate, then `build.ps1`, then the morning summary.
