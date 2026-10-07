//! Loads all audio event data of a mod and reports counts, unresolved samples and WAV/MP3 codecs.
//!
//! ```text
//! audio_census [workshop_id]
//! ```

use std::collections::BTreeMap;

use alamo_audio::resolve::Resolution;
use alamo_audio::{MusicLibrary, SampleResolver, SfxLibrary, SpeechLibrary, Wav};
use alamo_fs::{FileSource, GameFs, GameInstall};

fn main() {
    let id = std::env::args().nth(1);
    let install = GameInstall::detect().expect("game not found");
    let fs = GameFs::for_mod(&install, id.as_deref()).expect("fs");
    let mut w = Vec::new();
    let sfx = SfxLibrary::load(&fs, &mut w);
    let speech = SpeechLibrary::load(&fs, &mut w);
    let music = MusicLibrary::load(&fs, &mut w);
    println!(
        "sfx {} (presets {}), speech {}, music {}",
        sfx.len(),
        sfx.iter().filter(|e| e.is_preset).count(),
        speech.len(),
        music.len()
    );
    println!("warnings: {}", w.len());
    for x in w.iter().take(10) {
        println!("  {}:{} {}", x.file, x.line, x.message);
    }
    let resolver = SampleResolver::new(&fs);
    println!("audio files indexed: {}", resolver.len());

    let mut by_res: BTreeMap<String, usize> = BTreeMap::new();
    let mut unresolved: Vec<String> = Vec::new();
    let mut paths = std::collections::BTreeSet::new();
    for e in sfx.iter().filter(|e| !e.is_preset) {
        for s in e
            .samples
            .iter()
            .chain(&e.pre_samples)
            .chain(&e.post_samples)
        {
            match resolver.resolve(&fs, s) {
                Some((p, r)) => {
                    *by_res.entry(format!("{r:?}")).or_default() += 1;
                    paths.insert(p);
                }
                None => unresolved.push(format!("{}: {s}", e.name)),
            }
        }
    }
    println!(
        "sfx sample refs by resolution: {by_res:?}; unresolved refs: {}",
        unresolved.len()
    );
    let mut pre: BTreeMap<String, usize> = BTreeMap::new();
    for u in &unresolved {
        let s = u.rsplit(": ").next().unwrap();
        let k: String = if s.contains('\\') {
            s.rsplit_once('\\').unwrap().0.to_string()
        } else {
            s.chars().take(4).collect()
        };
        *pre.entry(k).or_default() += 1;
    }
    let mut top: Vec<_> = pre.into_iter().collect();
    top.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!("unresolved by prefix/dir: {:?}", &top[..top.len().min(12)]);
    for u in unresolved.iter().take(8) {
        println!("  {u}");
    }
    let _ = Resolution::Exact;

    let mut formats: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let (mut decoded, mut failed, mut mp3) = (0, 0, 0);
    for p in &paths {
        let Ok(bytes) = fs.read(p) else { continue };
        if p.ends_with(".MP3") {
            mp3 += 1;
            continue;
        }
        match Wav::parse(&bytes) {
            Ok(wav) => {
                let f = wav.format;
                let key = format!(
                    "tag {:#x} ch {} {}Hz {}bit align {}",
                    f.tag, f.channels, f.sample_rate, f.bits, f.block_align
                );
                let entry = formats.entry(key).or_insert((0, p.clone()));
                entry.0 += 1;
                match wav.decode() {
                    Ok(pcm) if !pcm.samples.is_empty() => decoded += 1,
                    _ => failed += 1,
                }
            }
            Err(_) => failed += 1,
        }
    }
    println!(
        "distinct referenced files: {} (mp3 {mp3}), wav decoded {decoded}, failed {failed}",
        paths.len()
    );
    for (k, (n, ex)) in &formats {
        println!("  {n:6} {k}   e.g. {ex}");
    }
}
