# Playtest guide

First live test of **Space Peace: Not at War**. The game is a work in progress: the galaxy map, the Remake scripts and AI,
the tactical battle round trip and save/load work in offscreen tests, but the real-window input has had little live testing.
Please note anything odd and send the log file (see below).

## Build and run

Requirements: Windows 10/11, the Steam copy of *Empire at War - Gold Pack* (Forces of Corruption) and the Workshop mod
*Empire at War Remake 4.0* (id 1770851727), Rust (installed by the script if missing).

```powershell
.\build.ps1            # release build, 10-20 min the first time
.\target\release\space-peace.exe
```

Useful flags: `--fullscreen`, `--no-vsync`, `--volume 0.5`, `--campaign <name>` (skip the menu),
`--difficulty easy|normal|hard|brutal|fair`, `--classic-ai`, `--load-from <save.json>`, `--game-dir <path>`, `--help` lists all.

## A first game

1. Main menu: **Galactic Conquest**, pick a galaxy and your side. Under the list: **AI difficulty** (Easy / Normal / Hard come
   from the mod, Brutal and Fair are custom) and **Classic AI** (the built-in AI instead of the Remake AI).
2. The galaxy starts running at 1x ("Preparing galaxy..." for about a second). Press **Space** to pause.
3. Click one of your planets (ringed in your colour): the bottom bar shows its build pads. Click a pad to queue a unit or structure
   (credits are paid at once). The top row of pads are ships and star base upgrades, the planet tab (second bar button) lists structures.
   Hover a pad for name, cost and build time.
4. Click a fleet (small ship marker next to a planet) or drag a box around fleets, then **right-click** a planet or drag the
   fleet onto it. A green ring marks the destination and the fleet line shows the route and ETA. Fleets in hyperspace cannot be
   redirected, order a retreat first.
5. When hostile fleets meet at a planet with you, a dialog offers **Fight** (tactical battle) or **Auto-resolve**.

## Controls

Galaxy map

| Input | Action |
|---|---|
| Left click | select planet / fleet; drag = box select fleets; drag a fleet onto a planet = move |
| Right click | move selected fleets to the planet under the cursor |
| Mouse wheel | zoom |
| Right or middle drag, WASD / arrows, screen edge | pan |
| Space | pause / resume |
| 1 - 5 | game speed levels |
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
| Bottom bar | selection slots, ability buttons, retreat (available after the countdown) |
| **F10** | abandon the battle: the galaxy simulation auto-resolves it |

Saving: Esc -> **Save Game** -> a slot (5 slots). Loading: **Load Game** in the main menu (any slot, restarts that galaxy) or in
the pause menu (same galaxy only). Saves live in `%APPDATA%\SpacePeace\saves\slotN.json` and replay the game when loaded, so
loading a long game takes a while (a progress bar shows the week reached).

## What to try

- Build a few ships and structures, watch the production queue and the credit counter.
- Send a fleet along two or three jumps; let the AI take planets and watch the notification ticker (top left).
- Fight a battle yourself, then auto-resolve another. After a battle the result (losses, captured planet) shows on the map.
- Save, quit to the menu, load; try Hard and the classic AI.
- Resize / fullscreen / different resolutions, 4K if you have it (the UI scales with the window height).

## Known gaps

- No reinforcements in battles yet (needs a galaxy-sim API), no ground combat, no story campaigns, no cinematics.
- Production queue entries cannot be cancelled; planet-screen pad layout is the simple version of the original.
- The tactical minimap shows the galaxy picture as background, not the battle map.
- Map lighting follows the map data: red sun systems (e.g. Celanon) make ships look red and dark.
- Window mode, click handling, hotkeys, audio and fullscreen were only partly tested live; audio is silent in the offscreen tests.
- Performance: if the frame rate is low, send the numbers from F3 and what was on screen.

## Logs and reporting problems

Every run writes a log to `%APPDATA%\SpacePeace\logs\space-peace-<time>.log` (the 10 newest are kept, panics are included).
Settings are in `%APPDATA%\SpacePeace\settings.toml`. Please send the newest log with a short description of what you did.
