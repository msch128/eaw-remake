# Battle regressions reported on 5df5dd2 (2026-10-15)

Method: diffs 7d939e7 (yesterday 20:54, reference) / edec2ea (turret commit) .. HEAD, offscreen runs of `battle_demo`, `tactical_roundtrip --ui` and the real app (`--battle-orders`) built from clean git worktrees (no checkout in the shared tree), with `SP_TURRET_LOG=1` and a temporary per-frame overlay log. Windowed runs could not reach a battle from the CLI (`--stop-on-battle` autoplay is offscreen only), so visual claims below say "not verified in a window" where that applies.

| Feature | Last good | Breaking commit | Cause / status |
|---|---|---|---|
| Turrets turn toward their target | never fully | none (code unchanged since edec2ea) | `TurretAim::from_hardpoint` limited yaw to extent / 2 while the sim fires within +-extent (`battle-sim` defs.rs arc_yaw). ISD HICT: extent 45 -> stop at 22.5 deg = 0.39 rad, speed 0.25 rad/s (data). Yaw sat at 0.39 for 35 s while the turret kept firing, so the barrel looked frozen. Fixed: yaw limit +-extent (alamo-render animation.rs, test `turret_yaw_limit_is_the_extent_to_each_side`). |
| Giant flat light-blue sphere after a retreat, never leaves | never (since the departure flash exists) | e393fb4 fixed the size; the lifetime was the other half | Departure flash and departing ship are timed on the sim clock only. At DEFEATED (terminal snapshot) or pause the clock stops, so they never despawn. Fixed: `RemoveAfter(3 s)` wall time on both (ships.rs, test `remove_after_despawns_on_wall_time_when_the_sim_clock_stands`). The 234 "Entity despawned" warnings in the user log are at battle start (galaxy teardown), not at the retreat. |
| Hover enemy -> hardpoint reticles, attack-order view, red right-click | same code | none found | `control.rs` (hover, hardpoint pick, right-click orders, red order ring and hardpoint markers) is unchanged since 7d939e7; `sync_overlays` changed only by the f40cb4a icon rule. What changed around it: the fog shroud (enemy ships in unseen areas have `visible == false`, so they are neither hoverable nor attackable), icon boxes (f40cb4a), camera start (278de4a). Not verified in a window. "Red right-click dialog": no UI with that description exists in the code history (red order ring on right-click, red hover ring, red no-spawn discs, red hardpoint markers do exist); needs the user's exact description. |
| Health / shield bars flicker while a ship moves | unknown | unknown | Overlay data is smooth (0.11 px per frame, no mode flips). Likely sub-pixel placement of thin textured bars. Fix: snap centre and size to whole physical pixels in `sync_overlays` (patch, not applied: tactical_ui.rs is being edited by another agent). Not verified in a window. |
| Shield bubble on ships without shields | - | aa70c94 (bubble) | Normal hits cannot light it (sim emits Shield hits only with shield > 0, `hide_downed_shields` hides it at 0). The special-weapon impact flash fired `ShieldHit` on any ship. Fixed in effects.rs (both triggers need shield > 0 in the snapshot and the data). |
| Purifier shows no hardpoint markers | - | not a regression | Data: all 176 weapon hardpoints of the Purifier (`Hardpoints_Empire_Star_Destroyer_Annihilator.xml`) and its shield generator have `Is_Targetable No` and `Is_Destroyable No`, so the marker rule (targetable and destroyable) correctly shows none. The Lucrehulk has destroyable ones, which is why it shows crosshairs. |

## Sweep (what was checked)

| Feature | Result |
|---|---|
| Hardpoint targeting / focus fire (U53) | code unchanged since 07d7e25 (control.rs); `AttackHardpoint` path intact. Not verified in a window. |
| Selection, box select, control groups | control.rs unchanged since 7d939e7; `--battle-orders` run selects, orders and attacks. Works offscreen. |
| Hover info cards | tactical_ui overlay name logic intact (f40cb4a changed only the icon rule). Not verified in a window. |
| Turret models and aiming | models spawn (`with_unit`), 173 turret models in the demo, 36-45 of ~600 targeted hardpoints are turrets and track. Works (limit bug above). |
| Shield bubble | works with shields; see above. |
| Retreat | flash and departure fixed above; sim retreat unchanged. |
| Reinforcement, abilities | not touched by any of these diffs except ability order (126bca4); not verified offscreen here. |
