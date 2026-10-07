//! Alamo model (`.alo`) and animation (`.ala`) parsing.
pub mod ala;
pub mod alo;
pub mod chunk;
pub mod error;
pub mod image;
mod summary;
pub use ala::Animation;
pub use alo::Model;
pub use error::{Error, Result};
