use std::path::Path;

use s2v_core::{AudioConfig, Cast, SceneConfig};

use crate::resampler::resample_mono;
use crate::reverb::IrCache;

/// room_size/reverb_wet の実効値を決定する (Cast > Scene > AudioConfig の優先順)。
/// Python版 audio_processor.py:81-88 (`c_room_size if ... else (s_room_size if ... else self.default_room_size)`) 相当。
fn resolve_reverb_params(cast: &Cast, scene: &SceneConfig, default_room_size: f64, default_reverb_wet: f64) -> (f64, f64) {
    let room_size = cast.params.get("room_size").and_then(|v| v.as_f64())
        .or(scene.room_size)
        .unwrap_or(default_room_size);
    let reverb_wet = cast.params.get("reverb_wet").and_then(|v| v.as_f64())
        .or(scene.reverb_wet)
        .unwrap_or(default_reverb_wet);
    (room_size, reverb_wet)
}

pub struct AudioProcessor {
    config: AudioConfig,
    ir_cache: IrCache,
}

impl AudioProcessor {
    pub fn new(config: AudioConfig) -> Self {
        let sample_rate = config.sample_rate;
        Self {
            config,
            ir_cache: IrCache::new(sample_rate),
        }
    }

    pub fn config_sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    pub fn config_room_size(&self) -> f64 {
        self.config.room_size
    }

    pub fn prewarm_ir_cache(&self, room_sizes: &[f64]) {
        self.ir_cache.prewarm(room_sizes);
    }

    /// WAV ファイルを読み込み、DSP 処理を施して stereo WAV として書き出す。
    /// 戻り値: 出力サンプル数（失敗時は Err）
    pub fn process(&self, input: &Path, output: &Path, cast: &Cast, scene: &SceneConfig) -> anyhow::Result<usize> {
        // --- パラメータ決定 (Cast > Scene > AudioConfig デフォルト) ---
        let (room_size, reverb_wet) = resolve_reverb_params(cast, scene, self.config.room_size, self.config.reverb_wet);

        self.ir_cache.compute_if_needed(room_size);

        // --- WAV 読み込み ---
        let mut reader = hound::WavReader::open(input)?;
        let spec = reader.spec();
        let samples_raw: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => {
                let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
                reader.samples::<i32>().map(|s| s.unwrap() as f32 / max).collect()
            }
            hound::SampleFormat::Float => {
                reader.samples::<f32>().map(|s| s.unwrap()).collect()
            }
        };

        // チャンネル 1 へ変換 (ステレオの場合は L ch のみ使用)
        let mono: Vec<f32> = if spec.channels == 1 {
            samples_raw
        } else {
            samples_raw.into_iter().step_by(spec.channels as usize).collect()
        };

        // ピーク正規化
        let peak = mono.iter().cloned().map(f32::abs).fold(0.0_f32, f32::max);
        let mono: Vec<f32> = if peak > 0.0 {
            mono.into_iter().map(|s| s / peak).collect()
        } else {
            mono
        };

        // リサンプリング
        let mono = resample_mono(&mono, spec.sample_rate, self.config.sample_rate)?;

        // --- 幾何学計算 ---
        let pan_rad = cast.pan.to_radians();
        let geo = self.calc_geometry(cast.distance, pan_rad);

        // 空気吸収フィルター
        let data_l = apply_air_absorption(&mono, geo.dist_l, self.config.sample_rate, self.config.air_absorption_coeff);
        let data_r = apply_air_absorption(&mono, geo.dist_r, self.config.sample_rate, self.config.air_absorption_coeff);

        // ITD
        let delay_l = ((geo.dist_l / self.config.sound_speed) * self.config.sample_rate as f64) as usize;
        let delay_r = ((geo.dist_r / self.config.sound_speed) * self.config.sample_rate as f64) as usize;
        let min_delay = delay_l.min(delay_r);
        let rel_l = delay_l - min_delay;
        let rel_r = delay_r - min_delay;

        // ILD + 基準音圧ゲイン
        let mic_angle_rad = self.config.mic_angle.to_radians();
        let k = self.config.mic_directivity;
        let nominal_pat = (1.0 - k) + k * (0.0_f64 - mic_angle_rad).cos();
        let max_gain_linear = 10.0_f64.powf(self.config.max_gain_db / 20.0);
        let base_norm = max_gain_linear / nominal_pat.max(1e-6);

        let engine_vol = self.config.engine_volume_offsets
            .get(&cast.engine_type).copied().unwrap_or(1.0);
        // Python版 (core/audio_processor.py) は config.REFERENCE_GAIN_DB を
        // 定義しているがゲイン計算には使用していない (未使用の設定値)。
        // 移植時に誤って乗算していたため、Python版に合わせて除外する。
        let vol_factor = base_norm * cast.volume * engine_vol;

        let dist_gain_l = self.config.reference_dist / geo.dist_l.max(0.1);
        let dist_gain_r = self.config.reference_dist / geo.dist_r.max(0.1);
        let pat_l = ((1.0 - k) + k * (geo.angle_l - mic_angle_rad).cos()).max(0.01);
        let pat_r = ((1.0 - k) + k * (geo.angle_r + mic_angle_rad).cos()).max(0.01);

        let gain_l = (vol_factor * dist_gain_l * pat_l) as f32;
        let gain_r = (vol_factor * dist_gain_r * pat_r) as f32;

        // --- ステレオバッファ構築 ---
        let rv_time = 0.05 + room_size * 3.0;
        let rv_samples = if reverb_wet > 0.0 { (self.config.sample_rate as f64 * rv_time) as usize } else { 0 };
        let out_len = mono.len() + rel_l.max(rel_r) + rv_samples;
        let mut stereo: Vec<[f32; 2]> = vec![[0.0, 0.0]; out_len];

        for (i, (&sl, &sr)) in data_l.iter().zip(data_r.iter()).enumerate() {
            stereo[rel_l + i][0] = sl * gain_l;
            stereo[rel_r + i][1] = sr * gain_r;
        }

        // リバーブ
        self.ir_cache.apply(&mut stereo, room_size, reverb_wet, cast.distance);

        // リミッター
        let peak_out = stereo.iter().flat_map(|s| s.iter()).cloned().map(f32::abs).fold(0.0_f32, f32::max);
        if peak_out > max_gain_linear as f32 {
            let scale = max_gain_linear as f32 / peak_out;
            stereo.iter_mut().for_each(|s| { s[0] *= scale; s[1] *= scale; });
        }

        // --- WAV 書き出し ---
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let out_spec = hound::WavSpec {
            channels: 2,
            sample_rate: self.config.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(output, out_spec)?;
        for frame in &stereo {
            writer.write_sample((frame[0] * 32767.0) as i16)?;
            writer.write_sample((frame[1] * 32767.0) as i16)?;
        }
        writer.finalize()?;
        Ok(stereo.len())
    }

    fn calc_geometry(&self, distance: f64, pan_rad: f64) -> GeoParams {
        let d_h = self.config.microphone_spacing / 2.0;
        let sx = distance * pan_rad.sin();
        let sy = distance * pan_rad.cos();
        let dist_l = ((sx + d_h).powi(2) + sy.powi(2)).sqrt();
        let dist_r = ((sx - d_h).powi(2) + sy.powi(2)).sqrt();
        let angle_l = (sx + d_h).atan2(sy);
        let angle_r = (sx - d_h).atan2(sy);
        GeoParams { dist_l, dist_r, angle_l, angle_r }
    }
}

struct GeoParams {
    dist_l: f64,
    dist_r: f64,
    angle_l: f64,
    angle_r: f64,
}

/// 簡易一次 IIR ローパスで空気吸収をシミュレート
fn apply_air_absorption(samples: &[f32], dist: f64, sample_rate: u32, air_coeff: f64) -> Vec<f32> {
    if air_coeff <= 0.0 {
        return samples.to_vec();
    }
    let nyquist = sample_rate as f64 / 2.0;
    let cutoff = (nyquist / (1.0 + air_coeff * dist)).min(nyquist - 100.0);
    // 一次 IIR ローパス: alpha = cutoff / (cutoff + fs/2π) 近似
    let rc = 1.0 / (2.0 * std::f64::consts::PI * cutoff);
    let dt = 1.0 / sample_rate as f64;
    let alpha = (dt / (rc + dt)) as f32;
    let mut out = vec![0.0_f32; samples.len()];
    out[0] = alpha * samples[0];
    for i in 1..samples.len() {
        out[i] = out[i - 1] + alpha * (samples[i] - out[i - 1]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn default_audio_config() -> AudioConfig {
        AudioConfig {
            sample_rate: 48000,
            microphone_spacing: 0.2,
            sound_speed: 340.0,
            air_absorption_coeff: 0.05,
            room_size: 0.1,
            reverb_wet: 0.3,
            reference_dist: 1.0,
            reference_gain_db: -5.0,
            max_gain_db: -1.0,
            mic_directivity: 0.5,
            mic_angle: 45.0,
            engine_volume_offsets: {
                let mut m = HashMap::new();
                m.insert("voicevox".to_string(), 1.0);
                m
            },
        }
    }

    fn dummy_cast(pan: f64, distance: f64) -> Cast {
        Cast {
            name: "テスト".to_string(),
            speaker_name: "ずんだもん".to_string(),
            engine_type: "voicevox".to_string(),
            pan,
            distance,
            volume: 1.0,
            params: HashMap::new(),
        }
    }

    fn default_scene() -> SceneConfig {
        SceneConfig { name: "テスト".to_string(), room_size: Some(0.1), reverb_wet: Some(0.3) }
    }

    #[test]
    fn resolve_reverb_params_falls_back_to_audio_config_when_scene_omits() {
        // s2v-57z: シーン側でroom_size/reverb_wetが省略された場合、AudioConfigの値にフォールバックする
        // (Python版 audio_processor.py:87-88 self.default_room_size/self.default_base_wet 相当)
        let cast = dummy_cast(0.0, 1.0);
        let scene = SceneConfig { name: "テスト".to_string(), room_size: None, reverb_wet: None };
        let (room_size, reverb_wet) = resolve_reverb_params(&cast, &scene, 0.42, 0.55);
        assert!((room_size - 0.42).abs() < 1e-10, "config値にフォールバックするはず, got {room_size}");
        assert!((reverb_wet - 0.55).abs() < 1e-10, "config値にフォールバックするはず, got {reverb_wet}");
    }

    #[test]
    fn resolve_reverb_params_prefers_scene_over_config_default() {
        let cast = dummy_cast(0.0, 1.0);
        let scene = SceneConfig { name: "テスト".to_string(), room_size: Some(0.8), reverb_wet: Some(0.2) };
        let (room_size, reverb_wet) = resolve_reverb_params(&cast, &scene, 0.42, 0.55);
        assert!((room_size - 0.8).abs() < 1e-10);
        assert!((reverb_wet - 0.2).abs() < 1e-10);
    }

    #[test]
    fn resolve_reverb_params_prefers_cast_over_scene_and_config() {
        let mut cast = dummy_cast(0.0, 1.0);
        cast.params.insert("room_size".to_string(), serde_json::json!(0.9));
        cast.params.insert("reverb_wet".to_string(), serde_json::json!(0.1));
        let scene = SceneConfig { name: "テスト".to_string(), room_size: Some(0.8), reverb_wet: Some(0.2) };
        let (room_size, reverb_wet) = resolve_reverb_params(&cast, &scene, 0.42, 0.55);
        assert!((room_size - 0.9).abs() < 1e-10);
        assert!((reverb_wet - 0.1).abs() < 1e-10);
    }

    fn write_test_wav(path: &Path, sample_rate: u32, freq: f32, duration_s: f32) {
        let n = (sample_rate as f32 * duration_s) as usize;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..n {
            let v = (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin();
            writer.write_sample((v * 32767.0) as i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn reference_gain_db_does_not_affect_output_level() {
        // Python版 (core/audio_processor.py) は config.REFERENCE_GAIN_DB を
        // 定義しているが、ゲイン計算には一切使用していない（未使用の設定値）。
        // Rust版が独自に reference_gain_db を音量係数へ乗じてしまうと、
        // Python版に対して出力音量が変化してしまう（移植バグ）。
        // よって reference_gain_db を変えても出力レベルは変わらないはず。
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.wav");
        write_test_wav(&input, 48000, 440.0, 0.1);

        let mut cfg_a = default_audio_config();
        cfg_a.reference_gain_db = -5.0;
        let mut cfg_b = default_audio_config();
        cfg_b.reference_gain_db = -20.0;

        let peak_for = |cfg: AudioConfig| -> f32 {
            let out = dir.path().join(format!("out_{}.wav", cfg.reference_gain_db));
            let proc = AudioProcessor::new(cfg);
            proc.process(&input, &out, &dummy_cast(0.0, 1.0), &default_scene()).unwrap();
            let mut r = hound::WavReader::open(&out).unwrap();
            r.samples::<i16>().map(|s| (s.unwrap() as f32).abs()).fold(0.0_f32, f32::max)
        };

        let peak_a = peak_for(cfg_a);
        let peak_b = peak_for(cfg_b);

        assert!(
            (peak_a - peak_b).abs() / peak_a.max(1.0) < 0.01,
            "reference_gain_db should not change output level (Python版に未使用): peak_a={peak_a}, peak_b={peak_b}"
        );
    }

    #[test]
    fn process_creates_stereo_wav() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.wav");
        let output = dir.path().join("out.wav");

        write_test_wav(&input, 24000, 440.0, 0.1);

        let proc = AudioProcessor::new(default_audio_config());
        let n = proc.process(&input, &output, &dummy_cast(0.0, 1.0), &default_scene()).unwrap();
        assert!(n > 0);
        assert!(output.exists());

        let reader = hound::WavReader::open(&output).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, 48000);
    }

    #[test]
    fn process_panning_creates_stereo_asymmetry() {
        // XYクロスペアマイク配置: Lマイクは+45°(右向き)、Rマイクは-45°(左向き)。
        // 左パン(pan=-45)ではRマイクが音源を強く拾いRchが大きくなり、
        // 右パン(pan=+45)ではLマイクが拾いLchが大きくなる。
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.wav");
        write_test_wav(&input, 48000, 440.0, 0.2);

        let proc = AudioProcessor::new(default_audio_config());

        let read_rms = |p: &Path, ch: usize| -> f64 {
            let mut r = hound::WavReader::open(p).unwrap();
            let samples: Vec<i16> = r.samples().map(|s| s.unwrap()).collect();
            let ch_samples: Vec<f64> = samples.iter().skip(ch).step_by(2).map(|&s| s as f64).collect();
            let sum_sq: f64 = ch_samples.iter().map(|s| s * s).sum();
            (sum_sq / ch_samples.len() as f64).sqrt()
        };

        // 中央: L≈R
        let out_center = dir.path().join("center.wav");
        proc.process(&input, &out_center, &dummy_cast(0.0, 1.0), &default_scene()).unwrap();
        let l_center = read_rms(&out_center, 0);
        let r_center = read_rms(&out_center, 1);
        let center_ratio = (l_center - r_center).abs() / (l_center + r_center + 1.0);
        assert!(center_ratio < 0.1, "center should be approximately symmetric, got ratio={center_ratio:.3}");

        // 左パン: L ≠ R (非対称が生まれる)
        let out_left = dir.path().join("left.wav");
        proc.process(&input, &out_left, &dummy_cast(-45.0, 1.0), &default_scene()).unwrap();
        let l_left = read_rms(&out_left, 0);
        let r_left = read_rms(&out_left, 1);
        let left_ratio = (l_left - r_left).abs() / (l_left + r_left + 1.0);
        assert!(left_ratio > 0.05, "left pan should create stereo asymmetry, got ratio={left_ratio:.3}");

        // 右パン: 左パンと逆方向の非対称
        let out_right = dir.path().join("right.wav");
        proc.process(&input, &out_right, &dummy_cast(45.0, 1.0), &default_scene()).unwrap();
        let l_right = read_rms(&out_right, 0);
        let r_right = read_rms(&out_right, 1);
        // 左パンと右パンで大小関係が逆転する
        let left_dominant = l_left > r_left;
        let right_dominant = l_right > r_right;
        assert_ne!(left_dominant, right_dominant,
            "left and right pan should create opposite stereo balance");
    }

    #[test]
    fn prewarm_then_process_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.wav");
        let output = dir.path().join("out.wav");
        write_test_wav(&input, 48000, 880.0, 0.1);

        let proc = AudioProcessor::new(default_audio_config());
        proc.prewarm_ir_cache(&[0.1, 0.3, 0.8]);
        let n = proc.process(&input, &output, &dummy_cast(30.0, 2.0), &default_scene()).unwrap();
        assert!(n > 0);
    }

    #[test]
    fn calc_geometry_symmetric_at_center() {
        let proc = AudioProcessor::new(default_audio_config());
        let geo = proc.calc_geometry(1.0, 0.0);
        // pan=0 のとき dist_l と dist_r は対称
        assert!((geo.dist_l - geo.dist_r).abs() < 1e-10);
    }

    #[test]
    fn apply_air_absorption_preserves_length() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.001).sin()).collect();
        let out = apply_air_absorption(&samples, 2.0, 48000, 0.05);
        assert_eq!(out.len(), samples.len());
    }
}
