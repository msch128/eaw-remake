# Contributing

EAW:Remake is a Rust / Bevy 0.19.1 workspace (edition 2024, Windows). This file covers building, testing, verifying a
change, the conventions, a manual playtest checklist and how to cut a release. For the design see
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md); for the rules the simulations implement see [docs/RULES.md](docs/RULES.md);
for what is already known to be missing see [docs/KNOWN-GAPS.md](docs/KNOWN-GAPS.md). Rules for coding agents are in
[AGENTS.md](AGENTS.md).

You need your own Steam copy of *Star Wars: Empire at War - Gold Pack* and the Workshop mod *Empire at War Remake 4.0*
(1770851727). Most tests skip without them; a green run without the game does not prove real data was checked.

## Build

```powershell
.\build.ps1            # release build -> target\release\eaw-remake.exe (4 to 20 minutes the first time)
.\build.ps1 -Debug     # debug build
```

`build.ps1` installs Rust and the Microsoft C++ Build Tools with `winget` when they are missing. Full builds are slow:
build only the crate you touched (`cargo check -p <crate>`). `profile.dev` uses opt-level 1 and dependencies at 3;
release uses thin LTO.

Give every area its own `CARGO_TARGET_DIR` so parallel builds do not block each other on the cargo lock and a build
never overwrites an executable you are running, for example:

```powershell
$env:CARGO_TARGET_DIR = "$PWD\target\render"; cargo check -p alamo-render
```

Typical folders: `target/app` (eaw-remake), `target/render`, `target/battle`, `target/ui`, `target/play`, `target/ai`,
`target/lua`, `target/audio`, `target/tmp` (tools). Two git worktrees must not share one target folder.

## Test

Before every commit:

1. `cargo check --workspace --all-targets` passes.
2. `cargo clippy -p <crate> --all-targets` is clean for the crates you changed (the workspace is kept clippy-clean).
3. `cargo test -p <crate>` passes for the crates you changed. A behaviour change needs a test that fails before the fix.

Real-data tests (`tests/real_data.rs` and similar) skip when the game or mod is missing; look for the skip message in
the output. The parser corpus checks must stay at zero errors: `alamo-model` examples `parse_all` (4,348 models),
`parse_all_ala` (7,685 animations) and `parse_all_particles` (1,505 particle systems).

## Verify

An offscreen picture or a unit test does not prove that clicking works. Say what was and was not checked.

### Offscreen render workflow

Every Bevy front end can render to a PNG without a window and without audio. Open the PNG and look at it before saying a
visual change works. Write PNGs and traces to a scratch folder or `target/`, never into the repository.

| Area | Command (PowerShell, repository root) |
|---|---|
| App (menus, galaxy map) | `cargo run -p eaw-remake -- --offscreen out.png [--screen menu\|campaigns\|galaxy\|pause\|options\|loading] [--campaign <name>] [--select <planet>] [--zoom 0..1] [--frames N]` renders 3840x2160 |
| Models, shaders, particles | `cargo run -p alamo-render --example render_scene -- <model\|-> out.png [--team --sun --cam yaw,pitch,dist --fx --times --many N --shadows --hdr]`; also `showcase <idle\|xwing\|turret\|hyper\|death> out.png` and `battle out.png` |
| Space battle view | `cargo run --release -p battle-view --example battle_demo -- --offscreen out.png --size 3840x2160 --times 5,30,90 --auto [--map ..] [--cam yaw,pitch,dist]`; add `--features profile` and env `TRACE_CHROME=trace.json` for a Chrome trace |
| Interface | `cargo run -p eaw-ui --example ui_demo -- --offscreen out.png --faction Rebel\|Empire\|Underworld --mode galactic\|space [--panel ...] [--hover name]` |
| Battle rules and balance | `cargo run --release -p battle-sim --example battle_duel -- [filter] [delay=S] [snipe] [range=F]`; performance: `battle_bench` |
| Galactic sim | `cargo run --release -p galaxy-sim --example headless_match -- [weeks] [seed] [campaign] [novictory] [tactical]` |
| Lua and AI | `galaxy-script` examples `scripted_match`, `ai_match`, `ai_check`; `alamo-lua` example `run_story` |

The app also takes scripted flags for reproducible scenes: `--campaign`, `--select`, `--zoom`, `--weeks`, `--build`,
`--move`, `--hover`, `--stop-on-battle`, `--battle-choice fight|auto`, `--battle-auto`, `--battle-seconds`,
`--battle-orders`, `--planet-view`, `--via-menu`, `--screen`, `--click`, `--grant-unit` with `--at`, `--save-to` and
`--load-from`. `SP_DROID_TAB=history|tech|summary` opens a droid menu tab. `--help` lists the rest.

### Look checks

When the look or behaviour should match the Remake, state what you compared it with (a data value, a reference frame, a
known number from RULES.md) and measure instead of judging by eye: sizes in pixels at a stated resolution, colours,
counts. Flicker, popping and other effects over time need several frames, not one. Reference frames from the original
game are never committed. Frame-time numbers depend on the machine and on other running programs: report them with the
conditions. Known reference results: Star Destroyer vs MC-80 1v1 takes about 270 s (the MC-80 wins), Victory vs Nebulon-B
about 30 s.

### Smoke test, soak test, setup test

* `.\tools\smoke.ps1 [-Exe <exe>] [-Only <text>] [-Windowed]` runs the release exe offscreen through the key scenarios
  (menu, galaxy, build, move, cancel, drag, battle, save and load, superweapon, missing game, Clone Wars) and prints a
  pass/fail table. `-Only` is a substring filter, so pass exact names. Output goes to a temp folder with its own
  `APPDATA`. The `windowed-*` rows open a real window and need `-Windowed`.
* `.\tools\soak.ps1 [-Exe <exe>] [-BattleMinutes 5] [-GalaxyMinutes 10] [-Only battle|galaxy|frametime]` runs hidden
  long battles and galaxy games and fails on memory above 8 GB, memory or asset-count growth, error lines, repeated
  warnings and a p99 frame time of 33 ms or more.
* `.\tools\test-setup.ps1` installs the setup silently into a scratch folder, runs the installed game, uninstalls and
  checks what is left. It writes and removes the real Start menu entry and uninstall key of the current user.
* `.\tools\audio-audit.ps1` checks which sound events a galaxy and a battle run play.
* `.\tools\look-scenes.ps1 -Exe <battle_demo.exe> -Out <folder> [-Scene Missiles,Death,OrbitExecutor,OrbitStarDestroyer,Reticles,Squadrons,SquadronsSelected,Hunt,SelectedExecutor]`
  renders offscreen frame sequences of the battle view (a missile salvo, a capital ship's death, the camera turning round an
  Executor and a Star Destroyer, the hardpoint reticles while the camera turns, squadrons at three zooms, selected squadrons, squadrons
  hunting, a selected Executor). Run it with the build before and after a look
  change and compare the frames: it is the reference set for look changes.

Hidden or windowed runs and the input tests (`SP_UI_TEST`, `SP_INPUT_TEST`, `SP_ORGANIZER_TEST`) move the real mouse,
open the audio device and take focus. Run them only while nobody is using the machine; otherwise stay with `--offscreen`
runs and unit tests. Test runs must never touch real settings or saves: use a scratch `APPDATA` (smoke.ps1 does) or set
`SP_SAVES_DIR`, and `--volume 0` (implies `--no-audio`).

## Conventions

* **Commits.** Subject at most 72 characters, a blank line, a short body. One item per commit; stage only the files you
  changed for it. No co-author or tool attribution trailers. Do not quote chats or issue discussions; write neutral
  English. No local paths with user names.
* **Language.** Code, comments and docs are English. The game text is English only.
* **Clean-room.** The repository contains original source only: no game files, models, textures, sounds, data, reference
  screenshots or decompiled code. The game folder and the Workshop folder are read-only; tests and tools never write
  there. File formats are documented from the shipped files ([docs/FORMATS.md](docs/FORMATS.md)), and undecoded bytes are
  kept raw instead of guessed.
* **Estimates are labelled.** Gameplay numbers come from the Remake XML and Lua, never invented. A rule the data does not
  define is a named estimate in a config struct (`SimConfig`, `BattleConfig`, `defs.rs`), says so in its doc comment and
  has an entry in [docs/RULES.md](docs/RULES.md). Inferred parser fields are marked "Inferred" or "(guess)".
* **Determinism.** `galaxy-sim` and `battle-sim` stay headless, deterministic (seeded RNG, no global state), `Send` and
  free of Bevy. Player and AI actions go through journaled `Command`s so saves and replays stay consistent.
* **Parsers are lenient.** Unknown chunks and broken mod XML produce warnings, not errors.
* **Dependencies.** No new dependency without a reason. No CI and no prebuilt binaries in the repository; users build
  locally or take the setup from the Releases page.
* **Docs.** Keep the docs a description of the current state, not a diary: no dates in file names, no per-session notes.
  Update [docs/KNOWN-GAPS.md](docs/KNOWN-GAPS.md) when a gap is closed or found, and RULES.md when a rule changes.

## Playtest checklist

Run this against a release build with a real mouse, keyboard and sound before a release and after larger changes.
Mark each item pass or fail and note the build.

Build under test: ____________   Date: ____________

### Start and galaxy

| # | Item | How to test | Pass | Fail |
|---|---|---|---|---|
| 1 | Game and mod found | Start `eaw-remake.exe`. With a wrong `--game-dir` the title screen names what is missing and how to fix it. | [ ] | [ ] |
| 2 | New campaign | Title screen, Galactic Conquest, pick a galaxy and a side; a loading screen shows, the map opens on your richest planet with it selected. | [ ] | [ ] |
| 3 | Difficulty and AI choice | On the new-game screen change AI difficulty (Easy to Fair) and Classic AI; start a game with each of two settings. | [ ] | [ ] |
| 4 | Pause and speed | Press Space, then keys 1 to 5 and the bar's buttons; the clock stops and runs at the chosen speed. | [ ] | [ ] |
| 5 | Camera | Mouse wheel zooms, right drag and WASD or arrows and screen edge pan, middle drag and Page Up / Page Down rotate, minimap click and drag move the camera. | [ ] | [ ] |
| 6 | Build | Select a planet, click a build pad: a ship, squadron, structure and upgrade can be queued; credits are paid at once; hovering shows name, cost, build time and why an item is greyed out. | [ ] | [ ] |
| 7 | Cancel | Click a queue slot: the item is cancelled, the tooltip shows the refund and a message confirms it. | [ ] | [ ] |
| 8 | Sell | Right-click an owned structure or ship: a "Sell for N" line appears at the cursor and a left click on it sells. | [ ] | [ ] |
| 9 | Move fleets | Click a fleet or drag a box around fleets, then right-click a planet or drag the fleet onto it: a green ring marks the destination and the route line shows ETA. A fleet in hyperspace cannot be redirected; M, V and R merge, split and retreat. | [ ] | [ ] |
| 10 | Notifications and AI | Play several weeks: the ticker (top left) reports events and the AI takes planets. | [ ] | [ ] |
| 11 | Menus | Esc opens the pause menu (Resume, Save Game, Load Game, Options, Main Menu, Quit); F3 shows the FPS counter, F11 toggles fullscreen. | [ ] | [ ] |
| 12 | Superweapon | With a planet destroyer (Death Star) in orbit of a hostile planet, a red "Fire superweapon" button appears, a confirmation dialog follows and the planet becomes an asteroid wreck. | [ ] | [ ] |

### Space battle

| # | Item | How to test | Pass | Fail |
|---|---|---|---|---|
| 13 | Battle choice | Let hostile fleets meet at a planet with yours: a dialog offers Fight or Auto-resolve. | [ ] | [ ] |
| 14 | Loading and entry | Fight: a loading screen shows, then the opening camera. Note any hitch when the battle starts. | [ ] | [ ] |
| 15 | Selection and orders | Click and box-select ships; right-click the ground to move, an enemy to attack, a hardpoint marker to attack the hardpoint; Shift + right click attack-moves; S stops. | [ ] | [ ] |
| 16 | Control groups | Ctrl + 0 to 9 stores a group, 0 to 9 recalls it, Ctrl + A selects all. | [ ] | [ ] |
| 17 | Camera and minimap | WASD or arrows and Q / E, middle drag and wheel move the camera; the minimap shows ships by side and the camera and moves it by click or drag; P pauses. | [ ] | [ ] |
| 18 | Abilities | Select a ship with abilities: ability buttons show, activate one, the effect and its recharge are visible. | [ ] | [ ] |
| 19 | Reinforcements | Open the reserve window from the bar, click a slot to call a unit in (Shift: all), place it where allowed; at the unit cap a message refuses. | [ ] | [ ] |
| 20 | Retreat | After the countdown the retreat button works, a second click calls the retreat off. | [ ] | [ ] |
| 21 | Result | A result dialog lists each side's units, survivors and losses; the galaxy applies them. F10 abandons a battle and the galaxy auto-resolves it. | [ ] | [ ] |
| 22 | Auto-resolve | Auto-resolve another battle and read the result dialog. | [ ] | [ ] |

### Save, options, end of game

| # | Item | How to test | Pass | Fail |
|---|---|---|---|---|
| 23 | Save and load | Esc, Save Game, a slot; go to the main menu and Load Game from any slot; loading a long game shows a progress bar. | [ ] | [ ] |
| 24 | Options | Change window mode, resolution, vsync, FPS limit, UI scale, volumes, edge scrolling, anti-aliasing, bloom, shadows and the quality preset; settings persist after a restart. | [ ] | [ ] |
| 25 | Window | Resize, fullscreen, other resolutions, 4K if available; the UI scales with the window height. | [ ] | [ ] |
| 26 | Victory and defeat | Finish or lose a campaign: the Victory or Defeat screen shows statistics and lets you stay on the map or return to the menu. | [ ] | [ ] |

### Sound

| # | Item | How to test | Pass | Fail |
|---|---|---|---|---|
| 27 | Music | Menu, galaxy, battle loading, battle and result each play their own music without gaps or overlaps. | [ ] | [ ] |
| 28 | Effects and speech | In a battle weapons, engines, explosions, unit responses and advisor lines are audible and positioned; a large fight stays balanced and does not clip. | [ ] | [ ] |

### Stability and performance

| # | Item | How to test | Pass | Fail |
|---|---|---|---|---|
| 29 | Frame rate | F3 during a large battle and on the galaxy map; note the numbers and what was on screen. | [ ] | [ ] |
| 30 | Long session | Play 30 minutes; memory stays flat and the log has no errors. | [ ] | [ ] |

### Logs and reporting problems

Every windowed run writes a log to `%APPDATA%\EAW-Remake\logs\` (the ten newest are kept, panics included); settings are
in `%APPDATA%\EAW-Remake\settings.toml`. Attach the newest log to a bug report with what you did and what you saw.

## Releasing

A release is the Windows setup `EAW-Remake-Setup-<version>.exe` on the GitHub Releases page. The version is the
`[workspace.package]` version in `Cargo.toml`. The setup is not code-signed.

1. **Clean tree.** `git status` is clean on `main`; `cargo check --workspace --all-targets` and `cargo test --workspace`
   pass.
2. **Version.** Raise `[workspace.package] version` in `Cargo.toml` (the setup file name and the installer follow it).
3. **Build and smoke.** `.\build.ps1`, then `.\tools\smoke.ps1`. Every row passes or the failure is understood and listed
   in [docs/KNOWN-GAPS.md](docs/KNOWN-GAPS.md). Run the playtest checklist above.
4. **Package.** `.\tools\package.ps1` (needs Inno Setup 6) builds a console-less exe with the C runtime linked in, checks
   that it imports only Windows DLLs and writes `dist\EAW-Remake-Setup-<version>.exe`. Then `.\tools\test-setup.ps1`.
5. **Public-repository checks.** The repository is public, so before any public step:
   * Licenses: [CREDITS.md](CREDITS.md) lists the open-source libraries and the clean-room statement; `LICENSE` is intact and
     the README legal section is unchanged. A new dependency needs a CREDITS.md entry.
   * Claims: the README status paragraph says only what has been run, with current numbers; no self-scored percentages.
   * No game assets: `git ls-files` shows no `.meg`, `.alo`, `.ala`, `.ted`, `.dds`, `.tga`, `.wav`, `.mp3` or `.dat` file
     and no reference screenshots; screenshots in `docs/screenshots` show the engine running with a credit line.
   * No chat quotes, no non-English text, no profanity and no local paths with user names in code, comments, docs or commit
     messages: search the tree and `git log` for them.
   * Links: every relative link in the Markdown files resolves.
6. **Publish.** Tag the commit, create the GitHub release and attach the setup; state in the notes that the setup is
   unsigned and what the status paragraph says.
