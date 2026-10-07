---
name: eaw-verify
description: Build, test and visually verify changes in the space-peace Rust/Bevy workspace - per-area CARGO_TARGET_DIRs, which crate test/example proves what, headless offscreen renders to PNG (app, alamo-render, battle-view, eaw-ui) that you then look at, battle-sim benches/duels, headless GC matches, clippy. Use after any code change before claiming it works, when asked to run/screenshot the game, or when a render/UI/battle change needs to be seen.
---

# Verifying work in space-peace

Bevy 0.19.1, edition 2024, Windows. `profile.dev` uses opt-level 1 and deps at 3, release uses thin LTO.
Full builds are slow, so build only the crate you touched (`-p <crate>`).

## Target directories
Each area uses its own `CARGO_TARGET_DIR`, so parallel builds and agents don't block each other on the cargo lock:
`target/app` (space-peace), `target/render`, `target/battle`, `target/ui`, `target/play`, `target/ai`,
`target/lua`, `target/audio`, `target/tmp` (tools). In PowerShell:
`$env:CARGO_TARGET_DIR="$PWD\target\render"; cargo ...`

## Checklist for a change
1. `cargo check -p <crate>`, then `cargo clippy -p <crate> --all-targets` (crates are kept clippy-clean).
2. `cargo test -p <crate>`. Real-data tests **skip** without the game; check the output for skip notes.
3. Visual or behavioural proof using the matching row below. **Open the PNG with the Read tool and look at it**
   before saying a visual change works.
4. Update the crate's `STATUS.md` (and RULES.md / UI.md if rules or findings changed).

## What proves what

| Area | Command (PowerShell, from repo root) |
|---|---|
| App (menus, galaxy map) | `cargo run -p space-peace -- --offscreen out.png [--screen menu\|campaigns\|galaxy\|pause\|options\|loading] [--campaign <name>] [--select <planet>] [--zoom 0..1] [--frames N]` renders 3840x2160 with no window and no audio |
| Models / shader / particles | `cargo run -p alamo-render --example render_scene -- <model\|-> out.png [--team --sun --cam yaw,pitch,dist --fx --times --many N --shadows --hdr]`; `showcase <idle\|xwing\|turret\|hyper\|death> out.png`; `battle out.png`. Offscreen only. Sim time is held ~3 s for shader warm-up |
| Space battle view | `cargo run --release -p battle-view --example battle_demo -- --offscreen out.png --size 3840x2160 --times 5,30,90 --auto [--map ..] [--cam yaw,pitch,dist]`. Add `--features profile` + env `TRACE_CHROME=trace.json` for a Chrome trace |
| Battle rules / balance | `cargo run --release -p battle-sim --example battle_duel -- [filter] [delay=S] [snipe] [range=F]` (env `TRACE`, `WINDOW`); perf: `battle_bench` (40v40 ~0.45 ms/step baseline) |
| Original UI | `cargo run -p eaw-ui --example ui_demo -- --offscreen out.png --faction Rebel\|Empire\|Underworld --mode galactic\|space [--panel ...] [--hover name]` |
| Galactic sim | `cargo run --release -p galaxy-sim --example headless_match -- [weeks] [seed] [campaign] [novictory] [tactical]` |
| Lua / AI | `galaxy-script` examples `scripted_match`, `ai_match`; `alamo-lua` example `run_story` |
| Parsers | `alamo-model` examples `parse_all`, `parse_all_ala`, `parse_all_particles` must stay at 0 errors (4,348 / 7,685 / 1,505 files) |

Put screenshots and traces in the scratchpad or `target/`, never in the repo root.

## Comparing against the original
When the look or behaviour should match EaW, say what you compared it with (data value, screenshot,
known number from RULES.md). Known reference results: ISD vs MC-80 1v1 ~270 s (MC-80 wins),
VSD vs Nebulon-B ~30 s.

## Honesty rules
- Windowed mode and real mouse input are mostly untested (see STATUS.md files). An offscreen PNG does not
  prove that clicking works. Say so instead of claiming it.
- Frame-time numbers depend on the machine and on other running programs. Report them with the conditions.
