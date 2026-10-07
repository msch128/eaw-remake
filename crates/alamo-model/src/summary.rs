//! Human readable summary of a parsed model.

use std::fmt;

use crate::alo::{Model, ParamValue};

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "bones: {}", self.bones.len())?;
        for (i, b) in self.bones.iter().enumerate() {
            let w = self.world_transform(i);
            writeln!(
                f,
                "  [{i}] {:<28} parent {:>3} vis {} bb {:?} local t ({:.3}, {:.3}, {:.3}) world t ({:.3}, {:.3}, {:.3}) world rows {:.2?}",
                b.name,
                b.parent.map_or(-1, |p| p as i64),
                u8::from(b.visible),
                b.billboard,
                b.transform[0][3],
                b.transform[1][3],
                b.transform[2][3],
                w[0][3],
                w[1][3],
                w[2][3],
                [&w[0][..3], &w[1][..3], &w[2][..3]],
            )?;
        }
        writeln!(f, "meshes: {}", self.meshes.len())?;
        for (i, m) in self.meshes.iter().enumerate() {
            let bone = self.bone_of_object(i);
            writeln!(
                f,
                "  [{i}] {} lod {} hidden {} collision {} bone [{bone}] {} bbox {:?} .. {:?}",
                m.name,
                m.lod,
                m.hidden,
                m.collision,
                self.bones.get(bone).map_or("?", |b| b.name.as_str()),
                m.bbox_min,
                m.bbox_max,
            )?;
            for (j, s) in m.sub_meshes.iter().enumerate() {
                writeln!(
                    f,
                    "    sub {j}: {} verts, {} tris, format {}, palette {}, shader {}",
                    s.vertices.len(),
                    s.indices.len() / 3,
                    s.vertex_format,
                    s.bone_palette.len(),
                    s.material.shader
                )?;
                for (n, v) in &s.material.params {
                    match v {
                        ParamValue::Texture(t) => writeln!(f, "      {n} = texture {t}")?,
                        other => writeln!(f, "      {n} = {other:?}")?,
                    }
                }
            }
        }
        writeln!(f, "lights: {}", self.lights.len())?;
        for l in &self.lights {
            writeln!(f, "  {l:?}")?;
        }
        writeln!(f, "connections: {}", self.connections.len())?;
        writeln!(f, "proxies: {}", self.proxies.len())?;
        for p in &self.proxies {
            let bone = self.bones.get(p.bone).map_or("?", |b| b.name.as_str());
            writeln!(
                f,
                "  {} on [{}] {bone} hidden {} alt_stay_hidden {}",
                p.name, p.bone, p.hidden, p.alt_decrease_stay_hidden
            )?;
        }
        Ok(())
    }
}
