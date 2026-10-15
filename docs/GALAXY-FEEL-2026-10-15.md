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
