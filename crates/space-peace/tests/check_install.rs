//! `eaw-remake --check-install` through the real binary: the exit codes the Windows installer reads (0 both found,
//! 2 game missing, 3 mod missing). The folders are scratch folders of our own with a placeholder file in them, no game
//! files; the process never opens a window.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A scratch Steam library below the temp folder, removed again when it goes out of scope.
struct Library(PathBuf);

impl Library {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("eaw-check-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    /// `steamapps/common/Star Wars Empire at War`.
    fn game_root(&self) -> PathBuf {
        self.0.join("steamapps").join("common").join("Star Wars Empire at War")
    }

    /// `steamapps/workshop/content/32470/1770851727/Data`, where Steam puts the Remake mod.
    fn mod_data(&self) -> PathBuf {
        self.0.join("steamapps").join("workshop").join("content").join("32470").join("1770851727").join("Data")
    }

    fn fill(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("placeholder.txt"), "x").unwrap();
    }

    /// Forces of Corruption data in the game folder.
    fn install_game(&self) {
        Self::fill(&self.game_root().join("corruption").join("Data"));
    }

    fn install_mod(&self) {
        Self::fill(&self.mod_data());
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs `eaw-remake --check-install <args>` with the search variables cleared, so only the arguments decide.
fn check_install(args: &[&OsStr], env: &[(&str, &Path)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_eaw-remake"));
    cmd.arg("--check-install").args(args).env_remove("EAW_GAME_DIR").env_remove("EAW_MOD_DIR");
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().expect("run eaw-remake --check-install")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn an_explicit_game_dir_that_is_empty_exits_with_2() {
    let lib = Library::new("empty");
    let empty = lib.0.join("empty-folder");
    fs::create_dir_all(&empty).unwrap();
    let out = check_install(&["--game-dir".as_ref(), empty.as_os_str()], &[]);
    assert_eq!(out.status.code(), Some(2), "{}", stdout(&out));
    assert!(stdout(&out).contains("game: NOT FOUND"), "{}", stdout(&out));
}

#[test]
fn a_game_dir_that_does_not_exist_exits_with_2() {
    let lib = Library::new("absent");
    let out = check_install(&["--game-dir".as_ref(), lib.0.join("no-such-folder").as_os_str()], &[]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn a_game_without_the_mod_exits_with_3() {
    let lib = Library::new("no-mod");
    lib.install_game();
    let out = check_install(&["--game-dir".as_ref(), lib.game_root().as_os_str()], &[]);
    assert_eq!(out.status.code(), Some(3), "{}", stdout(&out));
    assert!(stdout(&out).contains("mod 1770851727: NOT FOUND"), "{}", stdout(&out));
}

#[test]
fn the_game_and_the_mod_in_the_workshop_folder_exit_with_0_and_are_listed() {
    let lib = Library::new("both");
    lib.install_game();
    lib.install_mod();
    let out = check_install(&["--game-dir".as_ref(), lib.game_root().as_os_str()], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains("Star Wars Empire at War") && text.contains("1770851727"), "{text}");
}

#[test]
fn a_mod_dir_replaces_the_workshop_folder() {
    let lib = Library::new("mod-dir");
    lib.install_game();
    lib.install_mod();
    let nowhere = lib.0.join("no-such-mod");
    let out = check_install(&["--game-dir".as_ref(), lib.game_root().as_os_str(), "--mod-dir".as_ref(), nowhere.as_os_str()], &[]);
    assert_eq!(out.status.code(), Some(3), "the Workshop copy does not count: {}", stdout(&out));
    let mine = lib.0.join("my-mod");
    Library::fill(&mine.join("Data"));
    let out = check_install(&["--game-dir".as_ref(), lib.game_root().as_os_str(), "--mod-dir".as_ref(), mine.as_os_str()], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
}

#[test]
fn the_environment_variables_work_like_the_flags() {
    let lib = Library::new("env");
    lib.install_game();
    let out = check_install(&[], &[("EAW_GAME_DIR", &lib.game_root())]);
    assert_eq!(out.status.code(), Some(3), "no mod yet: {}", stdout(&out));
    let mine = lib.0.join("my-mod");
    Library::fill(&mine);
    let out = check_install(&[], &[("EAW_GAME_DIR", &lib.game_root()), ("EAW_MOD_DIR", &mine)]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    let empty = lib.0.join("empty-folder");
    fs::create_dir_all(&empty).unwrap();
    let out = check_install(&[], &[("EAW_GAME_DIR", &empty)]);
    assert_eq!(out.status.code(), Some(2), "{}", stdout(&out));
}
