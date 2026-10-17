# Agent guide

Instructions for any coding agent (Claude, Codex, Gemini, ...) working in this repository. `CLAUDE.md` and `GEMINI.md`
only point here, because those tools load their own file name.

## Project

EAW:Remake is a Rust / Bevy 0.19.1 reimplementation of *Star Wars: Empire at War - Forces of Corruption* that plays the
**Empire at War Remake 4.0** mod (Workshop `1770851727`). Scope: Galactic Conquest and space battles only. No ground
combat, story campaigns or movies. Windows, edition 2024. English only (no localization).

**No game files in the repo, ever.** All assets and data are read at run time from the player's own Steam install
(see `.claude/skills/remake-data`). The repository is public on GitHub.

Where to read first:

| For | Read |
|---|---|
| Crate map, pipeline, simulation versus rendering, where data comes from | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Game rules and whether each is data, estimate or project rule | [docs/RULES.md](docs/RULES.md) |
| File formats and coordinate conventions | [docs/FORMATS.md](docs/FORMATS.md) |
| What is missing, how to see it, where it lives | [docs/KNOWN-GAPS.md](docs/KNOWN-GAPS.md) |
| Build, test, verify, playtest checklist, releasing | [CONTRIBUTING.md](CONTRIBUTING.md) |

Each crate's `src/lib.rs` (or `main.rs`) starts with a `//!` module doc that says what the crate does.

## Hard rules

1. **Game and workshop folders are read-only.** Never write under the Steam install or the workshop content folder.
   No game files, assets or reference screenshots in the repo.
2. **Do not disturb a person using the machine.** While someone is, run only `--offscreen` renders and unit tests:
   no `--hidden` / `windowed-*` smoke scenarios, no input tests (`SP_UI_TEST`, `SP_INPUT_TEST`, `SP_ORGANIZER_TEST`)
   because they open a real window, can move the real mouse and open the audio device, and no desktop screenshots.
   Visual checks are offscreen renders of the engine's own scene (PNG into a scratch folder or `target/`, never the repo).
3. **Never touch real settings or saves** in `%APPDATA%\SpacePeace`. Test runs use a scratch `APPDATA`
   (`tools/smoke.ps1` does), `SP_SAVES_DIR` for scripted runs, and `--volume 0` (implies `--no-audio`). No audible
   playback in tests.
4. **One item per commit.** Commit only the files you changed for it; never `git add -A` blindly. Subject at most 72
   characters, blank line, short body. No co-author or tool attribution trailers.
5. **Before every commit:** `cargo check --workspace --all-targets` passes, plus the tests of the crates you changed
   (`cargo test -p <crate>`). A behaviour change needs a test that fails before the fix.
6. **Gameplay numbers come from the Remake XML data**, never invented. A rule the data does not define is a named
   estimate in a config struct (`SimConfig`, `BattleConfig`, `defs.rs`) and in [docs/RULES.md](docs/RULES.md).
7. Sims (`galaxy-sim`, `battle-sim`) stay deterministic (seeded RNG, no global state), `Send` and Bevy-free; player and
   AI actions go through journaled commands so saves and replays stay consistent.
8. No CI. Release setups are built locally with `tools/package.ps1` and published by hand. No prebuilt binaries in the
   repo, no new dependencies without a reason.
9. **Do not quote chats or issue discussions in commit messages, code comments or docs; write neutral English.** No local
   paths with user names.
10. Code, comments and docs are English. Reply to people in the language they write in.
11. **No silent regressions.** Never drop a feature to make something else work. Play a full battle and a few galaxy weeks
    before shipping a change that touches both.

## Conventions

* Parsers are lenient: warnings instead of errors. Keep undecoded bytes raw and mark inferred fields "Inferred" or
  "(guess)".
* Alamo space is Z-up with the bow at -Y. To Bevy: `(x, y, z) -> (x, z, -y)`. To battle-sim: `(x, y, z) -> (-y, x, z)`.
* Keep crates clippy-clean. Docs describe the current state, not a diary: no dates in file names; update
  [docs/KNOWN-GAPS.md](docs/KNOWN-GAPS.md) and [docs/RULES.md](docs/RULES.md) when a gap closes or a rule changes.

## Build and verify

* Release exe: `.\build.ps1` -> `target\release\eaw-remake.exe` (4 to 20 minutes). Use your own `CARGO_TARGET_DIR` per
  area (`target/app`, `target/render`, `target/battle`, `target/ui`, ...) for checks, so you do not block another build
  or overwrite a running exe. Two worktrees must not share one target folder.
* Visual check: `eaw-remake.exe --offscreen out.png --frames N ...`, then open the PNG and look at it.
* Regression: `.\tools\smoke.ps1 -Exe <exe> [-Only <name>]` (`-Only` is a substring filter; offscreen rows are safe
  while someone is using the machine, `windowed-*` are not). Leak, crash and frame-time guard: `.\tools\soak.ps1` (only
  when the machine is free).
* Details and per-area commands: [CONTRIBUTING.md](CONTRIBUTING.md) and `.claude/skills/eaw-verify`.

## Project skills

`.claude/skills/`: `eaw-formats` (file formats and coordinates), `remake-data` (finding and trusting game data),
`eaw-verify` (build, test and offscreen checks), `eaw-reverse-engineer` (decoding unknown engine behaviour).

## When unsure

Ask instead of guessing. Report honestly what is verified and what is not; an offscreen PNG does not prove that clicking
works.
