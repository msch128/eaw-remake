//! Lists the light and ambient values of every space map: `light_survey [workshop_id]`.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_map::SpaceMap;

fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "1770851727".into());
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some(&id)).expect("fs");
    let mut rows = Vec::new();
    for f in fs.list("Art/Maps") {
        let l = f.to_ascii_lowercase();
        if !l.ends_with(".ted") || !l.contains("space") {
            continue;
        }
        let Ok(d) = fs.read(&f) else { continue };
        let Ok(m) = SpaceMap::parse(&d) else { continue };
        let e = &m.environment;
        let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let sun = lum(e.lights[0].color) * e.lights[0].scalar;
        let fill: f32 = e.lights[1..].iter().map(|l| lum(l.color) * l.scalar).sum();
        let amb = lum(e.ambient);
        rows.push((f.rsplit('/').next().unwrap_or(&f).to_string(), e.lights[0].scalar, e.lights[1].scalar, e.lights[2].scalar, sun, fill, amb));
    }
    println!("{} maps", rows.len());
    println!("{:44} sun_s fill1 fill2 | sunL fillL  amb", "map");
    for r in &rows {
        println!("{:44} {:.2}  {:.2}  {:.2}  | {:.2}  {:.2}  {:.2}", r.0, r.1, r.2, r.3, r.4, r.5, r.6);
    }
    let stat = |name: &str, mut v: Vec<f32>| {
        v.sort_by(f32::total_cmp);
        println!("{name}: min {:.2} median {:.2} max {:.2}", v[0], v[v.len() / 2], v[v.len() - 1]);
    };
    stat("sun scalar", rows.iter().map(|r| r.1).collect());
    stat("fill1 scalar", rows.iter().map(|r| r.2).collect());
    stat("fill2 scalar", rows.iter().map(|r| r.3).collect());
    stat("sun luminance*scalar", rows.iter().map(|r| r.4).collect());
    stat("fill luminance*scalar", rows.iter().map(|r| r.5).collect());
    stat("ambient luminance", rows.iter().map(|r| r.6).collect());
}
