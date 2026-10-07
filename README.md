# Space Peace: Not at War

A lightweight, modern Rust engine that plays **Star Wars: Empire at War – Forces of Corruption** with the
**Empire at War Remake 4.0** mod (galactic conquest, economy, space battles – no ground combat):
smooth, in 4K, on all CPU cores.

> **No game files in this repository.** The engine reads models, textures, music and data at runtime from
> your own purchased installation (Steam: *Star Wars Empire at War: Gold Pack*) and the subscribed Workshop mod.
> No assets are redistributed.

## Requirements

- Windows 10/11 (x64)
- *Star Wars Empire at War: Gold Pack* on Steam
- Workshop mod *Empire at War Remake* (ID `1770851727`) subscribed

## Building

Double-click **`build.bat`**. It installs Rust and the MSVC Build Tools if missing (one-time, needs admin
confirmation) and builds the game. Alternatively:

```powershell
.\build.ps1          # release
.\build.ps1 -Debug   # debug
```

There are intentionally no prebuilt binaries and no CI – everyone builds locally.

## Layout

| Crate | Purpose |
|---|---|
| `crates/alamo-fs` | MEG archives, layered file system (mod → FoC → EaW), install detection |
| `crates/alamo-data` | Game data: XML (galaxy, planets, trade routes, factions), `.dat` string tables |
| `crates/space-peace` | The game (Bevy) |

## Status

Early development. First milestone: the Remake 4.0 galaxy map with the original music.
