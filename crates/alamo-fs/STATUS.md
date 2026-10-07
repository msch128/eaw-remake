# alamo-fs status

Read-only access to Alamo engine files: MEG archives, loose directories and the layered game file system.

## What works
- `MegArchive` (case-insensitive lookup), `LooseDir`, and the `FileSource` trait (`read` / `exists` / `list`).
- `GameFs::for_mod(install, workshop_id)`: mod over Forces of Corruption over base game, `megafiles.xml` lists,
  unlisted patch MEGs, the audio MEGs the engine mounts itself; non-English speech archives are skipped.
- `GameInstall::detect()`: env `EAW_GAME_DIR`, otherwise registry `SteamPath` + `libraryfolders.vdf` + the library holding app 32470.
- Example `megtool` (list / cat / `vfs-cat` / search); `tests/real_install.rs` skips without the game.

## Key decisions
- Paths are normalized (upper case, `/`, no `DATA/` prefix); everything is read-only and `Send + Sync`.
- Precedence rules are documented at the top of `vfs.rs`; the patch-MEG ordering is an educated guess, not verified.

## Gaps / next steps
- Windows/Steam only in practice (a Linux Steam path guess exists but is untested).
- No config file for the game location; override is env `EAW_GAME_DIR` or the app's `--game-dir`.
- Whole files are read into memory; no streaming or caching layer.
