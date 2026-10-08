# Playtest guide

First live test of **Space Peace: Not at War**. The whole loop (title screen, galaxy map, battle, result, end screen,
save/load) runs and passes an automated offscreen smoke test (`tools\smoke.ps1`, 18 scenarios), but almost everything
was checked with scripted input and rendered pictures, not with a real mouse and keyboard. Expect rough edges in input,
camera feel and performance on your machine, and please send the log file (see the end).

## Build and run

Requirements: Windows 10/11, the Steam copy of *Empire at War - Gold Pack* (Forces of Corruption) and the Workshop mod
*Empire at War Remake 4.0* (id 1770851727), Rust (installed by the script if missing).

```powershell
.\build.ps1            # release build, 10-20 min the first time
.\target\release\space-peace.exe
```

If the game or the mod is not found, the title screen says what is missing and how to fix it (install via Steam,
subscribe to the Workshop mod, or `--game-dir <path>`).

Useful flags: `--fullscreen`, `--no-vsync`, `--volume 0.5`, `--campaign <name>` (skip the menu),
`--difficulty easy|normal|hard|brutal|fair`, `--classic-ai`, `--load-from <save.json>`, `--game-dir <path>`, `--help` lists all.

## A first game

1. Title screen (one of the game's own space maps, slowly drifting): **Galactic Conquest**, pick a galaxy and your side.
   Under the list: **AI difficulty** (Easy / Normal / Hard come from the mod, Brutal and Fair are custom) and **Classic AI**
   (the built-in AI instead of the Remake's scripts and AI).
2. A faction loading screen shows while the galaxy is prepared (about a second). The map opens on your richest planet with
   it selected. The galaxy runs at 1x; press **Space** to pause.
3. The bottom bar shows the planet's build pads. Click a pad to queue a ship, squadron, structure or upgrade (credits are paid
   at once; hover for name, cost, build time, and why it is greyed out). The second bar tab lists structures. Click a queue
   slot to **cancel** it; the tooltip shows the refund and a message confirms it.
4. Click a fleet (marker next to a planet) or drag a box around fleets, then **right-click** a planet or drag the fleet onto
   it. A green ring marks the destination, the fleet line shows route and ETA. Fleets in hyperspace cannot be redirected,
   order a retreat first.
5. When hostile fleets meet at a planet with you, a dialog offers **Fight** (tactical battle) or **Auto-resolve**.
6. After a battle a result dialog lists each side's units, survivors and losses. A won campaign (or a lost one) ends with a
   Victory / Defeat screen with your statistics; you can stay on the map or return to the menu.
7. **Superweapon:** with a fleet that has a planet destroyer (e.g. the Death Star) in orbit of a hostile planet, a red
   "Fire superweapon" button appears; a confirmation dialog follows. The planet becomes an asteroid wreck.

## Controls

Galaxy map

| Input | Action |
|---|---|
| Left click | select planet / fleet; drag = box select fleets; drag a fleet onto a planet = move |
| Right click | move selected fleets to the planet under the cursor |
| Mouse wheel | zoom |
| Right drag, WASD / arrows, screen edge | pan |
| Middle drag (sideways = rotate, up/down = tilt), Page Up / Page Down | rotate the view |
| Space | pause / resume |
| 1 - 5 | game speed levels (also the bar's play/pause and fast-forward buttons) |
| M / V / R | merge / split / retreat the selected fleets |
| Minimap click or drag | move the camera |
| Esc | pause menu (Resume, Save Game, Load Game, Options, Main Menu, Quit) |
| F3 / F11 | FPS counter / fullscreen |

Tactical battle

| Input | Action |
|---|---|
| Left click / drag | select ships / box select |
| Right click | move; on an enemy ship = attack (on a hardpoint marker = attack that hardpoint) |
| Shift + right click | attack-move |
| S | stop |
| Ctrl + 0-9, 0-9 | store / recall control groups; Ctrl + A selects all |
| WASD / arrows, Q / E, middle drag, wheel | camera |
| Bottom bar | selection slots, ability buttons, retreat (after the countdown) |
| Reinforcement button (bar) | opens the reserve window: units that did not fit under the unit cap; click a slot to call one in (Shift: all). A refusal at the cap shows a message |
| Minimap | tactical minimap (map objects, ships by side, camera); click or drag to move the camera |
| **F10** | abandon the battle: the galaxy simulation auto-resolves it |

Saving: Esc -> **Save Game** -> a slot (5 slots). Loading: **Load Game** in the main menu (any slot, restarts that galaxy) or in
the pause menu (same galaxy only). Saves live in `%APPDATA%\SpacePeace\saves\slotN.json` and replay the game when loaded, so
loading a long game takes a while (a progress bar shows the week reached).

Options (title screen or pause menu): window mode, resolution, vsync, FPS limit, UI scale, volumes, edge scrolling, camera
speed, anti-aliasing, bloom, shadows, and a graphics quality preset (which also sets particle density). Settings are saved.

## What to try

- Build a few ships and structures, cancel one, watch the credits and the production queue.
- Send a fleet along two or three jumps; watch the notification ticker (top left) and the AI taking planets.
- Fight a battle yourself (selection, orders, abilities, reinforcements, retreat), then auto-resolve another; read the result dialog.
- Save, go to the menu, load; try Hard, Brutal and the classic AI.
- Resize, fullscreen, other resolutions, 4K if you have it (the UI scales with the window height); try the quality presets
  if the frame rate is low.
- If you have a way to get a Death Star: fire it at an enemy planet.

## Known gaps

- Overall state is about 58% (see `HANDOFF.md`). Weakest areas: tactical battle controls (no action camera,
  reinforcement placement, ability buttons or shield/hull bars yet) and the galaxy map look (routes, fleet/station/hero
  models, background, labels are not EaW-like yet). Report anything else you find.
- No ground combat, story campaigns or cinematics (out of scope).
- Entering a battle: models now preload behind the loading screen, but a short hitch (~60 ms in release tests) can remain;
  hardpoint models still load on first use.
- Unit voices, advisor lines, the victory announcer and music hooks exist but were never heard in testing (playback is
  silent in the automated tests); expect missing or mistimed sounds.
- Title screen: the camera framing is not tuned per map (one map shows mostly nebula); no menu art from the original
  (the original logo plate is deliberately not used).
- Battle loading uses your faction's loading art, not the defender's. An early render of a planet explosion showed
  hard-edged squares; not reproduced since the particle rework – tell us if you see it.
- Real mouse and keyboard input, window handling, hotkeys and fullscreen have had limited live testing (a real-mouse QA
  pass is logged in `docs/QA-2026-10-08.md`).
- Performance: offscreen the galaxy map runs at about 5 ms/frame at 4K. Hidden long runs (`tools\soak.ps1`) show flat
  memory (about 2 GB) and a median p99 under 25 ms; send the F3 numbers and what was on screen if it is slow.

## Logs and reporting problems

Every windowed run writes a log to `%APPDATA%\SpacePeace\logs\space-peace-<time>.log` (the 10 newest are kept, panics
included). Settings are in `%APPDATA%\SpacePeace\settings.toml`. Please send the newest log with a short description of
what you did and what you saw.
