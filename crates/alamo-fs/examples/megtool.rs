//! Small inspection tool for MEG archives and the layered game file system.
//!
//! ```text
//! megtool list <meg>
//! megtool extract <meg> <outdir>
//! megtool vfs-list <workshop_id|-> <dir>
//! megtool vfs-cat <workshop_id|-> <path>
//! megtool vfs-extract <workshop_id|-> <path> <outfile>
//! ```
//! Use `-` as workshop id for the unmodded Forces of Corruption game.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use alamo_fs::{FileSource, GameFs, GameInstall, MegArchive};

fn vfs(id: &str) -> Result<GameFs, String> {
    let install =
        GameInstall::detect().ok_or("Empire at War install not found (set EAW_GAME_DIR)")?;
    let id = (id != "-").then_some(id);
    GameFs::for_mod(&install, id).map_err(|e| e.to_string())
}

fn run(args: &[String]) -> Result<(), String> {
    let a = |i: usize| {
        args.get(i)
            .map(String::as_str)
            .ok_or_else(|| "missing argument".to_string())
    };
    let err = |e: std::io::Error| e.to_string();
    match a(0)? {
        "list" => {
            let meg = MegArchive::open(a(1)?).map_err(err)?;
            for e in meg.entries() {
                println!("{:>10}  {}", e.size, e.name);
            }
            eprintln!("{} entries", meg.len());
        }
        "extract" => {
            let meg = MegArchive::open(a(1)?).map_err(err)?;
            let out = Path::new(a(2)?);
            for (i, e) in meg.entries().iter().enumerate() {
                let dest = out.join(&e.name);
                std::fs::create_dir_all(dest.parent().unwrap()).map_err(err)?;
                std::fs::write(dest, meg.read_entry(i).map_err(err)?).map_err(err)?;
            }
            eprintln!("extracted {} files", meg.len());
        }
        "vfs-list" => {
            let fs = vfs(a(1)?)?;
            let files = fs.list(a(2)?);
            for f in &files {
                println!("{f}");
            }
            eprintln!(
                "{} files ({} visible in total)",
                files.len(),
                fs.file_count()
            );
        }
        "vfs-cat" => {
            let data = vfs(a(1)?)?.read(a(2)?).map_err(err)?;
            std::io::stdout().write_all(&data).map_err(err)?;
        }
        "vfs-extract" => {
            let fs = vfs(a(1)?)?;
            let path = a(2)?;
            std::fs::write(a(3)?, fs.read(path).map_err(err)?).map_err(err)?;
            eprintln!("from {}", fs.origin(path).unwrap_or_default());
        }
        _ => return Err("usage: megtool list|extract|vfs-list|vfs-cat|vfs-extract ...".into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
