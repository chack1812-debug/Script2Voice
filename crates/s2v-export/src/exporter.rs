use std::path::{Path, PathBuf};

use s2v_core::{BgmConfig, EventType, TimelineEvent};
use tracing::{info, warn};

pub struct Exporter<'a> {
    events: &'a [TimelineEvent],
    output_dir: PathBuf,
    sample_rate: u32,
    bgm_config: BgmConfig,
}

impl<'a> Exporter<'a> {
    pub fn new(
        events: &'a [TimelineEvent],
        output_dir: impl Into<PathBuf>,
        sample_rate: u32,
        bgm_config: BgmConfig,
    ) -> Self {
        Self {
            events,
            output_dir: output_dir.into(),
            sample_rate,
            bgm_config,
        }
    }

    pub fn generate_srt(&self) -> anyhow::Result<()> {
        let dir = self.output_dir.join("timeline");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("subtitles.srt");

        let audio_events: Vec<_> = self.events.iter()
            .filter(|e| e.event_type == EventType::Audio)
            .collect();

        let mut content = String::new();
        for (i, event) in audio_events.iter().enumerate() {
            let start_s = event.start_ms / 1000.0;
            let end_s = (event.start_ms + event.duration_ms) / 1000.0;
            content.push_str(&format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                format_srt_time(start_s),
                format_srt_time(end_s),
                event.display_text.as_deref().unwrap_or(""),
            ));
        }

        std::fs::write(&path, &content)?;
        info!("SRT exported to: {}", path.display());
        Ok(())
    }

    pub fn generate_fcpxml(&self) -> anyhow::Result<()> {
        let dir = self.output_dir.join("timeline");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("timeline.fcpxml");

        let total_s = self.events.iter()
            .map(|e| (e.start_ms + e.duration_ms) / 1000.0)
            .fold(0.0_f64, f64::max);
        let total_ticks = (total_s * 30000.0) as u64;

        let resources = self.build_resource_tags();
        let clips = self.build_audio_clips();

        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE fcpxml>
<fcpxml version="1.8">
    <resources>
        <format id="r1" name="FFVideoFormat1080p2997" frameDuration="1001/30000s"/>
        {resources}
    </resources>
    <library>
        <event name="VoiceProduction">
            <project name="ScriptTimeline">
                <sequence format="r1" duration="{total_ticks}/30000s">
                    <spine>
                        <gap name="Gap" offset="0s" duration="{total_ticks}/30000s">
                            {clips}
                        </gap>
                    </spine>
                </sequence>
            </project>
        </event>
    </library>
</fcpxml>"#
        );

        std::fs::write(&path, xml.as_bytes())?;
        info!("FCPXML exported to: {}", path.display());
        Ok(())
    }

    pub fn generate_combined_audio(&self) -> anyhow::Result<()> {
        let out_path = self.output_dir.join("full_dialogue.wav");
        let sr = self.sample_rate;

        // dialogue クリップを収集
        let audio_events: Vec<_> = self.events.iter()
            .filter(|e| e.event_type == EventType::Audio)
            .filter(|e| e.path.as_ref().map(|p| p.exists()).unwrap_or(false))
            .collect();

        if audio_events.is_empty() {
            warn!("ミックス対象の音声が存在しません。");
            return Ok(());
        }

        // 総サンプル数を算出
        let mut total_samples: usize = 0;
        let mut clips: Vec<(usize, Vec<[f32; 2]>)> = Vec::new();

        for event in &audio_events {
            let path = event.path.as_ref().unwrap();
            let start = (event.start_ms / 1000.0 * sr as f64) as usize;
            match read_stereo_float(path, sr) {
                Ok(samples) => {
                    total_samples = total_samples.max(start + samples.len());
                    clips.push((start, samples));
                }
                Err(e) => warn!("読み込みスキップ: {} ({e})", path.display()),
            }
        }

        if total_samples == 0 {
            warn!("有効なサンプルがありません。");
            return Ok(());
        }

        // float32 バッファに加算ミックス
        let mut buf: Vec<[f32; 2]> = vec![[0.0, 0.0]; total_samples];
        for (start, samples) in clips {
            for (i, s) in samples.iter().enumerate() {
                if start + i < buf.len() {
                    buf[start + i][0] += s[0];
                    buf[start + i][1] += s[1];
                }
            }
        }

        // クリッピング防止
        let peak = buf.iter().flat_map(|s| s.iter()).cloned().map(f32::abs).fold(0.0_f32, f32::max);
        if peak > 1.0 {
            buf.iter_mut().for_each(|s| { s[0] /= peak; s[1] /= peak; });
        }

        // WAV 書き出し
        std::fs::create_dir_all(out_path.parent().unwrap_or(Path::new(".")))?;
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: sr,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&out_path, spec)?;
        for frame in &buf {
            writer.write_sample((frame[0] * 32767.0) as i16)?;
            writer.write_sample((frame[1] * 32767.0) as i16)?;
        }
        writer.finalize()?;
        info!("ミックス音声を出力しました: {}", out_path.display());
        Ok(())
    }

    fn build_resource_tags(&self) -> String {
        self.events.iter().enumerate()
            .filter_map(|(i, e)| {
                let p = e.path.as_ref()?;
                let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                let uri = format!("file://{}", abs.display()).replace('\\', "/");
                let name = p.file_name()?.to_string_lossy();
                Some(format!(r#"<asset id="a{i}" name="{name}" src="{uri}"/>"#))
            })
            .collect::<Vec<_>>()
            .join("\n        ")
    }

    fn build_audio_clips(&self) -> String {
        self.events.iter().enumerate()
            .filter_map(|(i, e)| match e.event_type {
                EventType::Audio => {
                    let start = (e.start_ms / 1000.0 * 30000.0) as u64;
                    let dur = (e.duration_ms / 1000.0 * 30000.0) as u64;
                    Some(format!(
                        r#"<audio ref="a{i}" lane="{}" offset="{start}/30000s" duration="{dur}/30000s" role="dialogue"/>"#,
                        i + 1
                    ))
                }
                EventType::BgmStart => {
                    let start = (e.start_ms / 1000.0 * 30000.0) as u64;
                    let dur = (self.bgm_config.crossfade_s * 30000.0) as u64;
                    Some(format!(
                        r#"<audio ref="a{i}" lane="{}" offset="{start}/30000s" duration="{dur}/30000s" role="music"/>"#,
                        i + 1
                    ))
                }
                EventType::Se => {
                    let start = (e.start_ms / 1000.0 * 30000.0) as u64;
                    let dur = 1500_u64; // SE デフォルト 0.05s
                    Some(format!(
                        r#"<audio ref="a{i}" lane="{}" offset="{start}/30000s" duration="{dur}/30000s" role="effects"/>"#,
                        i + 1
                    ))
                }
                EventType::BgmStop => None,
            })
            .collect::<Vec<_>>()
            .join("\n                            ")
    }
}

fn format_srt_time(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0) as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s = (total_ms % 60_000) / 1_000;
    let ms = total_ms % 1_000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

fn read_stereo_float(path: &Path, target_sr: u32) -> anyhow::Result<Vec<[f32; 2]>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.unwrap() as f32 / max).collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
    };
    let stereo: Vec<[f32; 2]> = if spec.channels == 1 {
        raw.iter().map(|&s| [s, s]).collect()
    } else if spec.channels == 2 {
        raw.chunks(2).map(|c| [c[0], c[1]]).collect()
    } else {
        raw.chunks(spec.channels as usize).map(|c| [c[0], c[1]]).collect()
    };
    // 簡易リサンプリング (同レートの場合はそのまま)
    if spec.sample_rate != target_sr {
        let ratio = target_sr as f64 / spec.sample_rate as f64;
        let new_len = (stereo.len() as f64 * ratio) as usize;
        Ok((0..new_len).map(|i| {
            let src_idx = (i as f64 / ratio) as usize;
            stereo[src_idx.min(stereo.len() - 1)]
        }).collect())
    } else {
        Ok(stereo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use s2v_core::EventType;

    fn make_audio_event(start_ms: f64, duration_ms: f64, text: &str, path: Option<PathBuf>) -> TimelineEvent {
        TimelineEvent {
            event_type: EventType::Audio,
            start_ms,
            duration_ms,
            path,
            text: Some(text.to_string()),
            display_text: Some(text.to_string()),
            cast: Some("テスト".to_string()),
        }
    }

    fn default_bgm() -> BgmConfig {
        BgmConfig { crossfade_s: 3.0, se_fade_out_s: 0.05 }
    }

    fn write_wav(path: &Path, sr: u32, seconds: f32) {
        let spec = hound::WavSpec {
            channels: 2, sample_rate: sr, bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let n = (sr as f32 * seconds) as usize;
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..n {
            let v = ((i as f32 / sr as f32) * 440.0 * 2.0 * std::f32::consts::PI).sin();
            w.write_sample((v * 8000.0) as i16).unwrap();
            w.write_sample((v * 8000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn srt_generates_correct_format() {
        let events = vec![
            make_audio_event(0.0, 1500.0, "こんにちは", None),
            make_audio_event(2000.0, 800.0, "さようなら", None),
        ];
        let dir = tempfile::tempdir().unwrap();
        let exp = Exporter::new(&events, dir.path(), 48000, default_bgm());
        exp.generate_srt().unwrap();

        let content = std::fs::read_to_string(dir.path().join("timeline/subtitles.srt")).unwrap();
        assert!(content.contains("1\n"));
        assert!(content.contains("00:00:00,000 --> 00:00:01,500"));
        assert!(content.contains("こんにちは"));
        assert!(content.contains("2\n"));
        assert!(content.contains("00:00:02,000 --> 00:00:02,800"));
        assert!(content.contains("さようなら"));
    }

    #[test]
    fn srt_time_format() {
        assert_eq!(format_srt_time(0.0), "00:00:00,000");
        assert_eq!(format_srt_time(1.5), "00:00:01,500");
        assert_eq!(format_srt_time(3661.1), "01:01:01,100");
        assert_eq!(format_srt_time(3600.0), "01:00:00,000");
    }

    #[test]
    fn fcpxml_generates_valid_structure() {
        let events = vec![
            make_audio_event(0.0, 2000.0, "テスト", None),
        ];
        let dir = tempfile::tempdir().unwrap();
        let exp = Exporter::new(&events, dir.path(), 48000, default_bgm());
        exp.generate_fcpxml().unwrap();

        let content = std::fs::read_to_string(dir.path().join("timeline/timeline.fcpxml")).unwrap();
        assert!(content.contains(r#"<fcpxml version="1.8">"#));
        assert!(content.contains("<resources>"));
        assert!(content.contains("<library>"));
        assert!(content.contains("dialogue"));
    }

    #[test]
    fn combined_audio_produces_wav() {
        let dir = tempfile::tempdir().unwrap();
        let wav1 = dir.path().join("a1.wav");
        let wav2 = dir.path().join("a2.wav");
        write_wav(&wav1, 48000, 0.1);
        write_wav(&wav2, 48000, 0.1);

        let events = vec![
            make_audio_event(0.0, 100.0, "A", Some(wav1)),
            make_audio_event(200.0, 100.0, "B", Some(wav2)),
        ];
        let out_dir = dir.path().join("out");
        let exp = Exporter::new(&events, &out_dir, 48000, default_bgm());
        exp.generate_combined_audio().unwrap();

        let out = out_dir.join("full_dialogue.wav");
        assert!(out.exists());
        let reader = hound::WavReader::open(&out).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, 48000);
        assert!(reader.spec().bits_per_sample == 16);
    }

    #[test]
    fn combined_audio_mix_is_louder_at_clip_start() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("a.wav");
        write_wav(&wav, 48000, 0.2);

        let events = vec![
            make_audio_event(500.0, 200.0, "A", Some(wav)),
        ];
        let out_dir = dir.path().join("out");
        let exp = Exporter::new(&events, &out_dir, 48000, default_bgm());
        exp.generate_combined_audio().unwrap();

        let out = out_dir.join("full_dialogue.wav");
        let mut reader = hound::WavReader::open(&out).unwrap();
        let all: Vec<i16> = reader.samples().map(|s| s.unwrap()).collect();

        let silence_end = (0.5 * 48000.0 * 2.0) as usize; // 0.5s offset
        let silence_max = all[..silence_end].iter().map(|&s| s.unsigned_abs()).max().unwrap_or(0);
        let sound_start = silence_end;
        let sound_max = all[sound_start..].iter().map(|&s| s.unsigned_abs()).max().unwrap_or(0);
        assert!(silence_max == 0, "silence section should be zero");
        assert!(sound_max > 0, "sound section should be non-zero");
    }

    #[test]
    fn empty_audio_events_skips_wav_creation() {
        let dir = tempfile::tempdir().unwrap();
        let events = vec![
            make_audio_event(0.0, 1000.0, "テスト", None), // path=None
        ];
        let out_dir = dir.path().join("out");
        let exp = Exporter::new(&events, &out_dir, 48000, default_bgm());
        exp.generate_combined_audio().unwrap();
        assert!(!out_dir.join("full_dialogue.wav").exists());
    }
}
