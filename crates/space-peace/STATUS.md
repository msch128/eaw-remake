# space-peace (app crate) - status

Bevy 0.19.1 shell for "Space Peace: Not at War": Galactic Conquest only (no story, no movies).

## What works
- Screens (`flow.rs`, `AppScreen`): Loading -> MainMenu -> CampaignSelect -> Galaxy; SpaceBattle is a placeholder.
- Loading (`loading.rs`): game data parsed on a worker thread, galaxy backdrop + hidden planet-model warm-up, progress bar.
- Menus (`menu.rs`, `options.rs`): main menu (Galactic Conquest / Options / Quit), campaign list grouped by size with
  Rebel/Empire/Underworld, pause menu on Esc, options overlay. UI is built from plain `MenuSpec` data via `spawn_menu`
  (single replacement point for the future `eaw-ui` look).
- Settings (`settings.rs`): window mode, resolution, vsync, FPS cap, UI scale, volumes, edge scroll, camera speed;
  stored in `%APPDATA%\SpacePeace\settings.toml`; CLI flags override per run.
- Galaxy map (`galaxy_map.rs`, `planet_visual.rs`, `camera.rs`, `ui.rs`): real `.alo` planets via `alamo-render`,
  galaxy disc/stars from `w_galaxy00`, route lines from TRADEROUTELINES.XML, declutter labels, owner rings, selection,
  info panel (ability, structure slots, starting forces), faction bar, tooltip, click/hover SFX.
- Audio: `alamo-audio` (music director: MainMenu / Galactic(faction), GUI SFX). UI sends `UiSound`, played only when audio is active.
- Game data mapping (`game.rs`): campaigns, planets (dummies filtered), exact ownership, routes, props, light.
- Offscreen testing: `--offscreen out.png` renders 3840x2160 headless (no window, no audio) and logs frame times (~4 ms/frame).

## CLI flags
`--game-dir`, `--mod <id>`, `--campaign <name>`, `--list-campaigns`, `--demo`, `--volume`, `--no-vsync`, `--fullscreen`,
`--offscreen <png> [--frames N] [--screen menu|campaigns|galaxy|pause|options|loading] [--select <planet>] [--zoom 0..1]`,
`--loading-delay <s>`. No flags: start on the menu; `--campaign`/`--demo` skip to the map.

## SpaceBattle seam (`battle_seam.rs`, plan in the module doc)
galaxy-play `PlayEvent::OpenTacticalBattle(PendingBattle)` -> `OpenBattle` -> `AppScreen::SpaceBattle` (galaxy scene kept hidden) ->
host builds `battle_sim::BattleSetup` + `BattleSession` for battle-view -> `BattleResult` -> `BattleFinished` -> back to Galaxy.
F9 on the map opens the placeholder screen. Needed: galaxy-sim API to apply a tactical `BattleResult` to a pending battle
(today only headless `resolve_battle_tactical` / AutoResolve); battle-view must let the galaxy camera be switched off.

## Gaps
- Not wired: galaxy-play (fleets/sim driver), battle-view, music for battles.
- Menu/HUD look is a plain placeholder; live window input (clicks, Esc, options) only checked via unit tests/offscreen renders.
- alamo-render: emissive-only materials render flat (Stars dome patched locally), no `Nebula.fx` shader, black-hole TGA type 78 textures fail.
- Starting-force names are prettified ids (no object text ids in alamo-data); Camera_Shift_*/Camera_Distance unused.

## Next steps
Connect galaxy-play plugins and the battle round trip, switch menus to `eaw-ui`, run the app in a real window (input, settings apply,
fullscreen, FPS cap), then galaxy-sim result injection.
