//! Prints the resolved definition of SFX events with the length of their samples: `sfx_dump <event>...` (Remake 4.0).
use alamo_audio::{SampleResolver, SfxLibrary, Wav};
use alamo_fs::{FileSource, GameFs, GameInstall};

fn main() {
    let install = GameInstall::detect().expect("game not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let mut w = Vec::new();
    let sfx = SfxLibrary::load(&fs, &mut w);
    let resolver = SampleResolver::new(&fs);
    for name in std::env::args().skip(1) {
        let Some(e) = sfx.get(&name) else {
            println!("{name}: not found");
            continue;
        };
        let secs: Vec<f32> = e
            .samples
            .iter()
            .filter_map(|s| resolver.resolve(&fs, s))
            .filter_map(|(p, _)| fs.read(&p).ok())
            .filter_map(|b| Wav::parse(&b).ok()?.decode().ok())
            .map(|p| p.samples.len() as f32 / (p.channels.max(1) as f32 * p.sample_rate.max(1) as f32))
            .collect();
        let avg = secs.iter().sum::<f32>() / secs.len().max(1) as f32;
        println!(
            "{}: samples {} (avg {avg:.2}s, max {:.2}s) vol {:?} prio {} max_inst {:?} prob {} sat {:?} 2d {} loop {} overlap {:?} kills_prev {}",
            e.name, e.samples.len(), secs.iter().cloned().fold(0.0, f32::max), e.volume, e.priority, e.max_instances, e.probability, e.saturation_distance,
            e.is_2d, e.looping, e.overlap_group, e.kills_previous
        );
    }
}
