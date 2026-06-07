use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub voicevox: EngineUrl,
    pub aivis: EngineUrl,
    pub xtts: EngineUrl,
    pub audio: AudioConfig,
    pub concurrency: ConcurrencyConfig,
    pub bgm: BgmConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineUrl {
    pub url: String,
    /// 未起動時に自動起動する実行ファイルのパス（省略可）
    #[serde(default)]
    pub exe_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub microphone_spacing: f64,
    pub sound_speed: f64,
    pub air_absorption_coeff: f64,
    pub room_size: f64,
    pub reverb_wet: f64,
    pub reference_dist: f64,
    pub reference_gain_db: f64,
    pub max_gain_db: f64,
    pub mic_directivity: f64,
    pub mic_angle: f64,
    pub engine_volume_offsets: HashMap<String, f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConcurrencyConfig {
    pub voicevox: usize,
    pub aivis: usize,
    pub xtts: usize,
    pub audio_process: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BgmConfig {
    pub crossfade_s: f64,
    pub se_fade_out_s: f64,
}

impl Config {
    pub fn from_toml(s: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(s)?)
    }

    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        Self::from_toml(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TOML: &str = r#"
[voicevox]
url = "http://127.0.0.1:50021"

[aivis]
url = "http://127.0.0.1:10101"

[xtts]
url = "http://localhost:8020"

[audio]
sample_rate = 48000
microphone_spacing = 0.2
sound_speed = 340.0
air_absorption_coeff = 0.05
room_size = 0.1
reverb_wet = 0.7
reference_dist = 1.0
reference_gain_db = -5.0
max_gain_db = -1.0
mic_directivity = 0.5
mic_angle = 45.0

[audio.engine_volume_offsets]
voicevox = 1.2
aivis = 0.9
xtts = 1.0

[concurrency]
voicevox = 3
aivis = 3
xtts = 2
audio_process = 0

[bgm]
crossfade_s = 3.0
se_fade_out_s = 0.05
"#;

    #[test]
    fn parses_engine_urls() {
        let cfg = Config::from_toml(SAMPLE_TOML).unwrap();
        assert_eq!(cfg.voicevox.url, "http://127.0.0.1:50021");
        assert_eq!(cfg.aivis.url, "http://127.0.0.1:10101");
        assert_eq!(cfg.xtts.url, "http://localhost:8020");
    }

    #[test]
    fn exe_path_is_optional_and_defaults_to_none() {
        let cfg = Config::from_toml(SAMPLE_TOML).unwrap();
        assert_eq!(cfg.voicevox.exe_path, None);
    }

    #[test]
    fn parses_exe_path_when_present() {
        let toml_with_exe = SAMPLE_TOML.replacen(
            r#"url = "http://127.0.0.1:50021""#,
            "url = \"http://127.0.0.1:50021\"\nexe_path = \"C:\\\\VOICEVOX\\\\run.exe\"",
            1,
        );
        let cfg = Config::from_toml(&toml_with_exe).unwrap();
        assert_eq!(cfg.voicevox.exe_path.as_deref(), Some("C:\\VOICEVOX\\run.exe"));
    }

    #[test]
    fn parses_audio_config() {
        let cfg = Config::from_toml(SAMPLE_TOML).unwrap();
        assert_eq!(cfg.audio.sample_rate, 48000);
        assert!((cfg.audio.microphone_spacing - 0.2).abs() < 1e-10);
        assert!((cfg.audio.sound_speed - 340.0).abs() < 1e-10);
        assert!((cfg.audio.reverb_wet - 0.7).abs() < 1e-10);
        assert!((cfg.audio.reference_gain_db - (-5.0)).abs() < 1e-10);
        assert!((cfg.audio.max_gain_db - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn parses_engine_volume_offsets() {
        let cfg = Config::from_toml(SAMPLE_TOML).unwrap();
        assert!((cfg.audio.engine_volume_offsets["voicevox"] - 1.2).abs() < 1e-10);
        assert!((cfg.audio.engine_volume_offsets["aivis"] - 0.9).abs() < 1e-10);
        assert!((cfg.audio.engine_volume_offsets["xtts"] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn parses_concurrency_config() {
        let cfg = Config::from_toml(SAMPLE_TOML).unwrap();
        assert_eq!(cfg.concurrency.voicevox, 3);
        assert_eq!(cfg.concurrency.aivis, 3);
        assert_eq!(cfg.concurrency.xtts, 2);
        assert_eq!(cfg.concurrency.audio_process, 0);
    }

    #[test]
    fn parses_bgm_config() {
        let cfg = Config::from_toml(SAMPLE_TOML).unwrap();
        assert!((cfg.bgm.crossfade_s - 3.0).abs() < 1e-10);
        assert!((cfg.bgm.se_fade_out_s - 0.05).abs() < 1e-10);
    }

    #[test]
    fn rejects_invalid_toml() {
        let result = Config::from_toml("this is not valid toml [[[");
        assert!(result.is_err());
    }
}
