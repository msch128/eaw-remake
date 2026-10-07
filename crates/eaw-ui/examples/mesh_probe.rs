use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::Model;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let m = Model::parse(&fs.read(&a[0]).unwrap()).unwrap();
    let filt = a.get(1).cloned().unwrap_or_default();
    for me in &m.meshes {
        if !me.name.to_lowercase().contains(&filt.to_lowercase()) { continue; }
        let b = m.world_transform(m.bone_of_object(m.meshes.iter().position(|x| x.name==me.name).unwrap()));
        println!("mesh {} hidden {} bone t=({:.1},{:.1},{:.1})", me.name, me.hidden, b[0][3], b[1][3], b[2][3]);
        for s in &me.sub_meshes {
            println!(" shader {} tex {:?}", s.material.shader, s.material.texture("BaseTexture"));
            for (i, v) in s.vertices.iter().enumerate() { println!("  v{i} p=({:.1},{:.1},{:.1}) uv=({:.4},{:.4})", v.position[0], v.position[1], v.position[2], v.uv[0][0], v.uv[0][1]); }
            println!("  idx {:?}", s.indices);
        }
    }
}
