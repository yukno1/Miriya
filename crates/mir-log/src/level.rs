//! 日志等级。

use std::fmt;
use std::str::FromStr;

/// 日志等级，从低到高：Trace、Debug、Info、Warn、Error。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// 极详细的跟踪信息。
    Trace = 0,
    /// 调试信息。
    Debug = 1,
    /// 一般运行信息。
    Info = 2,
    /// 警告。
    Warn = 3,
    /// 错误。
    Error = 4,
}

impl Level {
    /// 全部等级，从低到高。
    pub const ALL: [Level; 5] = [
        Level::Trace,
        Level::Debug,
        Level::Info,
        Level::Warn,
        Level::Error,
    ];

    /// 等级的字符串形式（大写）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    /// 是否高于等于 `other`（即更严重）。
    pub const fn at_least(self, other: Level) -> bool {
        (self as u8) >= (other as u8)
    }

    /// 输出到终端时使用的 ANSI 颜色码（Trace 灰、Debug 青、Info 绿、Warn 黄、Error 红）。
    pub const fn ansi_color(self) -> &'static str {
        match self {
            Level::Trace => "\x1b[90m",
            Level::Debug => "\x1b[36m",
            Level::Info => "\x1b[32m",
            Level::Warn => "\x1b[33m",
            Level::Error => "\x1b[31m",
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 解析日志等级失败时的错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseLevelError;

impl fmt::Display for ParseLevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "无效的日志等级")
    }
}

impl std::error::Error for ParseLevelError {}

impl FromStr for Level {
    type Err = ParseLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "trace" => Ok(Level::Trace),
            "debug" => Ok(Level::Debug),
            "info" => Ok(Level::Info),
            "warn" | "warning" => Ok(Level::Warn),
            "error" => Ok(Level::Error),
            _ => Err(ParseLevelError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering() {
        assert!(Level::Error > Level::Warn);
        assert!(Level::Trace < Level::Debug);
        assert!(Level::Error.at_least(Level::Error));
        assert!(Level::Warn.at_least(Level::Info));
        assert!(!Level::Trace.at_least(Level::Info));
    }

    #[test]
    fn parse() {
        assert_eq!("warn".parse::<Level>(), Ok(Level::Warn));
        assert_eq!("ERROR".parse::<Level>(), Ok(Level::Error));
        assert!("nope".parse::<Level>().is_err());
    }

    #[test]
    fn display() {
        assert_eq!(Level::Trace.to_string(), "TRACE");
    }
}
