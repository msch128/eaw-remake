//! World-space scale of the bones that carry proxies: `bone_scales <model>`.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::Model;

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let m = Model::parse(&fs.read(&path).unwrap()).unwrap();
    for p in &m.proxies {
        let w = m.world_transform(p.bone);
        let col = |c: usize| (w[0][c] * w[0][c] + w[1][c] * w[1][c] + w[2][c] * w[2][c]).sqrt();
        let local = m.bones[p.bone].transform;
        let lcol = |c: usize| {
            (local[0][c] * local[0][c] + local[1][c] * local[1][c] + local[2][c] * local[2][c])
                .sqrt()
        };
        println!(
            "{:36} bone {:3} world scale ({:.3},{:.3},{:.3}) local scale ({:.3},{:.3},{:.3}) pos ({:.0},{:.0},{:.0})",
            p.name,
            p.bone,
            col(0),
            col(1),
            col(2),
            lcol(0),
            lcol(1),
            lcol(2),
            w[0][3],
            w[1][3],
            w[2][3]
        );
    }
}
