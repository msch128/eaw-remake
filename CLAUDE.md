# Space Peace: Not at War

Rust/Bevy 0.19.1 reimplementation of *Star Wars: Empire at War - Forces of Corruption* playing the
**Empire at War Remake 4.0** mod (Workshop `1770851727`). Scope: Galactic Conquest + space battles only.
No ground combat, story campaigns or movies. Windows, edition 2024.

**No game files in the repo, ever.** All assets and data are read at runtime from the user's Steam install
(see `.claude/skills/remake-data`).

## Hard rules (user decisions)
- **Never open visible windows and never take desktop screenshots** – the user games on this machine. Visual checks are
  offscreen renders of our own scene only (PNG into the scratchpad or `target/`, never the repo).
- **Game and workshop folders are read-only.** No audible playback in tests.
- **No CI / GitHub Actions, no prebuilt binaries.** Users build locally with `build.bat` / `build.ps1`.
- **English only** (no localization). Talk to the user in German.
- **Usage budget:** 5-hour usage window – 7 parallel agents burned ~33% in 25 min. Default to 3–4 agents on the
  critical path, few render iterations, short reports. Current plan and state: `HANDOFF.md`.
- Commit + push to `main` after `cargo check --workspace --all-targets` passes.
- **Never quote the user or the chat in commit messages, code comments or docs; write neutral English. No local paths with user names.**

## Crates (pipeline order)
`alamo-fs` (MEG + layered VFS) -> `alamo-data` (XML, string tables, game objects) / `alamo-model` (.alo/.ala/particles)
/ `alamo-map` (.ted) / `alamo-audio` / `alamo-lua` (script runtime) -> `galaxy-sim` (headless GC) / `battle-sim`
(headless deterministic space battles) / `galaxy-ai` / `galaxy-script` (Lua against galaxy-sim) -> `alamo-render`,
`battle-view`, `eaw-ui`, `galaxy-play` (Bevy plugins) -> `space-peace` (the app).

Every crate has a `STATUS.md` (what works / key decisions / gaps / next steps). Read it before working on a crate
and update it afterwards. Rules and their data sources: `battle-sim/RULES.md`, `galaxy-sim/RULES.md`.
UI findings: `eaw-ui/UI.md`. Lua API inventory: `alamo-lua/API.md`.

## Conventions
- Sims (`galaxy-sim`, `battle-sim`) are headless, deterministic (seeded RNG, no global state) and `Send`. Keep Bevy out of them.
- Gameplay numbers come from the data, never invented. Hidden engine rules are named **estimates** in config structs + RULES.md.
- Parsers are lenient: warnings instead of errors. Keep undecoded bytes raw and mark inferred fields "Inferred"/"(guess)".
- Alamo space is Z-up with the bow at -Y. To Bevy: `(x,y,z) -> (x,z,-y)`. To battle-sim: `(x,y,z) -> (-y,x,z)`.
- Code, comments and docs in English. Keep crates clippy-clean.

## Build and verify
Build per crate with a per-area `CARGO_TARGET_DIR` (`target/app`, `target/render`, `target/battle`, `target/ui`, ...).
Verify visually with headless `--offscreen out.png` renders and look at the PNG. Details: `.claude/skills/eaw-verify`.
Full build: `.\build.ps1` (release) / `.\build.ps1 -Debug`. Regression smoke test of the release exe (offscreen
scenarios, pass/fail table): `.\tools\smoke.ps1 [-Exe path] [-Only name]`.
Leak/crash/frame-time guard (hidden window, `SP_ASSET_LOG=1`, memory + asset-count growth, log spam, p99): `.\tools\soak.ps1 [-Exe path] [-BattleMinutes 5] [-GalaxyMinutes 10] [-Only battle|galaxy|frametime]`.

## Project skills
`eaw-formats` (file formats + coordinates), `remake-data` (finding/trusting game data),
`eaw-verify` (build/test/offscreen checks), `eaw-reverse-engineer` (decoding unknown engine behaviour).
