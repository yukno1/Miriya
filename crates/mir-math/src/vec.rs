//! 向量类型：对 `glam` 向量的 newtype 封装。

#![allow(non_snake_case)]

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use glam::{Vec2 as GVec2, Vec3 as GVec3, Vec4 as GVec4};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! impl_vec {
    ($name:ident, $glam:ident, $len:expr, [ $($axis:ident),+ ]) => {
        #[derive(Debug, Clone, Copy, PartialEq, Default)]
        #[repr(transparent)]
        pub struct $name(pub(crate) $glam);

        impl Serialize for $name {
            #[inline]
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                self.to_array().serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            #[inline]
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let values = <[f32; $len]>::deserialize(deserializer)?;
                Ok(Self($glam::from_array(values)))
            }
        }

        impl $name {
            pub const ZERO: Self = Self($glam::ZERO);
            pub const ONE: Self = Self($glam::ONE);
            $(
                /// 单位轴常量：仅该分量为 1。
                pub const $axis: Self = Self($glam::$axis);
            )+

            #[inline]
            pub fn new($($axis: f32),+) -> Self {
                Self($glam::new($($axis),+))
            }

            #[inline]
            pub fn splat(v: f32) -> Self {
                Self($glam::splat(v))
            }

            #[inline]
            pub fn dot(self, other: Self) -> f32 {
                self.0.dot(other.0)
            }

            #[inline]
            pub fn length(self) -> f32 {
                self.0.length()
            }

            #[inline]
            pub fn length_sq(self) -> f32 {
                self.0.length_squared()
            }

            /// 归一化；零向量返回 `ZERO`。
            #[inline]
            pub fn normalized(self) -> Self {
                self.0.normalize_or_zero().into()
            }

            #[inline]
            pub fn distance(self, other: Self) -> f32 {
                self.0.distance(other.0)
            }

            #[inline]
            pub fn lerp(self, other: Self, s: f32) -> Self {
                self.0.lerp(other.0, s).into()
            }

            #[inline]
            pub fn min(self, other: Self) -> Self {
                self.0.min(other.0).into()
            }

            #[inline]
            pub fn max(self, other: Self) -> Self {
                self.0.max(other.0).into()
            }
        }

        impl From<$glam> for $name {
            #[inline]
            fn from(v: $glam) -> Self {
                Self(v)
            }
        }

        impl From<$name> for $glam {
            #[inline]
            fn from(v: $name) -> Self {
                v.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = $glam;
            #[inline]
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl std::ops::DerefMut for $name {
            #[inline]
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl Add for $name {
            type Output = Self;
            #[inline]
            fn add(self, rhs: Self) -> Self {
                (self.0 + rhs.0).into()
            }
        }

        impl AddAssign for $name {
            #[inline]
            fn add_assign(&mut self, rhs: Self) {
                self.0 += rhs.0;
            }
        }

        impl Sub for $name {
            type Output = Self;
            #[inline]
            fn sub(self, rhs: Self) -> Self {
                (self.0 - rhs.0).into()
            }
        }

        impl SubAssign for $name {
            #[inline]
            fn sub_assign(&mut self, rhs: Self) {
                self.0 -= rhs.0;
            }
        }

        impl Mul<f32> for $name {
            type Output = Self;
            #[inline]
            fn mul(self, rhs: f32) -> Self {
                (self.0 * rhs).into()
            }
        }

        impl Mul<$name> for f32 {
            type Output = $name;
            #[inline]
            fn mul(self, rhs: $name) -> $name {
                (self * rhs.0).into()
            }
        }

        impl Div<f32> for $name {
            type Output = Self;
            #[inline]
            fn div(self, rhs: f32) -> Self {
                (self.0 / rhs).into()
            }
        }

        impl Neg for $name {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self {
                (-self.0).into()
            }
        }
    };
}

impl_vec!(Vec2, GVec2, 2, [X, Y]);
impl_vec!(Vec3, GVec3, 3, [X, Y, Z]);
impl_vec!(Vec4, GVec4, 4, [X, Y, Z, W]);

impl Vec2 {
    /// 由 `Vec2` 与 z 分量构造 `Vec3`。
    #[inline]
    pub fn extend(self, z: f32) -> Vec3 {
        self.0.extend(z).into()
    }
}

impl Vec3 {
    /// 由 `Vec3` 与 w 分量构造 `Vec4`。
    #[inline]
    pub fn extend(self, w: f32) -> Vec4 {
        self.0.extend(w).into()
    }

    /// 截断 z 分量得到 `Vec2`。
    #[inline]
    pub fn truncate(self) -> Vec2 {
        self.0.truncate().into()
    }

    /// 叉积。
    #[inline]
    pub fn cross(self, other: Self) -> Self {
        self.0.cross(other.0).into()
    }
}

impl Vec4 {
    /// 截断 w 分量得到 `Vec3`。
    #[inline]
    pub fn truncate(self) -> Vec3 {
        self.0.truncate().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_ops() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, 5.0, 6.0);
        assert_eq!(a.dot(b), 32.0);
        assert_eq!(a + b, Vec3::new(5.0, 7.0, 9.0));
        assert_eq!(b - a, Vec3::new(3.0, 3.0, 3.0));
        assert_eq!(a * 2.0, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(2.0 * a, Vec3::new(2.0, 4.0, 6.0));
    }

    #[test]
    fn field_access_via_deref() {
        let mut v = Vec3::ZERO;
        v.x = 1.0;
        v.y = 2.0;
        v.z = 3.0;
        assert_eq!(v, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(v.length(), 14.0_f32.sqrt());
    }

    #[test]
    fn normalize_zero() {
        assert_eq!(Vec3::ZERO.normalized(), Vec3::ZERO);
    }

    #[test]
    fn cross_and_convert() {
        let a = Vec3::new(1.0, 0.0, 0.0);
        let b = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(a.cross(b), Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(Vec2::new(1.0, 0.0).extend(5.0), Vec3::new(1.0, 0.0, 5.0));
        assert_eq!(a.extend(5.0), Vec4::new(1.0, 0.0, 0.0, 5.0));
        assert_eq!(
            Vec4::new(1.0, 2.0, 3.0, 4.0).truncate(),
            Vec3::new(1.0, 2.0, 3.0)
        );
    }
}
