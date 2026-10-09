# alamo-model status

Parses Alamo engine assets into engine-neutral data. Depends only on `alamo-fs`.

## What works
- `.alo` models (`Model::parse`): skeleton, meshes (LOD from `_ALTn`, hidden/collision flags, bbox),
  sub-meshes with material (shader + typed params), all vertex formats (144/128 byte), bone
  palettes, connections, proxies, lights. 4,348/4,348 Remake models parse, 0 errors.
- `.ala` animations (`Animation::parse`): both encodings (per-bone keys, pooled channels),
  dequantized translation/rotation/scale tracks + visibility bits. 7,685/7,685 parse.
- Particle systems (`ParticleSystem::parse`, `.alo` with root 0x900): emitters, shapes, 7 tracks
  (R,G,B,A,size,frame,spin), blend mode, flags. 1,505/1,505 parse. Format is reverse engineered;
  inferred fields are marked "Inferred" in doc comments. `light_*`/type-4 emitters are `BlendMode::Light`.
- Particle property groups (chunk 0x29, three 0x1101 blocks) are, in file order, **initial speed, lifetime, position** (`Emitter::speed_group`, `lifetime_group`, `position_group`), per the public format doc https://modtools.petrolution.net/docs/ParticleFileFormat (Oct 2026). Earlier versions had speed/position swapped and used the lifetime group as a velocity. The doc also names props 0x12 = `randomizedScale` and 0x13 = `randomizedLifetime` (verified: lifetime group min.y = lifetime*(1-0x13) in the data). Shape kinds: 0 exact (`params[6..9]`), 3 sphere (`params[1]` radius, `params[2]` surface), 4 cylinder (`params[3]` radius, `params[4]` surface, `params[5]` height); in the data kind 1 is a min..max box and kind 2 a cube of side `params[0]` (the doc has 1/2 swapped). The doc's property id table is decimal (0x0b outwardAcceleration, 0x0c gravity); our `gravity` still reads 0x0b and `speed` 0x09, unverified.
- Textures: `image::decode_texture` (DDS DXT1/3/5 + uncompressed, TGA, PNG via own inflate),
  `encode_png`.

## Conventions
Alamo space is right-handed, Z-up, bows point -Y, UV origin top-left, CCW front faces.
Bone transforms are row-major 3x4, local to the parent. Rigid mesh vertices are bone-local,
skinned ones are model-space. Particle emitter space is Y-up (maps to bone +Z on proxies).
Convert to Bevy with `(x,y,z) -> (x,z,-y)`.

## Examples (`cargo run -p alamo-model --example <name> -- ...`)
`alo_dump <path>`, `chunk_tree <path>`, `alo_to_gltf <path> <out.glb>`, `parse_all`,
`parse_all_ala`, `parse_all_particles`, `particle_summary <path>`, `particle_dump`,
`xml_models <Tag>`. Paths are game paths (e.g. `Art/Models/X.ALO`), mod 1770851727 by default.

## Gaps / next steps
- Chunks 0x1200 (collision tree) and 0x1300 extras ignored; billboard mode 7 and light fields partly guessed.
- Particle semantics partly inferred (spawn rate/burst rules, shape block 3, tracks modes, flags 0x08/0x41/0x43).
- No glTF animation/skin export; no `.ted`/map parsing.
