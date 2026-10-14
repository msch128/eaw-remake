//! Audits every SFX / music hook tag of a mod's XML: does the named event exist, have samples, and do the files exist?
//!
//! ```text
//! audio_hooks [workshop_id] [--md]     (default mod: Remake 4.0, 1770851727)
//! ```
//! `--md` prints the per-tag table as Markdown; the default prints the problems of each tag.

use alamo_audio::hooks::{audit_hooks, is_wired, scan_hooks};
use alamo_audio::{MusicLibrary, SampleResolver, SfxLibrary};
use alamo_fs::{GameFs, GameInstall};

fn main() {
    let mut md = false;
    let mut id = Some("1770851727".to_string());
    for a in std::env::args().skip(1) {
        if a == "--md" {
            md = true;
        } else {
            id = Some(a);
        }
    }
    let install = GameInstall::detect().expect("game not found");
    let fs = GameFs::for_mod(&install, id.as_deref()).expect("fs");
    let mut w = Vec::new();
    let sfx = SfxLibrary::load(&fs, &mut w);
    let music = MusicLibrary::load(&fs, &mut w);
    let resolver = SampleResolver::new(&fs);
    let refs = scan_hooks(&fs);
    let audit = audit_hooks(&refs, &sfx, &music, &resolver, &fs);
    println!("{} hook references, {} tags", refs.len(), audit.tags.len());
    if md {
        print!("{}", audit.to_markdown(is_wired));
        return;
    }
    for (tag, t) in &audit.tags {
        if t.unknown.is_empty() && t.no_samples.is_empty() && t.missing.is_empty() {
            continue;
        }
        println!(
            "{tag} [{}] refs {} events {} play {} (e.g. {})",
            if is_wired(tag) { "wired" } else { "NOT wired" },
            t.refs,
            t.events.len(),
            t.ok.len(),
            t.example_file
        );
        for e in t.unknown.iter().take(6) {
            println!("    unknown event {e}");
        }
        for e in t.no_samples.iter().take(6) {
            println!("    no samples    {e}");
        }
        for (e, s) in t.missing.iter().take(6) {
            println!("    missing file  {e}: {s}");
        }
    }
}
