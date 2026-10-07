# galaxy-play status

Bevy plugin that makes the galactic map playable: `galaxy-sim` runs on a worker thread, fleets are drawn on the map,
the player selects and orders them, builds at owned planets, controls game speed and gets notifications and battle prompts.

## What works
- `driver.rs`: threaded `SimDriver` (commands, advance, battle resolution, snapshots), headless-testable.
- `GalaxyPlayPlugin::deferred()` + `start_play(world, PlayInit)` / `stop_play(world)`: the host starts a game when a campaign
  opens (the `space-peace` app does this on entering the map) and stops it when leaving. `GalaxyPlayPlugin::new(init)` still
  starts one at plugin build (used by the demo example).
- Fleets on the map (generic fleet model or flagship), orbit slots, lane travel, selection, box select, drag-and-drop and
  right-click move orders with route/ETA hint, merge/split/retreat, blockade-aware route planning.
- `BuildMenu` (units, structures, star base, tech, research) with availability reasons; `PlayAction::Build` queues it, credits go
  through galaxy-sim. Production queue model.
- `GameClock`: pause + speed levels, `hold` flag for the host (pause menu, battle screen); time is held while a human battle waits.
- Notifications (captured/lost planets, battle pending/resolved, construction, income, refused orders).
- `PlayUiModel` (plain data) is the UI seam: `GalaxyPlayUiPlugin` is a simple built-in UI; the app uses `eaw-ui` instead.
- `PlayCursorBlocked` lets the host disable mouse handling (over its UI, menus, other screens).
- Battles: with `PlayConfig::tactical_hook` "Fight" emits `PlayEvent::OpenTacticalBattle`; the host reports back with
  `PlayAction::ResolveBattle` (currently always auto-resolved).

## Run
`cargo run -p galaxy-play --example galaxy_play_demo -- --offscreen out.png [--weeks N] [--human] ...` (no window).
Integration test path: `space-peace --offscreen out.png --campaign GC_Full_Web_Way_Rebel --select "The Yavin System" --weeks 8 --build Squadron --move Hoth`.

## Gaps
- Tactical results cannot be applied to a pending battle yet (needs galaxy-sim API, see `space-peace/STATUS.md`).
- No order to cancel a queue entry (no galaxy-sim command).
- Fleet picking/dragging was only verified in offscreen scenarios and unit tests, never with a real mouse.
- Orders to fleets in hyperspace are refused (retreat first).
