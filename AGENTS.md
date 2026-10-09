# Agent guide (any coding agent: Gemini, Codex, Claude, ...)

Read [`CLAUDE.md`](CLAUDE.md) first: project description, crate map, conventions and hard rules. Then
[`HANDOFF.md`](HANDOFF.md) (overall state), the `STATUS.md` of every crate you touch, and the open work list
`docs/QA-2026-10-09-user.md` (items U1–U27; U1–U24 except U16 are done, see `git log`).

## Hard rules
1. **Game and workshop folders are read-only.** Never write under the Steam install or the workshop content folder.
   No game files, assets or reference screenshots in the repo, ever. The repo stays private.
2. **The user is often at this machine.** While they are:
   - only `--offscreen` runs and unit tests;
   - **no** `--hidden` / `windowed-*` smoke scenarios and **no** input tests (`SP_UI_TEST`, `SP_INPUT_TEST`,
     `SP_ORGANIZER_TEST`): they open a real window, can move the real mouse and open the audio device;
   - never take desktop screenshots, never open visible windows.
3. **Never touch the user's real settings** in `%APPDATA%\SpacePeace`. Test runs use a scratch `APPDATA`
   (`tools/smoke.ps1` does this) and `--volume 0` (implies `--no-audio`).
4. **One item per commit.** The commit message starts with the item ID (e.g. `U26: ...`). Commit only the files you
   changed for that item; never `git add -A` blindly.
5. **Before every commit:** `cargo check --workspace --all-targets` must pass, plus the tests of the crates you
   changed (`cargo test -p <crate>`). Behaviour changes need a test that fails before the fix.
6. **Gameplay numbers come from the Remake XML data**, never invented. An estimate is named as such in the config
   struct and in the crate's `RULES.md`.
7. Code, comments and docs in English. Talk to the user in German.
8. Sims (`galaxy-sim`, `battle-sim`) stay deterministic and Bevy-free; player and AI actions go through journaled
   commands so saves and replays stay consistent.
9. No CI, no prebuilt binaries, no new dependencies without a reason.

## Build, test, play
- User exe: `.\build.ps1` → `target\release\space-peace.exe` (4–8 min). It builds the working tree.
- Use your own `CARGO_TARGET_DIR` (e.g. `target/agent`) for checks so you don't block the user's build.
- Regression: `.\tools\smoke.ps1 -Exe <exe> [-Only <name>]` (offscreen scenarios are safe while the user is here;
  `windowed-*` are not). Soak: `.\tools\soak.ps1` (only when the user is away).
- Visual check: `space-peace.exe --offscreen out.png --frames N ...` and look at the PNG yourself.

## When unsure
Ask the user (in German) instead of guessing. Report honestly what is verified and what is not.
