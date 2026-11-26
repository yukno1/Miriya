//! Cook：校验源资产并封装为版本化 `.miasset`（M4）。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use mir_asset::AssetError;
use mir_asset::importer::{self, ImportReport};

/// 单次 cook 的结果。
#[derive(Debug, Default)]
pub struct CookReport {
    pub cooked: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

impl CookReport {
    /// 是否全部成功。
    pub fn is_ok(&self) -> bool {
        self.failed.is_empty()
    }
}

/// 把单个源资产封装为 `.miasset`。
pub fn cook_file(input: &Path, source_root: &Path, output_root: &Path) -> Result<PathBuf> {
    let bytes = fs::read(input).with_context(|| format!("读取源资产失败：{}", input.display()))?;
    let type_tag =
        importer::type_tag_from_path(input).map_err(|error| anyhow!(error.to_string()))?;
    let mut report = ImportReport {
        source: input.to_path_buf(),
        type_tag: type_tag.to_string(),
        bytes: bytes.len(),
        ..ImportReport::default()
    };
    // 先按目标类型完整解析一次，损坏源资产不进入包体。
    importer::import_by_tag(type_tag, &bytes, input, &mut report)
        .map_err(|error| anyhow!(error.to_string()))?;

    let relative = input
        .strip_prefix(source_root)
        .unwrap_or_else(|_| Path::new(input.file_name().unwrap_or_default()));
    let output = output_root.join(relative).with_extension("miasset");
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建输出目录失败：{}", parent.display()))?;
    }
    let cooked =
        mir_asset::encode(&bytes).map_err(|error: AssetError| anyhow!(error.to_string()))?;
    fs::write(&output, cooked).with_context(|| format!("写入资产失败：{}", output.display()))?;
    Ok(output)
}

/// 递归 cook 目录；每个失败资产单独记录，不中断整体。
pub fn cook_dir(source_root: &Path, output_root: &Path) -> Result<CookReport> {
    fs::create_dir_all(output_root)
        .with_context(|| format!("创建输出目录失败：{}", output_root.display()))?;
    let mut report = CookReport::default();
    let mut stack = vec![source_root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries = fs::read_dir(&directory)
            .with_context(|| format!("读取目录失败：{}", directory.display()))?;
        for entry in entries {
            let path = entry
                .with_context(|| format!("读取目录项失败：{}", directory.display()))?
                .path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("miasset"))
            {
                continue;
            }
            match cook_file(&path, source_root, output_root) {
                Ok(output) => report.cooked.push(output),
                Err(error) => report.failed.push((path, format!("{error:#}"))),
            }
        }
    }
    report.cooked.sort();
    Ok(report)
}

/// 生成 1 秒 440 Hz 的 16-bit PCM WAV（仅用于示例 fixture）。
#[cfg(test)]
fn sample_wav() -> Vec<u8> {
    let sample_rate = 8000u32;
    let samples: Vec<i16> = (0..sample_rate)
        .map(|index| {
            let phase = 2.0 * std::f64::consts::PI * 440.0 * index as f64 / sample_rate as f64;
            (phase.sin() * 12000.0) as i16
        })
        .collect();
    let data_len = samples.len() * 2;
    let mut bytes = Vec::with_capacity(44 + data_len);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "只在生成示例 fixture 时运行"]
    fn generate_demo_wav_fixture() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let output = workspace.join("assets/fixtures/tone.wav");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        fs::write(&output, sample_wav()).unwrap();
        println!("已生成 {}", output.display());
    }

    #[test]
    fn cook_dir_reports_invalid_source() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = workspace.join("target/cook-test-source");
        let output = workspace.join("target/cook-test-output");
        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&output);
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("bad.wav"), b"not wav").unwrap();
        let report = cook_dir(&source, &output).unwrap();
        assert!(!report.is_ok());
        assert_eq!(report.failed.len(), 1);
    }
}
