use alamo_fs::{GameFs, GameInstall};
use alamo_model::image::encode_png;
use eaw_ui::faction::UiFaction;
use eaw_ui::shell::{ShellLayout, rasterize};
use eaw_ui::textures::TextureLibrary;
use std::sync::Arc;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().unwrap();
    let fs = Arc::new(GameFs::for_mod(&install, Some("1770851727")).unwrap());
    let lay = ShellLayout::load(&fs, &a[0]).unwrap();
    let lib = TextureLibrary::new(fs);
    let faction = UiFaction::from_name(&a[2]);
    for art in lay.art_for(faction) {
        let (lo, hi) = art.bounds();
        println!("{} alt={:?} tex={} tris={} bounds {lo}..{hi}", art.name, art.alt, art.texture, art.triangles.len());
        let tex = lib.load(&art.texture).unwrap();
        let img = rasterize(&art.triangles, &tex, lo, hi, 2.0);
        std::fs::write(format!("{}/{}.png", a[1], art.name), encode_png(&img)).unwrap();
    }
}
