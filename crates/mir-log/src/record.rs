//! 日志记录。

use std::fmt;

use crate::Level;

/// 单条日志记录，携带等级、目标、来源位置与格式化后的消息。
#[derive(Debug, Clone)]
pub struct Record<'a> {
    level: Level,
    target: Option<&'a str>,
    file: Option<&'a str>,
    line: Option<u32>,
    args: fmt::Arguments<'a>,
}

impl<'a> Record<'a> {
    /// 创建一条日志记录。
    pub fn new(
        level: Level,
        target: Option<&'a str>,
        file: Option<&'a str>,
        line: Option<u32>,
        args: fmt::Arguments<'a>,
    ) -> Self {
        Self {
            level,
            target,
            file,
            line,
            args,
        }
    }

    /// 日志等级。
    pub fn level(&self) -> Level {
        self.level
    }

    /// 日志目标（通常为模块路径）。
    pub fn target(&self) -> &'a str {
        self.target.unwrap_or("miriya")
    }

    /// 来源文件。
    pub fn file(&self) -> Option<&'a str> {
        self.file
    }

    /// 来源行号。
    pub fn line(&self) -> Option<u32> {
        self.line
    }

    /// 格式化消息参数。
    pub fn args(&self) -> &fmt::Arguments<'a> {
        &self.args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let record = Record::new(
            Level::Info,
            None,
            Some("lib.rs"),
            Some(7),
            format_args!("hi"),
        );
        assert_eq!(record.target(), "miriya");
        assert_eq!(record.file(), Some("lib.rs"));
        assert_eq!(record.line(), Some(7));
    }
}
