//! Every sound hook the Remake data names and the game code plays must resolve to a sample file in the file system.
//!
//! The Remake itself references files that are not in the install (other mods' voice packs such as `Voiceovers\GuP`, blank
//! `Samples`, typos). Those are listed in `known_audio_gaps.txt`; the test fails on any gap not in that list, so a broken
//! hook can not appear unnoticed, and prints gaps that are fixed so the list can shrink. Regenerate the list with
//! `UPDATE_AUDIO_GAPS=1 cargo test -p alamo-audio --test hook_audit`. Skipped when the Remake is not installed.

use std::collections::BTreeSet;

use alamo_audio::hooks::{audit_hooks, is_wired, scan_hooks};
use alamo_audio::{MusicLibrary, SampleResolver, SfxLibrary};
use alamo_fs::{GameFs, GameInstall};

const REMAKE: &str = "1770851727";
const KNOWN: &str = "tests/known_audio_gaps.txt";

#[test]
fn wired_hooks_resolve_to_files() {
    let Some(install) = GameInstall::detect().filter(|i| i.mod_data(REMAKE).is_dir()) else {
        eprintln!("skipped: Remake mod not installed");
        return;
    };
    let fs = GameFs::for_mod(&install, Some(REMAKE)).expect("fs");
    let mut w = Vec::new();
    let sfx = SfxLibrary::load(&fs, &mut w);
    let music = MusicLibrary::load(&fs, &mut w);
    let resolver = SampleResolver::new(&fs);
    let refs = scan_hooks(&fs);
    assert!(refs.len() > 30_000, "only {} hook references found", refs.len());
    let audit = audit_hooks(&refs, &sfx, &music, &resolver, &fs);

    // The hooks that matter most must exist at all.
    for tag in ["sfxevent_select", "sfxevent_move", "fire_sfxevent", "click_sfx", "projectile_sfxevent_detonate_reduced_by_armor"] {
        let t = audit.tags.get(tag).unwrap_or_else(|| panic!("no `{tag}` references found"));
        assert!(t.ok.len() >= 3, "`{tag}`: only {} playable events", t.ok.len());
    }

    let broken: BTreeSet<String> = audit.broken_wired(is_wired).into_iter().collect();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(KNOWN);
    if std::env::var_os("UPDATE_AUDIO_GAPS").is_some() {
        std::fs::write(&path, broken.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
        return;
    }
    let known: BTreeSet<String> = std::fs::read_to_string(&path).unwrap_or_default().lines().map(str::to_string).collect();
    let new: Vec<_> = broken.difference(&known).collect();
    let fixed: Vec<_> = known.difference(&broken).collect();
    if !fixed.is_empty() {
        eprintln!("{} known gaps are fixed, remove them from {KNOWN}: {fixed:?}", fixed.len());
    }
    assert!(new.is_empty(), "{} hooks name sounds that do not play (not in {KNOWN}):\n{}", new.len(), new.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
}

#[test]
fn faction_music_tags_name_existing_events() {
    let Some(install) = GameInstall::detect().filter(|i| i.mod_data(REMAKE).is_dir()) else {
        eprintln!("skipped: Remake mod not installed");
        return;
    };
    let fs = GameFs::for_mod(&install, Some(REMAKE)).expect("fs");
    let mut w = Vec::new();
    let music = MusicLibrary::load(&fs, &mut w);
    let c = alamo_audio::AudioConstants::load(&fs);
    assert_eq!(c.battle_load_music("Rebel"), Some("Tactical_Battle_Loading_Loop_Event_2"));
    assert_eq!(c.result_music("Rebel", Some("Empire"), false, true).first(), Some(&"Lose_To_Empire_Event"));
    for faction in ["Rebel", "Empire", "Underworld"] {
        let load = c.battle_load_music(faction).unwrap_or_else(|| panic!("{faction}: no battle load music"));
        assert!(music.get(load).is_some(), "{faction}: `{load}` is not a music event");
        for (win, tactical) in [(true, true), (false, true), (true, false), (false, false)] {
            let plain = c.result_music(faction, None, win, tactical);
            assert!(plain.iter().all(|e| music.get(e).is_some()), "{faction} win={win} tactical={tactical}: {plain:?} not all music events");
            assert!(!plain.is_empty(), "{faction} win={win} tactical={tactical}: no result theme");
        }
    }
}

/// The `SFX` line of every story dialog chapter (`Scripts/Story/Dialog_*.txt`) plays an existing sample.
#[test]
fn story_dialog_sfx_plays() {
    use alamo_fs::FileSource;
    let Some(install) = GameInstall::detect().filter(|i| i.mod_data(REMAKE).is_dir()) else {
        eprintln!("skipped: Remake mod not installed");
        return;
    };
    let fs = GameFs::for_mod(&install, Some(REMAKE)).expect("fs");
    let mut w = Vec::new();
    let sfx = SfxLibrary::load(&fs, &mut w);
    let resolver = SampleResolver::new(&fs);
    let mut names = BTreeSet::new();
    for f in fs.list("Scripts/Story").iter().filter(|f| f.to_ascii_lowercase().ends_with(".txt")) {
        let text = String::from_utf8_lossy(&fs.read(f).unwrap()).into_owned();
        names.extend(text.lines().filter_map(|l| l.trim().strip_prefix("SFX ")).map(|n| n.trim().to_string()));
    }
    assert!(!names.is_empty(), "no SFX lines in the story dialogs");
    for n in names {
        let e = sfx.get(&n).unwrap_or_else(|| panic!("dialog SFX `{n}` is not an SFX event"));
        assert!(!e.samples.is_empty(), "dialog SFX `{n}` has no samples");
        assert!(e.samples.iter().all(|s| resolver.resolve(&fs, s).is_some()), "dialog SFX `{n}`: a sample is missing");
    }
}
