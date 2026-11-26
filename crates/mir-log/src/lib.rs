//! # mir-log
//!
//! Miriya 引擎的日志系统：无外部依赖、纯 std 实现。
//!
//! 提供五个等级的日志宏（`trace!` / `debug!` / `info!` / `warn!` / `error!`）、
//! 进程级全局日志器注册（[`Log`] trait）以及内置的彩色控制台日志器 [`ConsoleLogger`]。
//!
//! ## 快速开始
//!
//! ```
//! mir_log::init(mir_log::Level::Info);
//! mir_log::info!("引擎启动，帧率限制 {}", 60);
//! mir_log::warn!(target: "mir_ecs", "实体 {} 已销毁", 42);
//! ```
//!
//! ## 自定义日志后端
//!
//! 实现 [`Log`] trait 并通过 [`set_logger`] 注册即可替换控制台输出。

pub mod level;
pub mod logger;
pub mod record;
pub mod writer;

pub use level::{Level, ParseLevelError};
pub use logger::{Log, SetLoggerError, init, logger, max_level, set_logger, set_max_level};
pub use record::Record;
pub use writer::ConsoleLogger;

#[doc(hidden)]
pub use logger::__dispatch;

/// 通用日志宏。
///
/// 用法：`log!(Level::Info, "消息 {}", 1)` 或 `log!(target: "mir_xxx", Level::Info, "消息")`。
#[macro_export]
macro_rules! log {
    (target: $target:expr, $level:expr, $($arg:tt)+) => {{
        let level = $level;
        if level.at_least($crate::max_level()) {
            $crate::__dispatch(level, Some($target), Some((file!(), line!())), format_args!($($arg)+));
        }
    }};
    ($level:expr, $($arg:tt)+) => {
        $crate::log!(target: module_path!(), $level, $($arg)+)
    };
}

/// 输出 Trace 级别日志。
#[macro_export]
macro_rules! trace {
    (target: $target:expr, $($arg:tt)+) => { $crate::log!(target: $target, $crate::Level::Trace, $($arg)+) };
    ($($arg:tt)+) => { $crate::log!(target: module_path!(), $crate::Level::Trace, $($arg)+) };
}

/// 输出 Debug 级别日志。
#[macro_export]
macro_rules! debug {
    (target: $target:expr, $($arg:tt)+) => { $crate::log!(target: $target, $crate::Level::Debug, $($arg)+) };
    ($($arg:tt)+) => { $crate::log!(target: module_path!(), $crate::Level::Debug, $($arg)+) };
}

/// 输出 Info 级别日志。
#[macro_export]
macro_rules! info {
    (target: $target:expr, $($arg:tt)+) => { $crate::log!(target: $target, $crate::Level::Info, $($arg)+) };
    ($($arg:tt)+) => { $crate::log!(target: module_path!(), $crate::Level::Info, $($arg)+) };
}

/// 输出 Warn 级别日志。
#[macro_export]
macro_rules! warn {
    (target: $target:expr, $($arg:tt)+) => { $crate::log!(target: $target, $crate::Level::Warn, $($arg)+) };
    ($($arg:tt)+) => { $crate::log!(target: module_path!(), $crate::Level::Warn, $($arg)+) };
}

/// 输出 Error 级别日志。
#[macro_export]
macro_rules! error {
    (target: $target:expr, $($arg:tt)+) => { $crate::log!(target: $target, $crate::Level::Error, $($arg)+) };
    ($($arg:tt)+) => { $crate::log!(target: module_path!(), $crate::Level::Error, $($arg)+) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macros_compile() {
        let level = Level::Warn;
        log!(target: "mir_test", level, "hello {}", 1);
        log!(level, "plain {}", 2);
        trace!("t");
        debug!("d");
        info!("i");
        warn!("w");
        error!("e");
    }

    #[test]
    fn level_threshold() {
        // 不修改全局等级，避免与 logger::set_and_dispatch 并发竞争。
        assert!(Level::Error.at_least(Level::Error));
        assert!(!Level::Info.at_least(Level::Error));
        assert!(Level::Trace.at_least(Level::Trace));
    }
}
