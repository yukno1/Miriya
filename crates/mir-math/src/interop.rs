//! 单位与坐标约定互操作（ADR-0003）。
//!
//! 运行时统一使用 **厘米、Z-up、左手系、X 前 / Y 右**。
//! 与米制输入/演示数值的换算只允许经过本模块，避免在业务代码里散落 `* 100`。

use crate::Vec3;

/// 一米对应的厘米数。
pub const CENTIMETERS_PER_METER: f32 = 100.0;

/// 一厘米对应的米数。
pub const METERS_PER_CENTIMETER: f32 = 1.0 / CENTIMETERS_PER_METER;

/// 米 → 厘米。
#[inline]
pub fn meters_to_centimeters(meters: f32) -> f32 {
    meters * CENTIMETERS_PER_METER
}

/// 厘米 → 米。
#[inline]
pub fn centimeters_to_meters(centimeters: f32) -> f32 {
    centimeters * METERS_PER_CENTIMETER
}

/// 米制向量 → 厘米向量。
#[inline]
pub fn meters_to_centimeters_vec3(v: Vec3) -> Vec3 {
    v * CENTIMETERS_PER_METER
}

/// 厘米向量 → 米制向量。
#[inline]
pub fn centimeters_to_meters_vec3(v: Vec3) -> Vec3 {
    v * METERS_PER_CENTIMETER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_centimeter_roundtrip() {
        assert_eq!(meters_to_centimeters(3.0), 300.0);
        assert_eq!(centimeters_to_meters(300.0), 3.0);
        assert_eq!(
            meters_to_centimeters_vec3(Vec3::new(1.0, -2.0, 0.5)),
            Vec3::new(100.0, -200.0, 50.0)
        );
        assert_eq!(
            centimeters_to_meters_vec3(Vec3::new(100.0, -200.0, 50.0)),
            Vec3::new(1.0, -2.0, 0.5)
        );
    }
}
