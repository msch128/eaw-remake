---
name: eaw-formats
description: Reference for the Petroglyph Alamo engine file formats used by Empire at War / Forces of Corruption and how this repo reads them - MEG archives, the layered VFS, chunk files, .alo models, .ala animations, particle .alo (0x900), .ted maps, .dat string tables, lenient XML - plus coordinate conventions (Z-up, bow -Y, Bevy conversion). Use when touching alamo-fs, alamo-model, alamo-map, alamo-data parsing, alamo-render loading, or whenever a model is rotated/mirrored wrong, a file is "not found", or a chunk/field is unknown.
---

# Alamo file formats in this repo

All readers are read-only and never modify game files. Paths are relative to a `Data` root,
case-insensitive, `/` or `\` allowed (`Art/Models/EV_Star_Destroyer.ALO`, `Xml/Planets.xml`).

## Where each format lives

| Format | Crate / file | Inspect with |
|---|---|---|
| `.meg` archives (v1 EaW/FoC; v2/v3 unencrypted untested) | `alamo-fs/src/meg.rs` | `cargo run -p alamo-fs --example megtool -- list <meg>` |
| Layered VFS (mod -> FoC -> EaW) | `alamo-fs/src/vfs.rs` (`GameFs::for_mod`) | `megtool vfs-list 1770851727 Art/Models`, `vfs-cat`, `vfs-extract` (`-` = unmodded FoC) |
| Install detection | `alamo-fs/src/install.rs` (`GameInstall::detect`, env `EAW_GAME_DIR`, Steam registry + `libraryfolders.vdf`) | |
| Generic chunks | `alamo-model/src/chunk.rs`, `alamo-map/src/chunk.rs` | `cargo run -p alamo-model --example chunk_tree -- <path>` |
| `.alo` models | `alamo-model/src/alo.rs` (`Model::parse`) | `alo_dump <path>`, `alo_to_gltf <path> <out.glb>`, `parse_all` |
| `.ala` animations | `alamo-model/src/ala.rs` (`Animation::parse`) | `ala_probe`, `parse_all_ala` |
| Particle systems (`.alo`, root `0x900`) | `alamo-model/src/particle.rs` | `particle_dump`, `particle_summary <path>`, `parse_all_particles` |
| Textures DDS (DXT1/3/5, uncompressed), TGA, PNG | `alamo-model/src/image.rs`, `png.rs` | `alamo-render --example tex_dump` |
| `.ted` maps (space) | `alamo-map/src/lib.rs` | `cargo run -p alamo-map --example ted_dump -- [--tree] [--mod 1770851727] Art/Maps/Name.ted` |
| `.dat` string tables | `alamo-data/src/text.rs` (English only) | |
| XML data | `alamo-data/src/xml.rs` (lenient, never fails) | `alamo-render --example xml_grep <substring> [after] [max]` |

Model examples take game paths and default to mod 1770851727.

## Format essentials

- **Chunk:** `u32 id, u32 size`; bit 31 of size = container (payload is child chunks), low 31 bits = length.
  Leaf chunks often hold **mini-chunks** `u8 id, u8 size, data`. Little-endian everywhere.
- **MEG v1:** `u32 num_filenames, u32 num_files`, filename table `(u16 len, bytes)`, then records of five u32
  `crc32, index, size, start, filename_index`. Entries are read lazily with positional reads.
- **VFS precedence (highest wins):** a stack's loose files first, then MEGs from the *highest* root's
  `Data/megafiles.xml` (it replaces lower lists), then unlisted `Patch.meg`/`Patch2.meg`/`64Patch.meg`
  (an educated guess), then the SFX MEGs (`sfx2d_english`, `sfx2d_non_localized`, `sfx3d_non_localized`)
  mounted from every root. Stacks: `[[mod, corruption/Data], [GameData/Data]]`. Non-English speech is skipped.
- **`.dat`:** `count`, then `count x (crc, value_len, key_len)`, then values UTF-16LE, then keys ASCII.
- **XML:** tags and attribute names are lower-cased while parsing, so look them up in lower case. Values keep
  their case. Mods rely on broken XML (`--` in comments, stray `&`, mismatched case, garbage at the end):
  report a `Warning`, never fail.
- **`.ted`:** chunk stream. Object types are the **CRC-32 of the upper-cased XML object name**, so call
  `SpaceMap::resolve_names`. Rotations are Euler degrees. Yaw is CCW from +X; attackers sit at -Y facing +Y.
- **`.ala`:** two encodings (per-bone key chunks `0x1002/0x1004/0x1006/0x1007/0x1008` vs pooled channels
  `0x1009/0x100a/0x100b`). Bones match the skeleton **by name**, case-insensitive. Quaternion = i16/32767.
  Translation/scale = `base + q*step`. Lookup name `<MODEL>_<KIND>_<NN>.ALA`.
- **Particles:** 7 tracks (R,G,B,A,size,frame,spin), emitter space is **Z-up** like the models (on a proxy: the bone's own axes;
  directed emitters point along +Z). Fields documented as
  "Inferred" are guesses based on value statistics.

## Coordinate conventions (most frequent source of bugs)

- Alamo/3ds Max space: **right-handed, Z-up, ship bow points -Y**. UV origin is top-left (D3D). Front faces are CCW.
- Bone transforms: row-major 3x4 `[R|t]`, column-vector convention, local to the parent.
- Rigid mesh vertices are bone-local (via `Connection`); skinned vertices are model space (via `bone_palette`).
- **To Bevy / glTF (Y-up):** `(x, y, z) -> (x, z, -y)`.
- **To battle-sim space (X forward, Y left, Z up):** `(x, y, z) -> (-y, x, z)`. Verified with the ISD bow turbolaser.
- Particle emitter `+Y` = proxy bone `+Z`.
- battle-view: `convert::SIM_TO_MAP_YAW` is 0 with `BattleConfig::arena`. Without an arena it must be PI/2.

## Known undecoded / guessed parts
Collision tree `0x1200`, `0x1300` extras, billboard mode 7, light fields, several particle flags and
shape block 3, many `.ted` mini-chunks (kept as `raw`/`unknown`). Before "fixing" behaviour that depends on
them, follow the `eaw-reverse-engineer` skill.
