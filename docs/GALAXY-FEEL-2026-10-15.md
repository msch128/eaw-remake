# Galaxy map feel vs Remake 4.0 (2026-10-15)

Compared our captures (offscreen 1280x720 at zoom 1.0 / 0.5 / 0.25, plus a real window at 2400x1350) with the Remake frames
CS9 f0007, f0011, f0016, f0019 and n93 f0029. Ranked by how much a player feels the difference. "estimate" marks visual constants
that were picked by eye, not read from the data.

Remake values from `GameConstants.xml` (mod 1770851727): `GMC_InitialPullbackDistance` 470, `GMC_ZoomedPitchAngleDegrees` -20,
`GMC_ZoomTime` 1.0 s, `GMC_ZoomedPullbackPlanetRadiusFraction` 6.0.

| # | Difference | Remake | Ours | Status |
|---|------------|--------|------|--------|
| 1 | Wheel zoom target | zooms toward what is under the cursor; a hovered planet opens its planet view | zoomed toward the galaxy centre, planet view needed a click first | fixed (item 1) |
| 2 | Hyperlanes | long, bright cyan beams, clearly the strongest line on screen | thin, dim, partly red dashes | fixed (item 2): beams wider and about 2x brighter |
| 3 | Star field | sparse tiny stars on pure black | dense bright speckle, reads as TV noise | partly: sky texture 4x sparser and dimmer; the dense speckle in captures still comes from another layer (to find) |
| 4 | Planet labels | small orange text (about 7 px at 1024 wide), few labels at once | about 2x bigger, bold, every planet labelled | fixed (item 3): 11 px close / 8 px far instead of 15 / 10 |
| 5 | Nebula / sector clouds | 3x larger swirl with dots, soft purple | small pink swirl, about a third of the size | fixed (item 4): sector clouds x1.5 |
| 6 | Camera range and tilt | start 470 away, tilt -20 deg when zoomed in | start and range derived from galaxy extent | open |
| 7 | Build bar | dark flat strip, thin red outline | bright maroon strip, bright top edge | open |
| 8 | Maroon core haze | none | visible maroon blob at the galaxy centre | open |
| 9 | Hero portraits top right, pause banner | 9 portraits, GAME PAUSED tab | not in galaxy captures | covered by another agent |

## Item 1: wheel zoom toward the cursor

- `camera.rs`: the wheel keeps the point of the map under the cursor fixed on screen (ray vs galaxy plane, helper
  `cursor_ground_point`); zooming in on a hovered planet also centres it (factor 0.3 per notch, estimate). Without a pointer the
  old behaviour (home in on the selected planet) stays for offscreen runs.
- `planet_view.rs`: a wheel-in over a hovered planet, once the camera is already close (target distance within 4x the closest),
  selects that planet, so the existing dive opens its view.
- Proof: real window, wheel x3 over empty space near Malachor kept the stars around the cursor in place; wheel x8 over Reltooine
  opened the Reltooine planet view. Unit tests in `camera.rs`.

## Fleet drag and drop

Source check: the three Remake videos (CS9VdxglewY 3605 s, n93_uvJ3G6Q 666 s, Bqc5Ei6gF8Y 322 s) were sampled in full (6 s steps
over the galaxy part of CS9, 15-30 s steps elsewhere, 2 fps around planet transitions, extracted with ffmpeg into `%TEMP%\gfeel`).
The player in CS9 keeps the game paused and only moves the camera, clicks planets and opens planet views; no drag of a fleet is
in any video. So the spec below is built from what the frames do show (fleet look, rings, route lines) plus the user's description
of Remake 4.0, and every number is an estimate.

What the frames show about fleets on the galaxy map (CS9 f0016, f0019; t 998-1006 s, t 3565 s, t 3585 s):

1. A parked fleet is a small cluster of ship models with a few tiny red or green icons next to the planet; no big numbers.
2. A selected planet gets a thin notched ring plus the large dashed territory ring; a selected fleet is not marked by anything big.
3. Hyperlanes are thick light shafts; a fleet on a lane is a tiny model with a short line behind it (t 1002-1005 s).
4. The game pause banner and portraits do not change while the player moves things.

Spec (user words: press and hold on a fleet, it hangs at the cursor like a picture dragged between Windows folders, drop it on a planet):

1. Click on a fleet: the whole group pops once (scale up 28 % and back in 0.3 s) and keeps the pulsing double ring. Status: done
   (`galaxy-play/src/carry.rs`, `pop_bump`).
2. Press and move more than 6 px: the group is picked up. It hangs at the cursor, lifted 1.1 pad radii above the map and 20 % bigger,
   following with a short exponential lag (rate 22 per second, frame rate independent). Several selected fleets fan out in a row
   around the cursor. Status: done (`carry_fleets`).
3. The spot it came from stays marked by a dashed ring in the owner colour with a thin dashed tether to the carried fleet. Status: done.
4. Hover target: two pulsing rings around the planet under the cursor; green on an own planet, yellow on a neutral one, red on an enemy
   one, red and still when no route exists. The yellow marching route line and the ETA hint stay. Status: done (`draw_drag`).
5. Drop: the fleet glides from the cursor to where the simulation puts it (its parking pad, or into the lane) in about 0.25 s
   (rate 14 per second) and shrinks back, then moves along the lane. Status: done.
6. A carried fleet is never a drop target for itself (picking skips carried fleets). Status: done.
7. Right click to move stays as it is (no carry animation, route line only).
8. Open: hero heads follow the fleet entity, so they travel with the carried group; checked only in code so far.

Offscreen proof: `--select <planet> --drag-to <planet>` fills the drag state by hand and captures the carried state.
