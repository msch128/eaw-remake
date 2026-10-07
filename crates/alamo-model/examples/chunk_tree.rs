//! Raw chunk tree dumper: `chunk_tree <path-in-gamefs> [workshop_id]`.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::chunk::{Chunk, Chunks};

fn dump(c: Chunk, depth: usize, max_hex: usize) {
    let pad = "  ".repeat(depth);
    if c.container {
        println!("{pad}{:#x} [container] {} bytes", c.id, c.data.len());
        for ch in c.children() {
            match ch {
                Ok(ch) => dump(ch, depth + 1, max_hex),
                Err(e) => println!("{pad}  ERR {e}"),
            }
        }
    } else {
        let n = c.data.len().min(max_hex);
        let hex: Vec<String> = c.data[..n].iter().map(|b| format!("{b:02x}")).collect();
        let txt: String = c.data[..n]
            .iter()
            .map(|&b| {
                if (32..127).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!(
            "{pad}{:#x} {} bytes: {} |{}|",
            c.id,
            c.data.len(),
            hex.join(" "),
            txt
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(
        &install,
        Some(args.get(2).map(String::as_str).unwrap_or("1770851727")),
    )
    .unwrap();
    let data = fs.read(&args[1]).unwrap();
    let max_hex = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(48);
    for c in Chunks::new(&data) {
        dump(c.unwrap(), 0, max_hex);
    }
}
