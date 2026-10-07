//! ALA probe.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::alo::minis;
use alamo_model::chunk::Chunks;

fn show(label: &str, d: &[u8]) {
    let mut s = String::new();
    for (id, p) in minis(d) {
        let v = if p.len() == 4 {
            format!(
                "u32={} f32={}",
                u32::from_le_bytes(p.try_into().unwrap()),
                f32::from_le_bytes(p.try_into().unwrap())
            )
        } else if p.len() == 12 {
            let f: Vec<f32> = p
                .chunks(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            format!("{f:?}")
        } else if p.len() == 2 {
            format!("u16={}", u16::from_le_bytes(p.try_into().unwrap()))
        } else {
            format!("{:?}", String::from_utf8_lossy(p))
        };
        s += &format!(" [{id:#x}:{}] {v}", p.len());
    }
    println!("{label}:{s}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let d = fs.read(&args[1]).unwrap();
    let nb: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);
    for root in Chunks::new(&d).flatten() {
        let mut bones = 0;
        for c in root.children().flatten() {
            match c.id {
                0x1001 => show("header", c.data),
                0x1002 => {
                    bones += 1;
                    if bones > nb {
                        continue;
                    }
                    let kids: Vec<String> = c
                        .children()
                        .flatten()
                        .map(|k| format!("{:#x}({})", k.id, k.data.len()))
                        .collect();
                    println!("bone {bones}: {}", kids.join(" "));
                    for k in c.children().flatten() {
                        if k.id == 0x1003 {
                            show("  info", k.data);
                        } else {
                            println!("  {:#x} {:02x?}", k.id, &k.data[..k.data.len().min(48)]);
                        }
                    }
                }
                other => println!(
                    "root child {other:#x} len {} {:02x?}",
                    c.data.len(),
                    &c.data[..c.data.len().min(64)]
                ),
            }
        }
    }
}
