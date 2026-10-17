//! Alamo model (`.alo`) and animation (`.ala`) parsing.
//!
//! Parses Alamo engine assets into engine-neutral data and depends only on `alamo-fs`: models with skeleton, meshes
//! (LOD from `_ALTn`), materials, bone palettes, proxies and lights ([`Model`]); animations in both encodings
//! ([`Animation`]); particle systems, which are `.alo` files with root chunk `0x900` ([`ParticleSystem`]); and
//! minimal texture decoding (DDS DXT1/3/5, TGA, PNG) in `image` and `png`. Data is returned as stored, in the
//! engine's right-handed Z-up space with the bow at -Y. Undecoded bytes are kept raw and inferred fields are marked
//! "Inferred" in their doc comments. The `parse_all*` examples walk the whole Remake corpus and must report 0 errors.
pub mod ala;
pub mod alo;
pub mod chunk;
pub mod error;
pub mod image;
pub mod particle;
pub mod png;
mod summary;
pub use ala::Animation;
pub use alo::Model;
pub use error::{Error, Result};
pub use particle::ParticleSystem;
