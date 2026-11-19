//! 世代句柄：槽位回收后递增代际，避免悬垂句柄被误用。

use std::fmt;

/// 世代计数。每次回收槽位并重新分配时递增。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Generation(u32);

impl Generation {
    /// 初始代际。
    pub const ZERO: Self = Self(0);

    /// 从原始值构造（后端内部使用）。
    #[inline]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// 原始值。
    #[inline]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// 递增一代；溢出时回绕。
    #[inline]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl fmt::Debug for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gen({})", self.0)
    }
}

macro_rules! define_handle {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name {
            slot: u32,
            generation: Generation,
        }

        impl $name {
            /// 空句柄：不指向任何对象。
            pub const NULL: Self = Self {
                slot: u32::MAX,
                generation: Generation::ZERO,
            };

            /// 由槽位与代际构造（仅后端实现使用）。
            #[inline]
            pub const fn new(slot: u32, generation: Generation) -> Self {
                Self { slot, generation }
            }

            /// 槽位索引。
            #[inline]
            pub const fn slot(self) -> u32 {
                self.slot
            }

            /// 代际。
            #[inline]
            pub const fn generation(self) -> Generation {
                self.generation
            }

            /// 是否为空句柄。
            #[inline]
            pub const fn is_null(self) -> bool {
                self.slot == u32::MAX
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::NULL
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    concat!(stringify!($name), "(slot={}, gen={})"),
                    self.slot,
                    self.generation.get()
                )
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    concat!(stringify!($name), "(slot={}, gen={})"),
                    self.slot,
                    self.generation.get()
                )
            }
        }
    };
}

define_handle!(BodyId, "刚体世代句柄。不透明，引擎只负责转发与校验。");
define_handle!(ColliderId, "碰撞体世代句柄。不透明，引擎只负责转发与校验。");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_handle_is_null() {
        assert!(BodyId::default().is_null());
        assert!(ColliderId::default().is_null());
    }

    #[test]
    fn generation_increments_and_handles_differ() {
        let g0 = Generation::ZERO;
        let g1 = g0.next();
        assert_eq!(g1.get(), g0.get() + 1);

        let a = BodyId::new(0, g0);
        let b = BodyId::new(0, g1);
        assert_ne!(a, b, "同槽位不同代际必须是不同句柄");
        assert_eq!(a.slot(), 0);
        assert_eq!(b.generation(), g1);
    }
}
