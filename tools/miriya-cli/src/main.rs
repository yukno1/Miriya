//! Miriya CLI：M4 先落地最小 `cook` 子命令。

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "miriya-cli", version, about = "Miriya 命令行工具")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 把源资产目录封装为版本化 .miasset
    Cook {
        /// 源资产目录
        input: PathBuf,
        /// 输出目录
        #[arg(short, long, default_value = "assets/cooked")]
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Cook { input, output } => {
            let report = mir_build::cook_dir(&input, &output)?;
            for path in &report.cooked {
                println!("cook: {}", path.display());
            }
            for (path, error) in &report.failed {
                eprintln!("失败: {}：{error}", path.display());
            }
            println!(
                "完成：{} 成功，{} 失败",
                report.cooked.len(),
                report.failed.len()
            );
            if !report.is_ok() {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
