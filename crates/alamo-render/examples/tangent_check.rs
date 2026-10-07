//! Checks whether file tangent/binormal agree with dP/du and dP/dv: `tangent_check <model>`.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::Model;

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let m = Model::parse(&fs.read(&path).unwrap()).unwrap();
    let (mut tu, mut tv, mut bu, mut bv, mut n) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0usize);
    for s in m.meshes.iter().flat_map(|m| &m.sub_meshes) {
        if !s.vertex_format.contains("U3U3") {
            continue;
        }
        for t in s.indices.as_chunks::<3>().0 {
            let v: Vec<_> = t.iter().map(|&i| s.vertices[i as usize]).collect();
            let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
            let (e1, e2) = (
                sub(v[1].position, v[0].position),
                sub(v[2].position, v[0].position),
            );
            let (du1, dv1) = (v[1].uv[0][0] - v[0].uv[0][0], v[1].uv[0][1] - v[0].uv[0][1]);
            let (du2, dv2) = (v[2].uv[0][0] - v[0].uv[0][0], v[2].uv[0][1] - v[0].uv[0][1]);
            let det = du1 * dv2 - du2 * dv1;
            if det.abs() < 1e-9 {
                continue;
            }
            let r = 1.0 / det;
            let dpdu: Vec<f32> = (0..3).map(|k| (e1[k] * dv2 - e2[k] * dv1) * r).collect();
            let dpdv: Vec<f32> = (0..3).map(|k| (e2[k] * du1 - e1[k] * du2) * r).collect();
            let dot = |a: &[f32], b: [f32; 3]| (0..3).map(|k| (a[k] * b[k]) as f64).sum::<f64>();
            tu += dot(&dpdu, v[0].tangent).signum();
            tv += dot(&dpdv, v[0].tangent).signum();
            bu += dot(&dpdu, v[0].binormal).signum();
            bv += dot(&dpdv, v[0].binormal).signum();
            n += 1;
        }
    }
    println!(
        "triangles {n}: sign sums  tangent.dPdu {tu} tangent.dPdv {tv} binormal.dPdu {bu} binormal.dPdv {bv}"
    );
}
