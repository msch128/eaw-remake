---
name: eaw-reverse-engineer
description: Method for decoding undocumented parts of the closed-source Alamo engine (unknown chunks, mini-chunk ids, particle/emitter fields, shader behaviour, engine rules not visible in XML) by corpus statistics over all Remake files instead of guessing from one sample. Use when a field/chunk meaning is unknown or marked Inferred/guess, when output looks wrong versus the original game and the cause may be a misread format, or when adding support for a new Alamo file type.
---

# Reverse engineering Alamo data

The engine is closed source. There is no HLSL source for the shaders and no format spec. What exists:
the full Remake corpus (4,348 models, 7,685 animations, 1,505 particle systems, every `.ted`, all XML/Lua),
the original game to compare against, and names (bones, effects, XML tags) that hint at meaning.

## Workflow

1. **Survey the whole corpus, not one file.** Write or extend a `parse_all*`-style example that walks
   every file of the type through the VFS (`fs.list("Art/Models")` etc.) and gathers per-field statistics:
   value range, distinct values, size distribution, co-occurrence with other fields, which file names
   use which value. Existing ones: `alamo-model` examples `parse_all`, `parse_all_ala`,
   `parse_all_particles`, `particle_summary`, `chunk_tree`; `alamo-map` `ted_dump --tree`.
2. **Form hypotheses from structure.** Common Alamo patterns: chunk/mini-chunk nesting, u32 counts in
   front of arrays, CRC-32 of upper-cased names as ids, floats that look like 0..1 or degrees, u16
   quantized values with a float `base`/`step` nearby, UTF-16 strings.
3. **Correlate with names.** Effect, bone and object names tell you a lot (`light_*` emitters ->
   light blend, `HP_*` bones -> hardpoints, `FP_*` -> fire points, `_ALTn` -> LOD).
4. **Test the hypothesis on the whole corpus.** It must hold for 100% of files, or the exceptions must be
   explained. Keep the `parse_all*` error count at 0.
5. **Check visually or behaviourally** against the original (see `eaw-verify`): render the case where the
   hypothesis matters most and compare it with the original game or with known numbers.

## Recording results (repo conventions)
- Never throw away undecoded bytes. Keep them in `raw` / `props` / `unknown` fields, so later work does
  not need to re-parse.
- Doc comments mark certainty explicitly: proven facts plainly, inferred ones with **"Inferred"** or
  **(guess)** plus the evidence ("all 212 `pe_` files have value 3").
- Engine rules not visible in the data go into `BattleConfig` / `SimConfig` / `defs.rs` as named,
  documented **estimates**, and into the crate's `RULES.md` with their source. Never bury them as magic numbers.
- Update the crate `STATUS.md` "Gaps" list when something is decoded or newly found to be unknown.
- Module docs carry the format layout (see `alamo-map/src/lib.rs`, `alamo-model/src/particle.rs` as templates).

## Pitfalls seen in this repo
- Axis conventions differ per format (models Z-up and bow -Y, particles Y-up, `.ted` yaw CCW from +X).
  See `eaw-formats`.
- A mirrored or upside-down result is usually a convention issue, not a parser bug.
- Per-frame vs per-second units (x30) in XML speeds.
- Values that look like one thing in one file but differ elsewhere: always count across the corpus.
- The VFS precedence for patch MEGs is a guess. If a file's content surprises you, check which archive
  it came from (`megtool vfs-cat`).
