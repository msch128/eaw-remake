# alamo-map status

Parser for `.ted` map files, focused on space tactical maps.

## What works
- Chunk and mini-chunk reader (`chunk.rs`) and `SpaceMap` parsing: header/names, bounds, lighting and fog,
  objects (asteroids, markers, spawn points) with position and rotation, defender slots.
- `resolve_names` turns the stored CRC-32 object-type hashes into game object names.
- Example `ted_dump`; `tests/real_maps.rs` parses the shipped space maps and skips without the game.

## Key decisions
- Z-up, same units as `.alo` models; yaw counter-clockwise from +X. Inferred fields are marked "(guess)".
- Undecoded data stays raw (`Minis` raw fields, `SpaceMap::unknown`) instead of being dropped.

## Gaps / next steps
- Land maps are not decoded (out of scope).
- Euler order of asteroid rotations is unverified.
- Preview image and the camera chunk are kept opaque.
