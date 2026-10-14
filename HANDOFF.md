# Handoff - state as of 2026-10-14

Read `CLAUDE.md` (rules) first. Every crate has a `STATUS.md` with details and next steps. Work list: `docs/QA-2026-10-09-user.md`.
Live test guide: `PLAYTEST.md`.

## Honest status
An audit on 2026-10-14 scored the project at roughly **70 / 100** against the goal "play Remake 4.0 Galactic Conquest with space
battles, EaW look and feel". The earlier "U100 / 100% fidelity" claim was a documentation statement only and is withdrawn.

| Area | Score (audit) |
|---|---|
| Galaxy UI | 74 |
| Galaxy rules / AI | 66 |
| Battle UI | 68 |
| Battle behaviour | 70 |
| Visuals | 72 |
| Audio | 45 (implemented, never heard live) |
| Stability | 78 |

## Done since the audit
- F2/F3: default mod is always Remake 4.0; scripted battle abort waits for scripted clicks.
- Pirate raids removed (Pirates are passive, garrison capped at 2 x player tech per planet; verified ~615 units on 68 planets).
- Galaxy look G1-G11: pale-blue lanes and grid, small owner-coloured labels, thin relation-tinted selection ring, hero portraits,
  pause tab, advisor text next to R2, hover card, battle banner with enemy star base disc, droid menu (log, history line charts,
  tech tree, summary), corrected military history.
- Three fleet slots per planet (S1-S5) in the sim, planet view and AI.
- Battle UI B1-B8: opening camera, unit-card bar and minimap, hero click, camera hotkeys (C, Page Up, Home, End; Space = Movie Mode
  like the original readme), objectives box, death bloom and wrecks. P pauses.

## Verified vs not verified
- Verified: offscreen renders of each item above, `cargo check --workspace --all-targets`, headless sim runs and unit tests.
- Not verified: any sound in a live session (nothing heard since the audio rewrite); real mouse/keyboard input this round (no
  `SP_UI_TEST` / `SP_INPUT_TEST` / windowed smoke runs); B8 bloom flash and wrecks were not seen in a capture; 4K 60 FPS and
  memory numbers predate the recent work.

## Open items
- Invented numbers (X1/X2) are being replaced by data values; remaining estimates are named in each crate's RULES.md.
- Superweapon gauge: no asset source found for the original gauge art.
- Missile arcs need sim work (trajectories are not simulated).
- U37 ground-to-space special weapons (Hypervelocity Gun, ion cannon) do not fire in space battles.
- U38 reinforcement placement: fog readability, ghost facing and +-15 degree wheel rotation.
- U39 laser bolt look (short bright bolts) vs the Remake.
- U41 abilities of every unit visible above a group card.
- U42 space battle Esc menu equal to the galaxy menu, saving during battle.
- U43 three autosave slots (pre-battle, post-battle, rolling 5 minutes).
- Also open from the QA list: U25 rest, U31, U33, U47 reading; see
  `docs/QA-2026-10-09-user.md` for the full text.

## Regression guards
- `tools/smoke.ps1`: offscreen / hidden scenarios with a pass/fail table (`windowed-*` open a real window: only when the user is away).
- `tools/soak.ps1`: hidden battle and galaxy soak with memory, asset-count, log-spam and frame-time checks.
- Input tests: `SP_UI_TEST=1` (galaxy screens) and `SP_INPUT_TEST` (tactical screen) inject window events.

## Crates
`alamo-fs`, `alamo-data`, `alamo-model`, `alamo-map`, `alamo-audio`, `alamo-lua` (parsers and runtime); `galaxy-sim`, `battle-sim`
(headless deterministic sims); `galaxy-ai`, `galaxy-script`; `alamo-render`, `battle-view`, `eaw-ui`, `galaxy-play` (Bevy plugins);
`space-peace` (the app). Per-crate state is in each `STATUS.md`.

## Key decisions (don't silently revert)
- Planet capture (space-only Remake): hostile faction alone in orbit and no enemy starbase changes the owner (inferred).
- Shields: continuous regen plus `Depleted_Shield_Disable_Time`. No attack-move hardpoint sniping by default.
- Default AI = Remake AI plus strategy layer, difficulty Normal; Brutal and Fair are our own tiers (estimates).
- Save = real state snapshot (sim, story engine, Lua globals, AI memory). AI-vs-AI battles use auto-resolve.
- All invented numbers are named estimates in the RULES.md / STATUS.md of their crate.

## Verification flags
`--offscreen out.png`, `--campaign`, `--select`, `--zoom`, `--weeks`, `--build`, `--move`, `--hover`, `--stop-on-battle`,
`--battle-choice fight|auto`, `--battle-auto`, `--battle-seconds`, `--battle-orders`, `--planet-view`, `--via-menu`, `--screen`,
`--click`, `--grant-unit/--at`, `--save-to` / `--load-from`; `SP_DROID_TAB=history|tech|summary` opens a droid menu tab.
