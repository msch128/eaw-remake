//! Minimal 3D vector math (battle space is X forward, Y left, Z up).

use std::ops::{Add, AddAssign, Mul, Neg, Sub};

/// A 3D vector / point.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    /// X (battle forward axis at yaw 0).
    pub x: f32,
    /// Y (left).
    pub y: f32,
    /// Z (up).
    pub z: f32,
}

impl Vec3 {
    /// Zero vector.
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    /// Creates a vector.
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// From an array.
    pub const fn from_array(a: [f32; 3]) -> Self {
        Self { x: a[0], y: a[1], z: a[2] }
    }

    /// To an array.
    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    /// Dot product.
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// Cross product.
    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    /// Squared length.
    pub fn len_sq(self) -> f32 {
        self.dot(self)
    }

    /// Length.
    pub fn len(self) -> f32 {
        self.len_sq().sqrt()
    }

    /// Unit vector, or zero for a zero vector.
    pub fn normalized(self) -> Vec3 {
        let l = self.len();
        if l > 1e-9 { self * (1.0 / l) } else { Vec3::ZERO }
    }

    /// Distance to another point.
    pub fn dist(self, o: Vec3) -> f32 {
        (self - o).len()
    }

    /// Rotates around Z by `yaw` radians.
    pub fn rotate_yaw(self, yaw: f32) -> Vec3 {
        let (s, c) = yaw.sin_cos();
        Vec3::new(self.x * c - self.y * s, self.x * s + self.y * c, self.z)
    }

    /// Unit vector in the XY plane for a heading.
    pub fn from_yaw(yaw: f32) -> Vec3 {
        let (s, c) = yaw.sin_cos();
        Vec3::new(c, s, 0.0)
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, k: f32) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// Wraps an angle to `(-PI, PI]`.
pub fn wrap_angle(mut a: f32) -> f32 {
    use std::f32::consts::PI;
    while a > PI {
        a -= 2.0 * PI;
    }
    while a <= -PI {
        a += 2.0 * PI;
    }
    a
}

/// Smallest distance from point `c` to the segment `a -> b`, and the parameter `t` in `[0, 1]`
/// of the closest point.
pub fn segment_point_distance(a: Vec3, b: Vec3, c: Vec3) -> (f32, f32) {
    let ab = b - a;
    let l2 = ab.len_sq();
    let t = if l2 > 1e-12 { ((c - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    ((a + ab * t).dist(c), t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        let v = Vec3::new(1.0, 0.0, 0.0).rotate_yaw(std::f32::consts::FRAC_PI_2);
        assert!((v.y - 1.0).abs() < 1e-6 && v.x.abs() < 1e-6);
        assert!((wrap_angle(3.0 * std::f32::consts::PI) - std::f32::consts::PI).abs() < 1e-5);
        let (d, t) = segment_point_distance(Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), Vec3::new(5.0, 3.0, 0.0));
        assert!((d - 3.0).abs() < 1e-6 && (t - 0.5).abs() < 1e-6);
    }
}
