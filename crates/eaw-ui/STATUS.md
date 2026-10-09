# eaw-ui status

Original Empire at War UI rebuilt in Bevy 0.19 UI from the game's data (galactic conquest + space
tactical only; no story or ground UI). Findings and design details: `UI.md`.

## What works
- Galactic and space-tactical command bars, Empire/Rebel/Underworld skins, any resolution
  (design space 1366x768, scale = height/768). Layout from the shell `.alo` models, frame art
  rasterized from the faceplate meshes, textures from `MT_COMMANDBAR` (decoded async).
- Buttons with state textures, tooltips (string table), build queue/pads with unit icons,
  credits/pop/planet texts, selection grid with health/shield bars, ability buttons
  (`I_SA_<TYPE>` icons + `TEXT_TOOLTIP_ABILITY_*` text), retreat countdown, reinforcement label.
- Panels: fleet organizer, planet screen (no ground parts), reinforcement/hero frames, galactic
  and tactical minimap content, GC message dialog, ship overlays (original bar textures, name
  labels), hardpoint markers (original reticles), slot drag and drop.

## API (see `src/lib.rs`)
`EawUiPlugin::new(fs).with_game_data(game)`; `commands.spawn_eaw_command_bar(CommandBarSpec)`;
`spawn_shell(world, &ShellSpec)`; `spawn_dialog`. Data in: resources `CommandBarData`, `PanelData`,
`MinimapData`, `TacticalData` (set `active`). Events out (Bevy messages): `EawButtonClicked`,
`EawAbilityClicked`, `EawSelectionClicked`, `EawHardpointClicked`, `EawSlotDropped`,
`EawMinimapClicked`, `EawDialogResult`; `UiTexturesLoaded` when textures are ready.
Set `EawUiSettings::camera` when rendering to an image. Fonts come from the OS (feature
`system-fonts`): Bahnschrift / Arial; the original EmpireAtWar fonts are not available.

## Run
`cargo run -p eaw-ui --example ui_demo -- --offscreen out.png [--faction Rebel|Empire|Underworld]
[--mode galactic|space] [--size WxH] [--panel planet|fleets|reinforce|hero|dialog|minimap-galactic|minimap-tactical|tactical]
[--hover name] [--tooltip name] [--select name]` (no window with `--offscreen`).
Tests: `cargo test -p eaw-ui` (2 integration tests need the game install and skip without it).

## Gaps
- Not built: planet screen build/unit pads (engine lays them out), black market/corruption/other
  dialogs, advisor movie, SFX, shell idle animations, mouse-over flash, radial progress dials
  (a rising tint is used), selection brackets use drawn corners (no original texture).
- `Scale`/`Icon_Offset` semantics inferred; some buttons may be a pixel or two off.
- `ForcedTooltip` anchor is approximate; real hover uses the cursor.
- Windowed mode of `ui_demo` and real mouse interaction (clicks, drag, minimap) were never run;
  only offscreen renders and unit tests were verified.

## Next steps
Integrate with the app crate (feed the data resources from the sim, handle the messages), test
interaction in a real window, build the planet-screen pad layout, replace the tint with radial
dials, and compare screenshots against the real game.

## Tactical round (2026-10-08)
- `TacticalOverlays { ships, hardpoints }` is a separate resource (was `TacticalData::overlays/hardpoints`); `ShipOverlay::always` shows the bars of own ships without brackets.
  Bars sit under the ship box, brackets are 7 px corners, own selected = green.
- `b_droid_help*` is spawned first so the pause / fast-forward buttons stay above its big rectangle (real clicks used to hit the droid).
- Re-exports: `TacticalOverlayRoot`, `TacticalOverlays`. `examples/ui_demo.rs` needs `CommandBarData { month_income, month_progress, planet_color }` (integrator's change) to compile.

## Planet-view structure row (2026-10-09)
- `CommandBarData::pads_bottom_row`: the pad entries fill only the bottom `b_create_NN` row, left to right (Remake planet view, v0260-v0450); other pads draw nothing. `PadRemap` (written by `widgets::slots`) maps slot to entry; the app uses `PadRemap::entry` for clicks. Space-tab galaxy bar unchanged.
- App pointer test (`SP_UI_TEST`, smoke `windowed-ui-pointer`) opens the planet view, clicks the left-most affordable pad of the bottom row and checks the structure lands in the queue.

## Galaxy tooltips and queue lanes (2026-10-09, QA U6-U12)
- Tooltip panel is rich: icon + title row, divider, wrapped body, stat rows (two columns) and a red warning line. Text format
  (`TooltipLayout::parse`): first line = title, `@Label<TAB>value` = stat row, `!text` = warning, everything else = body.
  The panel children are rebuilt on every change (reusing text nodes left the panel one row too short) and the panel gets a
  fixed width estimated from the text (`tooltip_width`); it stays invisible for two frames until the layout settles.
- Anchor: build pads (`b_create_*`), queue slots and the new `WorldTooltip` resource (planet / fleet under the cursor, set by the app)
  sit right of the advisor droid above the info panel, with three thought-bubble dots; other buttons keep the cursor anchor.
- `EawButton::tooltip_icon` (pad / queue / ability icon) and `marker_tooltip` (hovering the small upkeep / income marker of a
  pad shows its own explanation) are filled from `CommandBarData`. `BuildPadEntry::marker_tooltip` is new.
- `QueueEntry::lane` (0 space = left, blue strip; 1 ground = right, orange strip) and `count == 0` = empty slot. The app fills
  `queue00..04` with space items and `queue05..09` with ground structures.
- Tab buttons (`b_space_tab`, `b_planet_tab`) honour `<Mouse_Over_Offset>` (the hover tile of the tall tab sheet); before, the
  hover layer ignored it and drew the wrong tile through the frame (`CommandBarComponent::mouse_over_offset`).
