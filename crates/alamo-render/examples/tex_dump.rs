//! Writes a texture as PNG, downscaled, with RGB and alpha side by side: `tex_dump <name> <out.png>`.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::image::{Rgba8, decode_texture, encode_png};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let stem = args[0]
        .rsplit_once('.')
        .map_or(args[0].as_str(), |s| s.0)
        .to_string();
    let data = [".dds", ".tga"]
        .iter()
        .find_map(|e| fs.read(&format!("Art/Textures/{stem}{e}")).ok())
        .expect("texture not found");
    let mut img = decode_texture(&data).expect("decode");
    if args.len() >= 6 {
        let v: Vec<usize> = args[2..6].iter().map(|a| a.parse().unwrap()).collect();
        let mut px = Vec::new();
        for y in v[1]..v[1] + v[3] {
            px.extend_from_slice(
                &img.pixels[(y * img.width + v[0]) * 4..(y * img.width + v[0] + v[2]) * 4],
            );
        }
        img = Rgba8 {
            width: v[2],
            height: v[3],
            pixels: px,
        };
    }
    println!("{}x{}", img.width, img.height);
    let step = (img.width / 256).max(1);
    let (w, h) = (img.width / step, img.height / step);
    let mut px = vec![255u8; w * 2 * h * 4];
    for y in 0..h {
        for x in 0..w {
            let s = ((y * step) * img.width + x * step) * 4;
            let p = &img.pixels[s..s + 4];
            px[(y * w * 2 + x) * 4..][..3].copy_from_slice(&p[..3]);
            px[(y * w * 2 + w + x) * 4..][..3].copy_from_slice(&[p[3]; 3]);
        }
    }
    std::fs::write(
        &args[1],
        encode_png(&Rgba8 {
            width: w * 2,
            height: h,
            pixels: px,
        }),
    )
    .unwrap();
}
