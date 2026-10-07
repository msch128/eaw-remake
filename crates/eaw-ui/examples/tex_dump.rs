use alamo_fs::{GameFs, GameInstall};
use alamo_model::image::encode_png;
use eaw_ui::textures::TextureLibrary;
use std::sync::Arc;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().unwrap();
    let fs = Arc::new(GameFs::for_mod(&install, Some("1770851727")).unwrap());
    let lib = TextureLibrary::new(fs);
    let out = &a[0];
    for n in &a[1..] {
        match lib.load(n) {
            Some(img) => {
                let stem = n.rsplit_once('.').map_or(n.as_str(), |s| s.0);
                println!("{n}: {}x{} mega={}", img.width, img.height, lib.in_mega_texture(n));
                std::fs::write(format!("{out}/{stem}.png"), encode_png(&img)).unwrap();
            }
            None => println!("{n}: MISSING"),
        }
    }
}
