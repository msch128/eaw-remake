//! Ship length versus damage-fire proxies: `fire_scale`.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::Model;

fn main() {
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let mut rows = Vec::new();
    for f in fs.list("Art/Models") {
        if !f.ends_with(".ALO") {
            continue;
        }
        let Ok(d) = fs.read(&f) else { continue };
        let Ok(m) = Model::parse(&d) else { continue };
        let fires = m
            .proxies
            .iter()
            .filter(|p| p.name.to_lowercase().starts_with("p_ship_fire"))
            .count();
        if fires == 0 {
            continue;
        }
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (i, mesh) in m.meshes.iter().enumerate() {
            if mesh.hidden || mesh.is_shadow_volume() {
                continue;
            }
            let w = m.world_transform(m.bone_of_object(i));
            for c in 0..8 {
                let p = [
                    if c & 1 == 0 {
                        mesh.bbox_min[0]
                    } else {
                        mesh.bbox_max[0]
                    },
                    if c & 2 == 0 {
                        mesh.bbox_min[1]
                    } else {
                        mesh.bbox_max[1]
                    },
                    if c & 4 == 0 {
                        mesh.bbox_min[2]
                    } else {
                        mesh.bbox_max[2]
                    },
                ];
                for k in 0..3 {
                    let v = w[k][0] * p[0] + w[k][1] * p[1] + w[k][2] * p[2] + w[k][3];
                    lo[k] = lo[k].min(v);
                    hi[k] = hi[k].max(v);
                }
            }
        }
        let len = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max);
        rows.push((len, f.clone(), fires));
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (len, f, n) in rows.iter().step_by((rows.len() / 25).max(1)) {
        println!("{len:8.0}  fires {n:2}  {f}");
    }
    println!("{} models", rows.len());
}
