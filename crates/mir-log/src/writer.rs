//! 内置的终端控制台日志器。

use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::OnceLock;
use std::time::Instant;

use crate::Level;
use crate::logger::Log;
use crate::record::Record;

/// 控制台日志器：将日志以带颜色、带时间戳的格式输出到 stdout/stderr。
///
/// 等级高于（含）[`Self::max`] 的记录才会输出；`Warn`/`Error` 写入 stderr，其余写入 stdout。
#[derive(Debug, Clone)]
pub struct ConsoleLogger {
    /// 本地最大输出等级。
    pub max: Level,
    /// 是否使用 ANSI 颜色（默认启用）。
    pub color: bool,
    /// 是否在行尾附带来源位置 `file:line`。
    pub show_location: bool,
    /// 是否输出目标（模块路径）。
    pub show_target: bool,
}

impl ConsoleLogger {
    /// 创建输出等级为 `max` 的控制台日志器。
    pub fn new(max: Level) -> Self {
        Self {
            max,
            color: true,
            show_location: false,
            show_target: true,
        }
    }

    /// 格式化一条记录为单行文本。
    fn format(&self, record: &Record<'_>, start: Instant) -> String {
        let elapsed = start.elapsed().as_secs_f64();
        let mut line = String::with_capacity(64);

        // 时间戳：自日志器创建起的秒数。
        let _ = write!(line, "[{elapsed:8.3}s] ");

        // 等级（带颜色的 5 字符宽标签）。
        if self.color {
            let _ = write!(
                line,
                "{}{:<5}\x1b[0m",
                record.level().ansi_color(),
                record.level()
            );
        } else {
            let _ = write!(line, "{:<5}", record.level());
        }

        if self.show_target {
            let _ = write!(line, " [{}]", record.target());
        }

        let _ = write!(line, " {}", record.args());

        if self.show_location {
            if let (Some(file), Some(line_no)) = (record.file(), record.line()) {
                let _ = write!(line, " ({file}:{line_no})");
            }
        }

        line
    }
}

impl Log for ConsoleLogger {
    fn enabled(&self, level: Level, _target: &str) -> bool {
        level.at_least(self.max)
    }

    fn log(&self, record: &Record<'_>) {
        static START: OnceLock<Instant> = OnceLock::new();
        let start = START.get_or_init(Instant::now);

        let line = self.format(record, *start);
        let to_stderr = record.level().at_least(Level::Warn);

        let mut sink: Box<dyn std::io::Write> = if to_stderr {
            Box::new(std::io::stderr().lock())
        } else {
            Box::new(std::io::stdout().lock())
        };
        let _ = writeln!(sink, "{line}");
    }

    fn flush(&self) {
        let _ = std::io::stdout().lock().flush();
        let _ = std::io::stderr().lock().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_line() {
        let logger = ConsoleLogger::new(Level::Trace);
        let record = Record::new(
            Level::Warn,
            Some("mir_ecs"),
            None,
            None,
            format_args!("oops"),
        );
        let line = logger.format(&record, Instant::now());
        assert!(line.contains("WARN"), "行应包含等级：{line}");
        assert!(line.contains("[mir_ecs]"), "行应包含目标：{line}");
        assert!(line.contains("oops"), "行应包含消息：{line}");
    }

    #[test]
    fn respects_local_max() {
        let logger = ConsoleLogger::new(Level::Info);
        assert!(logger.enabled(Level::Error, "x"));
        assert!(!logger.enabled(Level::Debug, "x"));
    }
}
