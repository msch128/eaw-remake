//! Integration tests against a real Steam install; skipped when none is detected.

use alamo_fs::{FileSource, GameFs, GameInstall, MegArchive};

const REMAKE: &str = "1770851727";

fn install() -> Option<GameInstall> {
    let i = GameInstall::detect();
    if i.is_none() {
        eprintln!("skipped: Empire at War not installed");
    }
    i
}

#[test]
fn models_meg_has_entries() {
    let Some(i) = install() else { return };
    let meg = MegArchive::open(i.foc_data.join("Models.meg")).unwrap();
    assert!(meg.len() > 1000);
    let alo = meg
        .entries()
        .iter()
        .find(|e| e.name.ends_with(".ALO"))
        .unwrap();
    let data = meg.read(&alo.name).unwrap();
    assert_eq!(data.len() as u64, alo.size);
}

#[test]
fn unmodded_foc_reads_alo() {
    let Some(i) = install() else { return };
    let fs = GameFs::for_mod(&i, None).unwrap();
    let alo = fs
        .list("Art/Models")
        .into_iter()
        .find(|f| f.ends_with(".ALO"))
        .unwrap();
    assert!(!fs.read(&alo).unwrap().is_empty());
}

#[test]
fn remake_mod_layers() {
    let Some(i) = install() else { return };
    if !i.mod_data(REMAKE).is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return;
    }
    let fs = GameFs::for_mod(&i, Some(REMAKE)).unwrap();
    // Models.meg physically lives only in the base game.
    let alo = fs
        .list("Art/Models")
        .into_iter()
        .find(|f| f.ends_with(".ALO"))
        .unwrap();
    assert!(!fs.read(&alo).unwrap().is_empty());
    // A loose mod XML must win over any archive copy.
    let xml = fs
        .list("Xml")
        .into_iter()
        .next()
        .expect("mod has loose xml");
    let origin = fs.origin(&xml).unwrap();
    assert!(origin.contains(REMAKE) && !origin.contains('!'), "{origin}");
    assert!(fs.exists("Audio/Music") || !fs.list("Audio").is_empty());
}
