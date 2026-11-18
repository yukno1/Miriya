//! 全局日志器：`Log` trait 与进程级日志状态。

use std::fmt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::Level;
use crate::record::Record;

/// 日志后端 trait，实现后可通过 [`set_logger`] 注册为全局日志器。
pub trait Log: Send + Sync {
    /// 判断某条日志是否需要输出。
    fn enabled(&self, level: Level, target: &str) -> bool;
    /// 处理一条日志。
    fn log(&self, record: &Record<'_>);
    /// 刷新缓冲。
    fn flush(&self);
}

static MAX_LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);
static LOGGER: OnceLock<Box<dyn Log>> = OnceLock::new();
static SET_ERROR: OnceLock<SetLoggerError> = OnceLock::new();

/// 安装全局日志器失败的原因。
#[derive(Debug, Clone)]
pub struct SetLoggerError {
    /// 错误描述。
    pub message: String,
}

impl fmt::Display for SetLoggerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SetLoggerError {}

/// 安装全局日志器（进程内只能安装一次）。
///
/// 同时会调用 [`set_max_level`] 将全局等级设为 `max_level`。
pub fn set_logger(logger: Box<dyn Log>, max_level: Level) -> Result<(), SetLoggerError> {
    match LOGGER.set(logger) {
        Ok(()) => {
            set_max_level(max_level);
            Ok(())
        }
        Err(_) => Err(SET_ERROR
            .get_or_init(|| SetLoggerError {
                message: "全局日志器已被安装".to_string(),
            })
            .clone()),
    }
}

/// 便捷初始化：安装 [`crate::ConsoleLogger`] 并设置全局等级。
///
/// 若日志器已安装则返回 `false`。
pub fn init(level: Level) -> bool {
    set_logger(Box::new(crate::ConsoleLogger::new(level)), level).is_ok()
}

/// 设置全局最大日志等级：低于该等级的日志将被宏直接跳过。
pub fn set_max_level(level: Level) {
    MAX_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// 当前全局最大日志等级。
pub fn max_level() -> Level {
    match MAX_LEVEL.load(Ordering::Relaxed) {
        0 => Level::Trace,
        1 => Level::Debug,
        2 => Level::Info,
        3 => Level::Warn,
        _ => Level::Error,
    }
}

/// 当前已安装的全局日志器。
pub fn logger() -> Option<&'static dyn Log> {
    LOGGER.get().map(|box_| box_.as_ref())
}

/// 日志宏的分发入口（内部使用）。
#[doc(hidden)]
pub fn __dispatch(
    level: Level,
    target: Option<&str>,
    loc: Option<(&'static str, u32)>,
    args: fmt::Arguments<'_>,
) {
    let Some(logger) = logger() else {
        return;
    };
    let record = Record::new(
        level,
        target,
        loc.map(|(f, _)| f),
        loc.map(|(_, l)| l),
        args,
    );
    if logger.enabled(record.level(), record.target()) {
        logger.log(&record);
        logger.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Capturing(Mutex<Vec<String>>);

    impl Log for Capturing {
        fn enabled(&self, _level: Level, _target: &str) -> bool {
            true
        }
        fn log(&self, record: &Record<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{} {}", record.level(), record.args()));
        }
        fn flush(&self) {}
    }

    #[test]
    fn set_and_dispatch() {
        let capture = Box::new(Capturing(Mutex::new(Vec::new())));
        assert!(set_logger(capture, Level::Warn).is_ok());
        // 第二次安装应失败。
        assert!(set_logger(Box::new(Capturing(Mutex::new(Vec::new()))), Level::Info).is_err());
        assert_eq!(max_level(), Level::Warn);

        __dispatch(Level::Error, None, None, format_args!("boom"));
        __dispatch(Level::Debug, None, None, format_args!("hidden"));
        assert_eq!(max_level(), Level::Warn);
    }
}
