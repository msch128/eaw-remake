# File formats

What the project knows about the Petroglyph Alamo engine's file formats, and how each one is read. The engine is closed
source and there is no public specification, so everything here was read from the shipped files. Statements are marked
**proven** (holds for every file of the Remake corpus) or **inferred** (a guess from value statistics and names). The
same marks appear in the doc comments of the parsers (`alamo-fs`, `alamo-model`, `alamo-map`, `alamo-data`).

The reference corpus is the Remake 4.0 mod plus the base game: 4,348 models, 7,685 animations, 1,505 particle systems,
every `.ted` map, all XML and 709 Lua scripts. The `parse_all*` examples must keep parsing all of it with 0 errors.

## Overview

| Format | What it holds | Reader | Inspect with |
|---|---|---|---|
| `.meg` | archive of game files | `alamo-fs/src/meg.rs` | `cargo run -p alamo-fs --example megtool -- list <meg>` |
| `megafiles.xml` | which archives a root mounts, in order | `alamo-fs/src/vfs.rs` | `megtool vfs-list 1770851727 Art/Models` |
| chunk stream | container used by `.alo`, `.ala`, `.ted` | `alamo-model/src/chunk.rs`, `alamo-map/src/chunk.rs` | `cargo run -p alamo-model --example chunk_tree -- <path>` |
| `.alo` | 3D model, skeleton, hardpoints, or a particle system | `alamo-model/src/alo.rs`, `particle.rs` | `alo_dump <path>`, `alo_to_gltf <path> <out.glb>`, `parse_all` |
| `.ala` | skeletal animation | `alamo-model/src/ala.rs` | `ala_probe`, `parse_all_ala` |
| `.dds` `.tga` `.png` | textures | `alamo-model/src/image.rs`, `png.rs` | `cargo run -p alamo-render --example tex_dump` |
| `.mtd` | directory of the packed UI texture | `eaw-ui/src/textures.rs` | |
| `.ted` | tactical (space) map | `alamo-map/src/lib.rs` | `cargo run -p alamo-map --example ted_dump -- [--tree] [--mod 1770851727] Art/Maps/Name.ted` |
| `.dat` | string table (UTF-16) | `alamo-data/src/text.rs` | |
| XML | all gameplay data | `alamo-data/src/xml.rs` | `cargo run -p alamo-render --example xml_grep -- <substring> [after] [max]` |
| Lua | story plots, AI plans, object scripts | `alamo-lua` | `cargo run -p alamo-lua --example lua_survey` |
| WAV, MP3 | sound effects, speech, music | `alamo-audio/src/wav.rs` | `cargo run -p alamo-audio --example sfx_dump -- <event>` |

Model and map examples take game paths and default to the Remake mod (1770851727). Paths are relative to a `Data`
root, case-insensitive, with `/` or `\` allowed (`Art/Models/EV_Star_Destroyer.ALO`, `Xml/Planets.xml`). All readers are
read-only and never modify game files.

## Chunks and mini-chunks

Little-endian everywhere. A chunk is `u32 id, u32 size`. Bit 31 of `size` marks a container (the payload is child
chunks); the low 31 bits are the payload length. Leaf chunks often hold mini-chunks: `u8 id, u8 size, data`. Chunk ids
are meaningful only inside their parent. **Proven** for `.alo`, `.ala` and `.ted`.

## MEG archives and the virtual file system

* **MEG v1** (Empire at War, Forces of Corruption): `u32 num_filenames, u32 num_files`, a filename table of
  `(u16 len, bytes)`, then `num_files` records of five `u32`: `crc32, index, size, start, filename_index`. Entries are
  read lazily with positional reads, so an archive can be shared between threads. **Proven.**
* **MEG v2 / v3, unencrypted** (later Petroglyph games): `u32 0xFFFFFFFF, u32 0x3F7D70A4, u32 header_size, u32
  num_filenames, u32 num_files, u32 names_size`, the same name table, then 20-byte records `u16 flags, u32 crc32, u32
  index, u32 size, u32 start, u16 filename_index`. Untested against real files; encrypted v3 is rejected.
* **Layering** (`GameFs::for_mod`). Stacks, highest priority first, are `[mod, Forces of Corruption, base game]`. Inside
  the lookup, the highest wins: a stack's loose files, then the MEGs listed in the highest root's `Data/megafiles.xml`
  (it replaces the lower lists), then the unlisted `Patch.meg`, `Patch2.meg`, `64Patch.meg` (**inferred** order), then
  the sound archives (`sfx2d_english`, `sfx2d_non_localized`, `sfx3d_non_localized`) mounted from every root. Non-English
  speech is skipped. If a file's content surprises you, check which archive it came from with `megtool vfs-cat`.
* **Paths** are normalised: upper case, `/`, no `Data/` prefix.
* **Install detection** (`GameInstall::detect`): env `EAW_GAME_DIR` / `EAW_MOD_DIR`, the Steam path in the registry
  (`HKCU\Software\Valve\Steam\SteamPath`), then every library in `libraryfolders.vdf` for app `32470`; the mod lives in
  `steamapps/workshop/content/32470/1770851727`.

## Models (`.alo`)

An `.alo` without root chunk `0x900` is a model (with root `0x900` it is a particle system, see below). Output is
exactly as stored.

* **Space.** Right-handed, **Z-up**, ship bow along **-Y** (the space of 3ds Max, where the assets are authored). UV
  origin is top-left (Direct3D, `v` grows downwards). Front faces are counter-clockwise. Bone transforms are row-major
  3x4 `[R|t]` in the column-vector convention (`p' = R*p + t`), local to the parent.
* **Meshes.** A rigid mesh is attached to a bone through a `Connection`, its vertices are bone-local. A skinned mesh has
  per-vertex bone indices that go through the sub-mesh's `bone_palette`, its vertices are model space. Each sub-mesh
  names a shader and its parameters (texture names, colours); the Alamo shaders are not available as source, so the
  renderer reimplements them (`alamo-render/src/alamo.wgsl`).
* **Bones with meaning.** `HP_*` proxies are hardpoints (weapon mounts and engines), `FP_*` are fire points, `_ALTn`
  suffixes select levels of detail, shield and particle proxies carry their own axes. Texture names inside a model
  resolve against `Art/Textures`.
* **Conversions.** To Bevy or glTF (Y-up): `(x, y, z) -> (x, z, -y)`. To battle-sim space (X forward, Y left, Z up):
  `(x, y, z) -> (-y, x, z)`, verified with the Star Destroyer's bow turbolaser.
* **Not decoded (kept raw or ignored):** the collision tree (`0x1200`), `0x1300` extras, billboard mode 7, several light
  fields. Before changing behaviour that depends on them, follow "Reverse engineering method" below.

## Animations (`.ala`)

An animation has a frame count, a frame rate and, per bone, translation, rotation and scale tracks plus an optional
visibility track. Bones are matched to the skeleton **by name**, case-insensitive; the index stored in the file counts
animated bones only. Two encodings decode to the same result (**proven**):

* *per-bone*: each bone chunk (`0x1002`) carries key chunks `0x1004` (translation, 3 x u16 per frame), `0x1006`
  (rotation, 4 x i16 per frame) and `0x1007` / `0x1008` (visibility bit fields). A key chunk with a single key means a
  constant value.
* *pooled*: the bone references channels (`0x0e` translation, `0x10` rotation, `0x0f` scale mini-chunks) of the shared
  pools `0x100a` / `0x1009` / `0x100b`, laid out `[frame][channel]` as u16. The header gives the channel counts (`0x0b`
  rotation, `0x0c` translation, `0x0d` scale). Channel index `0xffff` means constant.

Dequantisation: translation and scale are `base + q * step` (u16 `q`), rotation is a quaternion `i16 / 32767`. The
transforms are local and replace the bone's transform from the skeleton. The animation for a model is found by name:
`<MODEL>_<KIND>_<NN>.ALA`.

## Particle systems (`.alo`, root chunk `0x900`)

Reverse engineered from the 1,505 particle files of the Remake; fields marked inferred are guesses from value
statistics and effect names, and unrecognised emitter properties stay raw in `Emitter::props`.

```text
0x900 system
  0x0   name (C string)
  0x1   system flags (mini-chunks without payload)
  0x800 emitter list
    0x700 emitter (one per emitter)
      0x2   properties (mini-chunks: u8 id, u8 size, payload)
      0x3   texture path   0x45 second texture path   0x16 emitter name
      0x29  three 0x1100 containers, each with a 0x1101 shape block (position, velocity, extra)
      0x1   seven tracks, each a 0x0 header (start, end, mode) followed by a 0x1 key list
      0x36  links to other emitters (mini-chunks 0x37, 0x39; 0xffffffff = none)
```

* Tracks, in order: red, green, blue, alpha (bytes), size, atlas frame index (start and end frame; flipbook when they
  differ) and rotation speed. A track runs from `start` at birth to `end` at the end of the particle's life; keys are
  `(value, time)` knots with time as a fraction of the life.
* Positions and velocities are in the emitter's local space, **Z-up** like the models. On a model proxy the axes are the
  proxy bone's own axes; directed emitters point along `+Z` (803 exact speeds along `+Z`, 53 along `+Y`); engine
  exhausts leave the stern along the bone's `+Z` (1139 of 1157 engine proxies).
* Inferred or undecoded: spawn-rate and burst rules, shape block 3, track modes, flags `0x08`, `0x41`, `0x43`, the
  `isHeatParticle`, `hasTail` and `noDepthTest` flags, property ids `0x09` (speed) and `0x0b` (gravity), emitter types
  other than additive.

## Textures

DDS (DXT1/3/5 and uncompressed; kept block-compressed on the GPU as BC1/2/3), TGA (including the black-hole type 78,
which does not decode) and PNG, with mip chains. Small UI textures are packed in `Art/Textures/MT_COMMANDBAR.TGA`
(4096 x 4096) with a directory in `MT_COMMANDBAR.MTD`: `u32 count`, then 81-byte records `name[64] x y w h flag`.

## Maps (`.ted`)

A chunk stream with mini-chunk leaves. Space maps are decoded; land maps are not.

```text
0x00  header (mini): version, names (UTF-16), map size
0x13  preview image (opaque, kept raw)
0x02  list of 0x0b "defender slot" records
0x01  map body
  0x100  environment
    0x04/0x06  lighting, ambient, sky, nebula, ... (mini)
    0x02/0x00  fog (mini)
  0x102/0x01  objects: a list of 0x44c entries
    0x44c/0x454  two u32 ids
    0x44c/0x459/0x4b0  object properties (mini): id, type crc, position, rotation, ...
    0x44c/0x459/0x4b7  optional attachment
    0x44c/0x459/0x4ba  77 constant "NONE" slots (ignored)
  0x103  bounds (mini)
  0x105  camera / misc (mini, kept raw)
```

* Object types are the **CRC-32 of the upper-cased XML object name** (`ASTEROID_LARGE`). Call
  `SpaceMap::resolve_names` with the game objects to turn the hashes into names.
* Z is up, the map plane is XY (bounds roughly +-9500 in X/Y, +-5000 in Z), in model units. Rotations are Euler angles in
  degrees; markers use only yaw, asteroids tumble around all axes (Euler order unverified). Yaw is counter-clockwise from
  +X: attackers sit at -Y with yaw about 83 to 90 (facing +Y), defenders at +Y with yaw 270; the facing vector is
  `(cos yaw, sin yaw, 0)`.
* Undecoded data stays available through `Minis` (`raw` fields) or `SpaceMap::unknown`.

## String tables (`.dat`)

`MasterTextFile_ENGLISH.DAT` and its siblings: all integers are little-endian `u32`. `count`, then `count` entries of
`(crc, value_len, key_len)`, then all values as UTF-16LE (`value_len` code units each), then all keys as ASCII. Only the
English table is read.

## XML

All gameplay data. Tags and attribute names are lower-cased while parsing, so look them up in lower case; values keep
their case. The mod relies on broken XML (`--` inside comments, stray `&`, mismatched case, garbage at the end), so the
reader reports a `Warning` and never fails.

* `Variant_Of_Existing_Type` inheritance applies only through `resolve()`. Reading raw fields of a variant without it is
  a classic bug.
* Unknown tags stay in `fields` maps; typed views (`space.rs`, `combat.rs`) cover ships, squadrons, hardpoints,
  projectiles, abilities, structures and the damage matrix.
* Units: `Max_Speed` (ships and projectiles) is per frame and multiplied by 30; `*_Seconds` fields are real seconds;
  `Max_Rate_Of_Turn` is used as radians per second. See [RULES.md](RULES.md).
* Search all XML (mod over Forces of Corruption over base game) with `xml_grep`; resolve one object with
  `cargo run -p alamo-data --example object_dump -- 1770851727 Star_Destroyer` (`raw:` shows every tag, `?` searches by
  name).

## Lua

709 scripts (Remake mod plus base game) drive story plots, the galactic and tactical AI, object behaviour and
libraries. The engine embeds Lua 5.0; the scripts use only features Lua 5.1 still accepts, and `alamo-lua` embeds a
vendored Lua 5.1. A script file is loaded, `Base_Definitions()` runs (calling `Definitions()`), then `main()` runs as
a thread; `Story_Event_Trigger(name)` fires on plot events; `PumpEvents()` yields. Scripts call 446 distinct engine
functions. `lua_survey` regenerates the inventory; the generated list is not stored in the repository. No script text is
stored either: scripts are read from the game at run time.

## Audio

Sound effects and speech are WAV (PCM and ADPCM) and MP3 in the sound archives; events, sample lists, probabilities,
instance limits and music playlists are XML (`Xml/Audio.xml`, `Xml/SFXEventFiles.xml` and the files it lists, `Xml/MusicEvents.xml`). `alamo-audio` decodes
on demand and resolves events to samples. See [ARCHITECTURE.md](ARCHITECTURE.md).

## Reverse engineering method

The engine is closed source: there is no HLSL for the shaders and no format spec. What exists is the full Remake corpus,
the original game to compare against, and names (bones, effects, XML tags) that hint at meaning.

1. **Survey the whole corpus, not one file.** Write or extend a `parse_all*` style example that walks every file of the
   type through the VFS and gathers per-field statistics: value range, distinct values, size distribution,
   co-occurrence with other fields, which file names use which value.
2. **Form hypotheses from structure.** Common Alamo patterns: chunk and mini-chunk nesting, `u32` counts in front of
   arrays, CRC-32 of upper-cased names as ids, floats that look like 0..1 or degrees, `u16` quantised values with a
   float `base` / `step` nearby, UTF-16 strings.
3. **Correlate with names.** `light_*` emitters mean light blend, `HP_*` bones are hardpoints, `FP_*` fire points,
   `_ALTn` levels of detail.
4. **Test on the whole corpus.** A hypothesis must hold for 100% of files or the exceptions must be explained. Keep the
   `parse_all*` error count at 0.
5. **Check visually or behaviourally** against the original game (offscreen render, a known number).

Recording results: never throw away undecoded bytes (keep them in `raw`, `props` or `unknown` fields); mark certainty in
doc comments (proven facts plainly, inferred ones with **"Inferred"** or **(guess)** and the evidence, such as "all 212
`pe_` files have value 3"); engine rules not visible in the data become named, documented estimates in `BattleConfig`,
`SimConfig` or `defs.rs` and an entry in [RULES.md](RULES.md); module docs carry the format layout.

Pitfalls seen here: axis conventions differ per format (models and particles Z-up with the bow at -Y, `.ted` yaw
counter-clockwise from +X); a mirrored or upside-down result is usually a convention issue, not a parser bug;
per-frame versus per-second units (x30) in XML speeds; a value that looks like one thing in one file can differ elsewhere,
so count across the corpus; the patch-archive precedence is a guess.
