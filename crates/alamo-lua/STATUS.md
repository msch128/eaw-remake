# alamo-lua status

Lua runtime for Empire at War scripts. The engine uses Lua 5.0; this embeds vendored Lua 5.1 (mlua) with 5.0-style compatibility.

## What works
- `ScriptRuntime`: script loader (case-insensitive `require`), cooperative threads (`Create_Thread`, `Sleep`, `PumpEvents`),
  global values, engine API bindings routed through the `ScriptHost` trait.
- Story/plot engine (`story.rs`): plot events, prerequisites, rewards, notifications, `Story_Event`.
- AI scripting API (`ai_api.rs`) used by `galaxy-ai` plans; handle objects for players, planets and units.
- `MockHost` for tests. Example `lua_survey` compiles every script (all of the Remake's) and writes `API.md`, the
  inventory of engine calls scripts make; `run_story` runs a plot.
- Tests: `tests/runtime.rs` (synthetic scripts), `tests/real_scripts.rs` (skips without the game).

## Key decisions
- Unknown engine calls are logged once and return nil; presentation calls (fades, cinematics) are ignored silently.
- Colon and dot call syntax both work on handles.
- No script text is stored in the repo; `API.md` lists only API names, counts and script names.

## Gaps / next steps
- Many gameplay calls are stubs; coverage follows what `API.md` shows scripts use. Ground-combat calls are ignored.
- Cinematic, camera and fog-of-war calls are no-ops.
- Regenerate `API.md` after API changes (`lua_survey -- <workshop_id> --md API.md`).

## Memory (soak finding)
Every plan script runs in its own Lua state. Ended scripts kept theirs (about 350 KB each, one per AI plan: 20 MB per game week, 2.2 GB after 100 weeks) and every bound function registered a new call counter per instance.
Now an ended script drops its Lua state (`Instance::lua` is an `Option`, released in `sweep_finished_plans`, which `update` runs first) and call counters are shared per function name (`Shared::fast_hits` is a map).
Soak: 340 MB flat over 100 weeks.
