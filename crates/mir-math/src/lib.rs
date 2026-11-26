//! 数学库：对 glam 的轻量封装。
//!
//! 向量、四元数、矩阵等类型均为 glam 对应类型的 newtype 包装，
//! 通过 `Deref` / `DerefMut` 透传 glam 的全部方法，同时提供稳定、
//! 受控的公共 API，后续可平滑替换底层实现。

pub mod frustum;
pub mod interop;
pub mod mat;
pub mod quat;
pub mod transform;
pub mod vec;

pub use frustum::{Aabb, Frustum};
pub use interop::*;
pub use mat::{Mat3, Mat4};
pub use quat::Quat;
pub use transform::Transform;
pub use vec::{Vec2, Vec3, Vec4};
