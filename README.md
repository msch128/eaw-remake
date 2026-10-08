# Space Peace: Not at War

An unofficial, lightweight Rust/Bevy engine that plays the **Galactic Conquest** and **space battles** of
*Star Wars: Empire at War - Forces of Corruption* with the Workshop mod
**[Empire at War Remake 4.0](https://steamcommunity.com/sharedfiles/filedetails/?id=1770851727)**.

> **Bring your own game.** This repository contains no game or mod files. The engine reads all models, textures,
> music and data at runtime from your own legally purchased Steam installation. No assets are included or
> distributed, and no prebuilt binaries are provided - you build it yourself.

## Status

**Work in progress - the whole loop runs, but it has had very little testing with a real mouse and keyboard.**

What works (verified with an automated offscreen smoke test, `tools\smoke.ps1`, and rendered pictures):

- Title screen on one of the game's own space maps, campaign and difficulty selection, faction loading screens.
- Galactic Conquest on the Remake's galaxies: the real planet models, the original command bar and minimap, fleets, build
  and move orders, production queue with cancel, notifications, pause and speed control, the Remake's story scripts and
  AI (several difficulty levels, or the built-in classic AI), save and load, superweapon planet destruction, victory and
  defeat screens with statistics.
- Space battles from the galaxy: the original tactical bar, orders, abilities, retreat, reinforcements, result dialog and
  the result applied back to the galaxy.

What is still rough or missing: real-input testing, sound and music were never heard in testing, entering a battle can
still hitch briefly while models load, AI balance needs live play, and ground combat, story campaigns and
cinematics are out of scope. Details: [`PLAYTEST.md`](PLAYTEST.md), [`HANDOFF.md`](HANDOFF.md) and the `STATUS.md` in each
crate.

## Goals

- 60+ FPS at 4K, using all CPU cores (simulations are parallel and run off the render thread).
- The look and feel of Empire at War: original UI, models, effects, sounds and music, read from your install.
- Galactic Conquest and space battles only. Ground combat, story campaigns and cinematics are out of scope.

## Requirements

- Windows 10/11 (x64)
- A legally owned Steam copy of *Star Wars: Empire at War - Gold Pack* (includes *Forces of Corruption*)
- The Workshop mod *Empire at War Remake* (ID `1770851727`), subscribed so Steam has downloaded it
- A Rust toolchain and the MSVC Build Tools; `build.ps1` installs both if missing

## Build and run

```powershell
.\build.ps1            # release build (or double-click build.bat)
.\build.ps1 -Debug     # debug build
.\target\release\space-peace.exe
```

There are intentionally no prebuilt binaries and no CI.

### Finding your game

The install is detected automatically: the Steam path is read from the registry, every library in
`libraryfolders.vdf` is searched for the game (Steam app `32470`), and the Workshop content is expected next to it
(`steamapps/workshop/content/32470/1770851727`). To override:

- set the environment variable `EAW_GAME_DIR` to the game root (`.../steamapps/common/Star Wars Empire at War`), or
- pass `--game-dir <path>` on the command line.

Other useful flags: `--mod <id>`, `--campaign <name>`, `--list-campaigns`, `--fullscreen`, `--no-vsync`,
`--volume <0..1>`; run with `--help` for all. User settings (window, graphics, audio, camera) are stored in
`%APPDATA%\SpacePeace\settings.toml`, never in the repository.

## Architecture

| Crate | Purpose |
|---|---|
| `alamo-fs` | MEG archives, layered file system (mod, Forces of Corruption, base game), Steam install detection |
| `alamo-data` | Lenient XML, string tables, campaigns, planets, game objects, typed space unit definitions |
| `alamo-model` | `.alo` models, `.ala` animations, particle systems, texture decoding |
| `alamo-map` | `.ted` space battle maps |
| `alamo-audio` | SFX, speech and music events, WAV/ADPCM decoding, Bevy audio plugin |
| `alamo-lua` | Lua 5.0-compatible runtime for the Remake's scripts, story engine, `ScriptHost` trait |
| `galaxy-sim` | Headless deterministic Galactic Conquest simulation |
| `battle-sim` | Headless deterministic, parallel space battle simulation |
| `galaxy-script` | Runs the Remake's plot scripts on `galaxy-sim` |
| `galaxy-ai` | The Remake's galactic AI (goals, perceptions, plan scripts) |
| `alamo-render` | Bevy plugin: models, EaW-style shaders, particles, projectiles, animation |
| `battle-view` | Bevy plugin: renders and controls a `battle-sim` battle |
| `eaw-ui` | The original EaW UI rebuilt from game data |
| `galaxy-play` | Bevy plugin: galaxy map gameplay (fleets, build and move orders) |
| `space-peace` | The application |

## Principles

- **No assets in the repository.** Everything comes from the user's install at runtime; real-data tests skip when
  the game is not found.
- **Data-driven numbers.** Gameplay values come from the game data. Rules the closed-source engine hides are
  documented as named estimates (see `crates/battle-sim/RULES.md`, `crates/galaxy-sim/RULES.md`).
- **Deterministic simulations.** The sims are headless, seeded, free of global state and independent of Bevy.
- **Lenient parsers.** Unknown data produces warnings, not crashes; undecoded bytes are kept raw.

## Contributing

Issues and pull requests are welcome. Project rules for contributors (and AI agents) are in
[`CLAUDE.md`](CLAUDE.md) and [`AGENTS.md`](AGENTS.md). Please never commit game or mod files. Binaries are not
provided by design.

## Credits

- The **[Empire at War Remake](https://steamcommunity.com/sharedfiles/filedetails/?id=1770851727)** team, whose
  mod provides the data this engine plays.
- **Petroglyph Games** for the original *Star Wars: Empire at War* and its expansion.
- The **[Bevy](https://bevy.org)** engine and its community.

## Legal / Disclaimer

Space Peace: Not at War is an unofficial, non-commercial fan project. It is **not affiliated with, authorized,
sponsored or endorsed by** Disney, Lucasfilm, LucasArts, Petroglyph Games, Valve/Steam, or the Empire at War Remake
team. *Star Wars* and all related names, characters and marks are trademarks of their respective owners and are
used here only descriptively, to say which game this software works with.

This repository contains no game or mod files, and none are distributed with it; it only contains original code
that reads files from an installation you already own. You need your own legal copy of the game and the mod to use
it. Do not use it to obtain or distribute copyrighted material.

The source code is released under the [MIT License](LICENSE).
