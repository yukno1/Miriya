//! 音频导入：WAV（PCM）与 OGG（symphonia）→ f32 交错 PCM。

use std::io::Cursor;
use std::path::Path;

use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::asset::{AudioData, NativeAsset};
use crate::error::{AssetError, Result};
use crate::importer::{ImportReport, Importer};

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn parse_wav(bytes: &[u8]) -> Result<AudioData> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(AssetError::Corrupt("WAV header 无效".to_string()));
    }
    let mut position = 12;
    let mut format: Option<(u16, u32, u16)> = None;
    let mut pcm: Option<&[u8]> = None;

    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let size = read_u32(bytes, position + 4) as usize;
        let start = position + 8;
        let end = start.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " if size >= 16 && end >= start + 16 => {
                format = Some((
                    read_u16(bytes, start + 2),
                    read_u32(bytes, start + 4),
                    read_u16(bytes, start + 14),
                ));
            }
            b"data" => pcm = Some(&bytes[start..end]),
            _ => {}
        }
        position = end + (size & 1);
    }

    let (audio_format, sample_rate, bits_per_sample) =
        format.ok_or_else(|| AssetError::Corrupt("WAV 缺少 fmt chunk".to_string()))?;
    let pcm = pcm.ok_or_else(|| AssetError::Corrupt("WAV 缺少 data chunk".to_string()))?;
    if audio_format != 1 {
        return Err(AssetError::Unsupported(format!(
            "仅支持 PCM WAV（audio_format={audio_format}）"
        )));
    }

    let samples = match bits_per_sample {
        8 => pcm
            .iter()
            .map(|sample| (*sample as f32 - 128.0) / 128.0)
            .collect(),
        16 => pcm
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32768.0)
            .collect(),
        other => {
            return Err(AssetError::Unsupported(format!(
                "不支持的 WAV 位深：{other}"
            )));
        }
    };
    let channels = format.map(|(channels, _, _)| channels).unwrap_or(1);
    Ok(AudioData {
        sample_rate,
        channels,
        samples,
    })
}

fn parse_ogg(bytes: &[u8]) -> Result<AudioData> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("ogg");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| AssetError::Decode(format!("OGG 探测失败：{error}")))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| AssetError::Decode("OGG 缺少默认音轨".to_string()))?;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| AssetError::Decode("OGG 音轨缺少采样率".to_string()))?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| AssetError::Decode(format!("OGG 解码器创建失败：{error}")))?;

    let mut samples = Vec::new();
    let mut channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);
    while let Ok(packet) = format.next_packet() {
        let decoded = decoder
            .decode(&packet)
            .map_err(|error| AssetError::Decode(format!("OGG 解码失败：{error}")))?;
        match decoded {
            AudioBufferRef::F32(buffer) => {
                let spec = buffer.spec();
                channels = spec.channels.count();
                let planes = buffer.as_ref().planes();
                let planes = planes.planes();
                let frames = planes.first().map_or(0, |plane| plane.len());
                for frame in 0..frames {
                    for plane in planes {
                        samples.push(plane[frame]);
                    }
                }
            }
            AudioBufferRef::S16(buffer) => {
                let spec = buffer.spec();
                channels = spec.channels.count();
                let planes = buffer.as_ref().planes();
                let planes = planes.planes();
                let frames = planes.first().map_or(0, |plane| plane.len());
                for frame in 0..frames {
                    for plane in planes {
                        samples.push(plane[frame] as f32 / 32768.0);
                    }
                }
            }
            other => {
                let _ = other;
                return Err(AssetError::Unsupported("不支持的 OGG 采样格式".to_string()));
            }
        }
    }

    if samples.is_empty() {
        return Err(AssetError::Decode("OGG 未解码出采样".to_string()));
    }
    Ok(AudioData {
        sample_rate,
        channels: channels as u16,
        samples,
    })
}

/// WAV/OGG 导入器。
pub struct AudioImporter;

impl Importer for AudioImporter {
    fn type_tag(&self) -> &'static str {
        "audio"
    }

    fn import(&self, bytes: &[u8], path: &Path, _report: &mut ImportReport) -> Result<NativeAsset> {
        let is_ogg = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ogg"));
        let audio = if is_ogg {
            parse_ogg(bytes)?
        } else {
            parse_wav(bytes)?
        };
        Ok(NativeAsset::Audio(audio))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_validation() {
        let mut report = ImportReport::default();
        assert!(matches!(
            AudioImporter.import(b"not wav", Path::new("bad.wav"), &mut report),
            Err(AssetError::Corrupt(_))
        ));
    }

    #[test]
    fn minimal_pcm_wav_is_decoded() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&0i16.to_le_bytes());
        bytes.extend_from_slice(&16384i16.to_le_bytes());

        let mut report = ImportReport::default();
        let native = AudioImporter
            .import(&bytes, Path::new("tone.wav"), &mut report)
            .unwrap();
        let NativeAsset::Audio(audio) = native else {
            panic!("expected audio");
        };
        assert_eq!(audio.sample_rate, 8000);
        assert_eq!(audio.channels, 1);
        assert_eq!(audio.samples, vec![0.0, 0.5]);
    }
}
