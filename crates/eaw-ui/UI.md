# Empire at War UI: findings and design

Everything here was derived from the mod "Empire at War Remake 4.0" (workshop 1770851727) and
the base game; nothing is copied into the repository, all assets are read at runtime.

## Where the UI is defined

| What | Where |
| --- | --- |
| Look of each element (textures per state, font, tooltip text id, flags) | `Data/Xml/CommandBarComponents.xml` (via `CommandBarComponentFiles.xml`), 870 components in the mod |
| **Placement** of each element and the frame art | shell models `Data/Art/Models/I_*.ALO` (flat quads) |
| Small UI textures (buttons, pads, bars, unit icons) | packed into the 4096x4096 mega texture `Art/Textures/MT_COMMANDBAR.TGA`, directory in `MT_COMMANDBAR.MTD` (a `...COMPRESSED.DDS` twin exists) |
| Frame art | `I_GALACTIC_DASHBOARD_WIDE[_REBEL|_UNDER].DDS`, `I_GALACTIC_DASHBOARD_SKIRMISH[_REBEL|_UNDER].DDS` (2048x512) |
| Unit icons | `Icon_Name` in the unit XML (e.g. `I_BUTTON_UNIT_REBEL_X_WING.TGA`), also inside the mega texture |
| Strings | `Text/MasterTextFile_ENGLISH.DAT` (`Tooltip_Text` ids) |

### CommandBarComponents.xml

Root `<CommandBarComponents>` with `<CommandBarComponent Name="...">`. `<Type>` counts in the mod:
`TextButton` 638, `Button` 122, `Bar` 85, `Shell` 19, `Icon` 8. Important tags:

* Textures: `Blank_Texture_Name` (pad/background, a list is indexed by faction), `Icon_Texture_Name`,
  `Icon_Alternate_Texture_Name` (variants by index: faction, state, ...), `Mouse_Over_`,
  `Selected_` (pressed / toggled), `Disabled_`, `Build_` (progress dial), `Flash_`, `Overlay[2]_`,
  `Lower_Effect_`; `Bar_Texture_Name` / `Bar_Overlay_Name` with `Max_Bar_Level` (see below).
* Geometry: `Scale` (multiplier of the texture's natural size), `Icon_Offset`, `Offset`,
  `Text_Offset`/`Text_Offset2` (second line, e.g. the price), `Size`, `Base_Layer`.
* Text: `Font_Name` (`EmpireAtWar-Medium`/`-Bold`), `Font_Point_Size`, `Text_Color[2]`,
  `Left/Right_Justified`, `Text_Outline`, `Max_Text_Width`.
* Behavior: `Tooltip_Text` (several ids = state dependent), `Toggle`, `Tab`, `Click_Shift`,
  `Selected_Alpha`, `Swap_Texture`, `Drag_And_Drop`, `Stackable`, SFX names, `Group`.
* Shells: `Model_Name`, `Model_Offset_X/Y`, `Manual_Offset`, `Hidden`.

Shells: `i_main_commandbar` (`i_galactic_controls_new.alo`), `i_main_skirmish`
(`i_tactical_controls_new.alo`, the space battle bar), `i_main_organize` (fleet organizer),
`map_shell` (`i_planetary_mode.alo`, planet screen), `i_main_reinforce`, `i_build_buttons`,
`good/evil_hero_frame`, `pending_battle_shell`, `autoresolve_shell`, `pause_shell`,
`market_frame`, `bm_frame`, `corruption_shell`, `hack_shell`, `target_type_back`, `ds_shell`.

Component families (by name): `b_*` buttons, `Text_*` labels, `queue00..09` / `tqueue00..09`
production queue slots (galactic / tactical, 5 left + 5 right of the centre), `b_Create_00..39` build
pads (two rows of 20), `s_select_NN` + `s_health_NN` + `s_shield_NN` tactical selection slots
(24, two per column), `special_button_NN` / `special_border_NN` abilities and column frames,
`c_button00..05` command buttons (attack, move, ...), `b_filters*` filter buttons, `land_fleet_NN`,
`q1_/q2_/q3_NNNN` fleet organizer slots, `r_NNNN` reinforcement slots, `hero_slot_*`, `Pad_*`.

### Shell models (placement)

The `.ALO` shells are 2D scenes: every component is a hidden quad whose **bone** gives its centre
and whose bounding box its size. Units are design pixels of the original **1366x768** layout
(`b_zoom` at 28,26; credits at 290,216 ...), x right, y up, origin at the bottom-left of the bar.
Visible meshes are the frame art: `Empire_Faceplate_ALT0`, `Rebel_Faceplate_ALT1`,
`Underworld_Faceplate_ALT2` (`_ALTn` is the faction variant; note Rebel/Empire are swapped
relative to the texture lists below). The galactic bar is 1379x287 design px (about one 1366 wide
screen), the tactical bar 1017 px wide. Both are anchored to the bottom-left corner, the UI scales
with *screen height / 768*. The faceplate quads use mapped regions of the 2048x512 sheets and
slanted edges, so they are rasterized from the mesh rather than drawn as sprites.

Other bones of the galactic shell: `radar_galactic` (171x171 minimap hole), `b_dial`,
`Text_Planet`, `Text_Allegiance_A` (symbol by owner), `Text_Planetary_Income`, `Text_Credits`,
`Text_Day_Counter`, `Text_Tech_Level`, `Text_Pop_Cap`, `b_space_tab`/`b_planet_tab`,
`b_play_pause`, `b_fast_forward`, `b_droid_help`, `Movie_galactic` and the droid help panel
(`b_Help_Droid_*_ALT*`, art that sticks out above the bar).

### Bars

A `Bar` component does not scale a fill: `Max_Bar_Level` (10) pre-rendered textures exist, one per
level (`i_bar_health_small0..10`, `i_bar_shield_small10` is the full shield), plus a frame texture
(`i_bar_control_wide`). The engine picks the texture for `round(fraction * 10)`.

### Factions

Texture lists in the XML are indexed Rebel 0, Empire 1, Underworld 2, Pirates 3, Hutts 4
(`i_button_pad_rebel.tga i_button_pad_empire.tga`; `Text_Allegiance_A`). Frame meshes use their own
`_ALTn` suffix (Empire 0, Rebel 1, Underworld 2). Skins: Empire blue, Rebel red, Underworld amber.

### Fonts and tooltips

`GameConstants`: `Command_Bar_Default_Font_Name` / `Text_Button_Default_Font_Name` =
`EmpireAtWar-Medium` (7 pt), labels `EmpireAtWar-Bold`, tooltips `Arial` 7 pt. **No font files exist in the game
data or the mod** (the names resolve through the engine), so the crate cannot load them. Fonts are
resolved at runtime from the operating system (cargo feature `system-fonts`, default on):
`Bahnschrift` for the bar and `Arial` for tooltips (both ship with Windows), configurable in
`EawUiSettings`. Without the feature or if a family is missing Bevy's built-in font is used.
Nothing is bundled.

## Crate layout

* `textures`: `MegaTextureDirectory` (`.mtd`: `u32 count`, then 81-byte records
  `name[64] x y w h flag`), `TextureLibrary` (mega texture crop, then loose `.dds/.tga`),
  `UiTextures` (resource, cache of Bevy images).
* `components`: `CommandBarDb`, `CommandBarComponent`.
* `shell`: `ShellLayout` (items and art from a shell `.alo`), `rasterize` (mesh to image).
* `widgets`: `spawn_command_bar`, systems (button state, clicks, tooltip, slots, bars, UI scale).
* `data`: `CommandBarData`, `EawButtonClicked`, `EawButtonState`, `ForcedTooltip`.
* `lib.rs`: `EawUiPlugin`, `EawUiSettings`, `unit_icon`, `EawUiCommandsExt`.

## Public API

```rust
app.add_plugins(EawUiPlugin::new(fs).with_game_data(game));     // fs: Arc<GameFs>
commands.spawn_eaw_command_bar(CommandBarSpec { mode: CommandBarMode::Galactic, faction: UiFaction::Rebel });
// drive it
*data = CommandBarData { credits, planet_name, queue, pads, selection, .. };   // ResMut<CommandBarData>
// listen
fn on_click(mut ev: MessageReader<EawButtonClicked>) { for e in ev.read() { /* e.name == "b_zoom" */ } }
```

`EawButtonState` (component of each button) lets the application set selected/disabled looks.
The UI camera must be set in `EawUiSettings::camera` when rendering to an image.

## Known gaps

* Not built: planet screen (`i_planetary_mode`), fleet organizer, reinforcement/hero frames, black
  market and corruption dialogs, minimap content (`radar_galactic` is only the frame), movie
  windows, `Text_Day_Credits`, `Text_Galactic_Help`, drag and drop, Click_Shift semantics beyond
  reporting `shift`, SFX playback, ability recharge dials (`Build_Texture_Name` radial sweep is
  approximated by a rising tint), mouse-over blink/flash animations, the idle `.ALA` animation of the shells.
* The ornament dome row visible on the dashboard sheets is not part of the faceplate mesh (it is
  covered by the queue hologram textures), so none is drawn.
* Original fonts are unavailable (see above); text metrics differ slightly.
* `Scale`/`Icon_Offset` semantics were inferred (texture size x scale, centred in the bone's
  rectangle); a few buttons may be a pixel or two off compared to the game.
* The tactical selection slots draw one centred unit icon; the original also uses
  `Overlay`/`Icon_Offset` boxes for sub-icons.
* The first spawn decodes the 64 MB mega texture on the calling thread (about 4 s in a dev build).
