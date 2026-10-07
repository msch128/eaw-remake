//! Searches the XML data: `xml_grep <regex-free substring, case-insensitive> [after_lines] [max_hits]`.
use alamo_fs::{FileSource, GameFs, GameInstall};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let needle = a[0].to_lowercase();
    let after: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let max: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let mut hits = 0;
    for f in fs.list("XML") {
        if !f.ends_with(".XML") {
            continue;
        }
        let Ok(bytes) = fs.read(&f) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            if l.to_lowercase().contains(&needle) {
                println!("--- {f}:{}", i + 1);
                for l in &lines[i..(i + 1 + after).min(lines.len())] {
                    println!("{l}");
                }
                hits += 1;
                if hits >= max {
                    return;
                }
            }
        }
    }
}
