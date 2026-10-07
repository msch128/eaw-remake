//! Particle chunk dumper with typed guesses for mini-chunks: `particle_dump <path> [workshop]`.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::chunk::{Chunk, Chunks, cstr};

fn minis_ok(d: &[u8]) -> bool {
    let mut r = d;
    if r.is_empty() {
        return false;
    }
    while !r.is_empty() {
        if r.len() < 2 {
            return false;
        }
        let n = r[1] as usize;
        if r.len() < 2 + n {
            return false;
        }
        r = &r[2 + n..];
    }
    true
}

fn val(p: &[u8]) -> String {
    match p.len() {
        1 => format!("u8 {}", p[0]),
        4 => format!(
            "u32 {} / f32 {}",
            u32::from_le_bytes(p.try_into().unwrap()),
            f32::from_le_bytes(p.try_into().unwrap())
        ),
        8 => format!(
            "u32 {} f32 {}",
            u32::from_le_bytes(p[..4].try_into().unwrap()),
            f32::from_le_bytes(p[4..].try_into().unwrap())
        ),
        12 => format!(
            "{:?}",
            p.chunks(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect::<Vec<_>>()
        ),
        _ => format!("{:?}", cstr(p)),
    }
}

fn dump(c: Chunk, depth: usize) {
    let pad = "  ".repeat(depth);
    if c.container {
        println!("{pad}{:#x} [container]", c.id);
        for ch in c.children().flatten() {
            dump(ch, depth + 1);
        }
    } else if minis_ok(c.data) {
        println!("{pad}{:#x} minis:", c.id);
        let mut r = c.data;
        while !r.is_empty() {
            let n = r[1] as usize;
            println!("{pad}    [{:#04x}] {}", r[0], val(&r[2..2 + n]));
            r = &r[2 + n..];
        }
    } else {
        println!(
            "{pad}{:#x} raw {} bytes: {:?}",
            c.id,
            c.data.len(),
            cstr(&c.data[..c.data.len().min(80)])
        );
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(
        &install,
        Some(a.get(1).map_or("1770851727", String::as_str)),
    )
    .unwrap();
    for c in Chunks::new(&fs.read(&a[0]).unwrap()).flatten() {
        dump(c, 0);
    }
}
