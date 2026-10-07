//! Small deterministic pseudo random generator (SplitMix64).

use crate::math::Vec3;

/// Deterministic RNG; the same seed yields the same sequence everywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    /// Creates a generator from a seed.
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    /// Generator for a (seed, tick, entity, salt) tuple; used so parallel work stays deterministic.
    pub fn for_entity(seed: u64, tick: u64, id: u32, salt: u64) -> Self {
        let mut r = Rng::new(seed ^ tick.wrapping_mul(0xA24B_AED4_963E_E407) ^ (id as u64).wrapping_mul(0x9FB2_1C65_1E98_DF25) ^ salt);
        r.next_u64();
        r
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform float in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform float in `[a, b)`.
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }

    /// Random point inside the unit ball.
    pub fn in_ball(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(self.range(-1.0, 1.0), self.range(-1.0, 1.0), self.range(-1.0, 1.0));
            if v.len_sq() <= 1.0 {
                return v;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let mut a = Rng::for_entity(1, 2, 3, 4);
        let mut b = Rng::for_entity(1, 2, 3, 4);
        assert_eq!(a.next_u64(), b.next_u64());
        assert_ne!(Rng::for_entity(1, 2, 3, 5).next_u64(), Rng::for_entity(1, 2, 3, 4).next_u64());
        assert!(a.in_ball().len() <= 1.0);
    }
}
