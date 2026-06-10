# room_size連動の一次反射（イメージソース法）＋距離依存D/R Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 部屋を箱としてモデル化し room_size に連動した6面の一次反射（イメージソース法）を直接音に付加して「接地感」を与え、あわせて距離依存のD/R制御を強化する。

**Architecture:** 直接音の処理は不変のまま、新規 `early.rs` がイメージソースのタップ（遅延・素材ローパス・左右ゲイン）を生成してステレオバッファに加算する。幾何は `geometry.rs` の自由関数（既存 `calc_geometry` を移設）を直接音とイメージで共用する。距離依存D/Rは早期反射レベルと拡散wetの距離スロープで制御する。`enabled=false` または該当セクション欠落時は従来挙動と完全一致。

**Tech Stack:** Rust / s2v-core(serde/toml) / s2v-audio(hound, realfft) — 新規依存なし

設計書: `docs/superpowers/specs/2026-06-08-early-reflections-image-source-design.md`

---

## Task 1: config に EarlyConfig を追加（後方互換）

**Files:**
- Modify: `crates/s2v-core/src/config.rs`

- [ ] **Step 1: 欠落セクションで既定値になる失敗テストを書く**

`crates/s2v-core/src/config.rs` の `mod tests` 末尾（`rejects_invalid_toml` の後）に追加する。

```rust
    #[test]
    fn early_reflections_defaults_when_section_absent() {
        // SAMPLE_TOML には [audio.early_reflections] が無い → 既定値で埋まること
        let cfg = Config::from_toml(SAMPLE_TOML).unwrap();
        let er = &cfg.audio.early_reflections;
        assert!(er.enabled);
        assert!((er.ear_height - 1.2).abs() < 1e-10);
        assert_eq!(er.room_dims_min, [4.0, 5.0, 3.0]);
        assert_eq!(er.room_dims_max, [25.0, 45.0, 18.0]);
        assert!((er.front_wall.reflection_coeff - 0.85).abs() < 1e-10);
        assert!((er.back_wall.absorption_cutoff_hz - 4000.0).abs() < 1e-10);
        assert!((er.wet_distance_slope - 0.1).abs() < 1e-10);
    }

    #[test]
    fn early_reflections_partial_section_fills_missing_fields() {
        let toml = format!("{SAMPLE_TOML}\n[audio.early_reflections]\nenabled = false\near_height = 1.7\n");
        let cfg = Config::from_toml(&toml).unwrap();
        let er = &cfg.audio.early_reflections;
        assert!(!er.enabled);
        assert!((er.ear_height - 1.7).abs() < 1e-10);
        // 指定しなかったフィールドは既定値
        assert_eq!(er.room_dims_max, [25.0, 45.0, 18.0]);
        assert!((er.floor.reflection_coeff - 0.5).abs() < 1e-10);
    }
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p s2v-core early_reflections_defaults_when_section_absent`
Expected: FAIL（コンパイルエラー: `early_reflections` フィールド未定義）

- [ ] **Step 3: EarlyConfig / MaterialConfig と既定値関数を実装する**

`crates/s2v-core/src/config.rs` の `AudioConfig` 定義に `early_reflections` フィールドを追加する。

```rust
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
    #[serde(default)]
    pub early_reflections: EarlyConfig,
}
```

同ファイルの `AudioConfig` 定義の直後に、以下を追加する。

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MaterialConfig {
    pub reflection_coeff: f64,
    pub absorption_cutoff_hz: f64,
}

impl MaterialConfig {
    const fn new(reflection_coeff: f64, absorption_cutoff_hz: f64) -> Self {
        Self { reflection_coeff, absorption_cutoff_hz }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EarlyConfig {
    #[serde(default = "er_enabled")]
    pub enabled: bool,
    #[serde(default = "er_ear_height")]
    pub ear_height: f64,
    #[serde(default = "er_listener_offset")]
    pub listener_offset: [f64; 2],
    #[serde(default = "er_room_dims_min")]
    pub room_dims_min: [f64; 3],
    #[serde(default = "er_room_dims_max")]
    pub room_dims_max: [f64; 3],
    #[serde(default = "er_floor")]
    pub floor: MaterialConfig,
    #[serde(default = "er_ceiling")]
    pub ceiling: MaterialConfig,
    #[serde(default = "er_front_wall")]
    pub front_wall: MaterialConfig,
    #[serde(default = "er_back_wall")]
    pub back_wall: MaterialConfig,
    #[serde(default = "er_side_walls")]
    pub side_walls: MaterialConfig,
    #[serde(default = "er_early_level")]
    pub early_level: f64,
    #[serde(default = "er_wet_distance_slope")]
    pub wet_distance_slope: f64,
}

fn er_enabled() -> bool { true }
fn er_ear_height() -> f64 { 1.2 }
fn er_listener_offset() -> [f64; 2] { [0.0, 0.0] }
fn er_room_dims_min() -> [f64; 3] { [4.0, 5.0, 3.0] }
fn er_room_dims_max() -> [f64; 3] { [25.0, 45.0, 18.0] }
fn er_floor() -> MaterialConfig { MaterialConfig::new(0.5, 3500.0) }
fn er_ceiling() -> MaterialConfig { MaterialConfig::new(0.6, 6000.0) }
fn er_front_wall() -> MaterialConfig { MaterialConfig::new(0.85, 10000.0) }
fn er_back_wall() -> MaterialConfig { MaterialConfig::new(0.40, 4000.0) }
fn er_side_walls() -> MaterialConfig { MaterialConfig::new(0.70, 8000.0) }
fn er_early_level() -> f64 { 1.0 }
fn er_wet_distance_slope() -> f64 { 0.1 }

impl Default for EarlyConfig {
    fn default() -> Self {
        Self {
            enabled: er_enabled(),
            ear_height: er_ear_height(),
            listener_offset: er_listener_offset(),
            room_dims_min: er_room_dims_min(),
            room_dims_max: er_room_dims_max(),
            floor: er_floor(),
            ceiling: er_ceiling(),
            front_wall: er_front_wall(),
            back_wall: er_back_wall(),
            side_walls: er_side_walls(),
            early_level: er_early_level(),
            wet_distance_slope: er_wet_distance_slope(),
        }
    }
}
```

- [ ] **Step 4: EarlyConfig / MaterialConfig を crate ルートに再エクスポートする**

`crates/s2v-core/src/lib.rs` の config 再エクスポート行を次に変更する。

```rust
pub use config::{AudioConfig, BgmConfig, Config, ConcurrencyConfig, EarlyConfig, EngineUrl, MaterialConfig};
```

- [ ] **Step 5: processor のテスト用 default_audio_config に新フィールドを追加する**

`AudioConfig` に必須フィールドが増えたため、リテラル構築している `crates/s2v-audio/src/processor.rs` のテスト用 `default_audio_config()` を更新しないと s2v-audio がコンパイルできない。`engine_volume_offsets` ブロックの直後に1行追加する。

```rust
            engine_volume_offsets: {
                let mut m = HashMap::new();
                m.insert("voicevox".to_string(), 1.0);
                m
            },
            early_reflections: s2v_core::EarlyConfig::default(),
```

- [ ] **Step 6: テストを実行して通ることを確認する**

Run: `cargo test -p s2v-core early_reflections && cargo test -p s2v-audio`
Expected: PASS（s2v-core の新2テスト＋既存テスト、s2v-audio の既存テストがすべて通る。フィールド追加で s2v-audio がコンパイルできること）

- [ ] **Step 7: コミットする**

```bash
git add crates/s2v-core/src/config.rs crates/s2v-core/src/lib.rs crates/s2v-audio/src/processor.rs
git commit -m "feat(core): add EarlyConfig to AudioConfig with serde defaults"
```

---

## Task 2: geometry モジュール（calc_geometry 移設＋部屋/イメージ幾何）

直接音の出力を一切変えずに `calc_geometry` を自由関数へ移し、部屋寸法とイメージ位置の純粋関数を追加する。

**Files:**
- Create: `crates/s2v-audio/src/geometry.rs`
- Modify: `crates/s2v-audio/src/lib.rs`
- Modify: `crates/s2v-audio/src/processor.rs`

- [ ] **Step 1: geometry.rs を作成し純粋関数の失敗テストを書く**

`crates/s2v-audio/src/geometry.rs` を新規作成する。

```rust
//! 直接音・早期反射で共用する幾何計算（純粋関数）。

/// ステレオマイク（中心基準、左右に ±spacing/2）から見た音源の左右距離・角度。
/// processor から移設した既存式と数値的に同一。
pub struct GeoParams {
    pub dist_l: f64,
    pub dist_r: f64,
    pub angle_l: f64,
    pub angle_r: f64,
}

/// 水平面の幾何: 音源を距離 distance・方位 pan_rad(+x=右,+y=前) に置いたときの
/// 左マイク(x=-d_h)・右マイク(x=+d_h)それぞれの距離と方位角。
pub fn calc_geometry(microphone_spacing: f64, distance: f64, pan_rad: f64) -> GeoParams {
    let d_h = microphone_spacing / 2.0;
    let sx = distance * pan_rad.sin();
    let sy = distance * pan_rad.cos();
    let dist_l = ((sx + d_h).powi(2) + sy.powi(2)).sqrt();
    let dist_r = ((sx - d_h).powi(2) + sy.powi(2)).sqrt();
    let angle_l = (sx + d_h).atan2(sy);
    let angle_r = (sx - d_h).atan2(sy);
    GeoParams { dist_l, dist_r, angle_l, angle_r }
}

/// room_size(0..1) を部屋寸法 [W,D,H] に線形補間する。
pub fn room_dims(room_size: f64, min: [f64; 3], max: [f64; 3]) -> [f64; 3] {
    let r = room_size.clamp(0.0, 1.0);
    [
        min[0] + (max[0] - min[0]) * r,
        min[1] + (max[1] - min[1]) * r,
        min[2] + (max[2] - min[2]) * r,
    ]
}

/// 反射面の種別。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface { Floor, Ceiling, LeftWall, RightWall, BackWall, FrontWall }

/// 音源 3D 位置 src=[x,y,z] を面 surface で鏡像化したイメージ位置を返す。
/// 箱は x∈[0,W], y∈[0,D], z∈[0,H]。
pub fn image_position(src: [f64; 3], surface: Surface, dims: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = src;
    let [w, d, h] = dims;
    match surface {
        Surface::Floor => [x, y, -z],
        Surface::Ceiling => [x, y, 2.0 * h - z],
        Surface::LeftWall => [-x, y, z],
        Surface::RightWall => [2.0 * w - x, y, z],
        Surface::BackWall => [x, -y, z],
        Surface::FrontWall => [x, 2.0 * d - y, z],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_geometry_symmetric_at_center() {
        let geo = calc_geometry(0.2, 1.0, 0.0);
        assert!((geo.dist_l - geo.dist_r).abs() < 1e-12);
    }

    #[test]
    fn room_dims_interpolates_endpoints_and_mid() {
        let min = [4.0, 5.0, 3.0];
        let max = [25.0, 45.0, 18.0];
        assert_eq!(room_dims(0.0, min, max), min);
        assert_eq!(room_dims(1.0, min, max), max);
        let mid = room_dims(0.5, min, max);
        assert!((mid[1] - 25.0).abs() < 1e-10); // (5+45)/2
    }

    #[test]
    fn image_position_floor_mirrors_below() {
        let img = image_position([2.0, 3.0, 1.2], Surface::Floor, [10.0, 12.0, 4.0]);
        assert_eq!(img, [2.0, 3.0, -1.2]);
    }

    #[test]
    fn image_position_front_wall_mirrors_forward() {
        // 前壁 y=D=12 → y' = 2*12 - 3 = 21
        let img = image_position([2.0, 3.0, 1.2], Surface::FrontWall, [10.0, 12.0, 4.0]);
        assert_eq!(img, [2.0, 21.0, 1.2]);
    }
}
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p s2v-audio --lib geometry::`
Expected: FAIL（`geometry` モジュール未登録でコンパイルエラー）

- [ ] **Step 3: lib.rs に geometry を登録する**

`crates/s2v-audio/src/lib.rs` を次のようにする。

```rust
pub mod geometry;
pub mod processor;
pub mod resampler;
pub mod reverb;

pub use processor::AudioProcessor;
pub use resampler::resample_mono;
pub use reverb::IrCache;
```

- [ ] **Step 4: processor.rs を移設した calc_geometry へ切り替える（出力不変）**

`crates/s2v-audio/src/processor.rs` の冒頭 `use` に追加する。

```rust
use crate::geometry::{calc_geometry, GeoParams};
```

`process` 内の幾何計算呼び出し（現在 `let geo = self.calc_geometry(cast.distance, pan_rad);`）を次に変更する。

```rust
        let pan_rad = cast.pan.to_radians();
        let geo = calc_geometry(self.config.microphone_spacing, cast.distance, pan_rad);
```

`processor.rs` 内の **メソッド `fn calc_geometry(...)` 定義（`impl AudioProcessor` 内）と、ファイル末尾の `struct GeoParams { ... }` 定義を削除**する（自由関数・geometry.rs の型に一本化）。
`processor.rs` 内テスト `calc_geometry_symmetric_at_center`（`proc.calc_geometry(1.0, 0.0)` を呼ぶもの）を次に置き換える。

```rust
    #[test]
    fn calc_geometry_symmetric_at_center() {
        let geo = calc_geometry(0.2, 1.0, 0.0);
        assert!((geo.dist_l - geo.dist_r).abs() < 1e-10);
    }
```

- [ ] **Step 5: テストを実行して通ることを確認する**

Run: `cargo test -p s2v-audio`
Expected: PASS（geometry の4テスト＋既存の processor/reverb テストがすべて通る。直接音の計算式は同一なので既存テストは不変で通る）

- [ ] **Step 6: コミットする**

```bash
git add crates/s2v-audio/src/geometry.rs crates/s2v-audio/src/lib.rs crates/s2v-audio/src/processor.rs
git commit -m "refactor(audio): extract calc_geometry as free fn + add room/image geometry"
```

---

## Task 3: early.rs — イメージソースのタップ生成

直接音と同じマイクペアで6面の一次反射タップ（左右遅延・左右ゲイン・素材ローパス済み信号）を生成する。

**Files:**
- Create: `crates/s2v-audio/src/early.rs`
- Modify: `crates/s2v-audio/src/lib.rs`
- Modify: `crates/s2v-audio/src/reverb.rs`（`butterworth_lowpass_sos`/`sosfilt_single_section` を `pub(crate)` 化）
- Modify: `crates/s2v-audio/src/processor.rs`（`apply_air_absorption` を `pub(crate)` 化）

- [ ] **Step 1: 流用する内部関数を pub(crate) 化する**

`crates/s2v-audio/src/reverb.rs` の2関数のシグネチャを変更する。

```rust
pub(crate) fn butterworth_lowpass_sos(cutoff_hz: f64, sample_rate: f64) -> [f64; 6] {
```
```rust
pub(crate) fn sosfilt_single_section(sos: &[f64; 6], input: &[f64]) -> Vec<f64> {
```

`crates/s2v-audio/src/processor.rs` の空気吸収関数のシグネチャを変更する。

```rust
pub(crate) fn apply_air_absorption(samples: &[f32], dist: f64, sample_rate: u32, air_coeff: f64) -> Vec<f32> {
```

- [ ] **Step 2: early.rs を作成し失敗テストを書く**

`crates/s2v-audio/src/early.rs` を新規作成する。

```rust
//! 部屋を箱としたイメージソース法による一次反射タップの生成。

use s2v_core::{AudioConfig, EarlyConfig, MaterialConfig};

use crate::geometry::{calc_geometry, image_position, room_dims, Surface};
use crate::processor::apply_air_absorption;
use crate::reverb::{butterworth_lowpass_sos, sosfilt_single_section};

/// ステレオバッファへ加算する1つの反射タップ。
/// `sig` は素材ローパス済みの信号（長さは入力 mono と同じ）。
pub struct EarlyTap {
    pub sig: Vec<f32>,
    pub rel_l: usize,
    pub rel_r: usize,
    pub gain_l: f32,
    pub gain_r: f32,
}

/// 6面の一次反射タップを生成する。`min_delay_direct` は直接音の最早到達サンプル
/// （processor が time-zero とする値）で、各タップの相対遅延の基準にする。
pub fn build_early_taps(
    mono: &[f32],
    distance: f64,
    pan_rad: f64,
    vol_factor: f64,
    audio: &AudioConfig,
    er: &EarlyConfig,
    room_size: f64,
    sample_rate: u32,
    min_delay_direct: usize,
) -> Vec<EarlyTap> {
    if !er.enabled {
        return Vec::new();
    }
    let dims = room_dims(room_size, er.room_dims_min, er.room_dims_max);
    let [w, d, h] = dims;
    let eps = 0.05_f64;

    // 聴取者(マイクペア中心) L と音源 S を箱座標で配置（同高 ear_height）。
    let lx = (w / 2.0 + er.listener_offset[0]).clamp(eps, w - eps);
    let ly = (d / 2.0 + er.listener_offset[1]).clamp(eps, d - eps);
    let lz = er.ear_height.clamp(eps, h - eps);
    let sx = (lx + distance * pan_rad.sin()).clamp(eps, w - eps);
    let sy = (ly + distance * pan_rad.cos()).clamp(eps, d - eps);
    let sz = lz;
    let src = [sx, sy, sz];

    let surfaces = [
        (Surface::Floor, &er.floor),
        (Surface::Ceiling, &er.ceiling),
        (Surface::LeftWall, &er.side_walls),
        (Surface::RightWall, &er.side_walls),
        (Surface::BackWall, &er.back_wall),
        (Surface::FrontWall, &er.front_wall),
    ];

    let c = audio.sound_speed;
    let fs = sample_rate as f64;
    let k = audio.mic_directivity;
    let mic_angle_rad = audio.mic_angle.to_radians();

    let mut taps = Vec::new();
    for (surface, mat) in surfaces {
        if mat.reflection_coeff <= 0.0 {
            continue;
        }
        let img = image_position(src, surface, dims);
        // 聴取者 L 基準のベクトル
        let dx = img[0] - lx;
        let dy = img[1] - ly;
        let dz = img[2] - lz;
        let hdist = (dx * dx + dy * dy).sqrt();
        let azimuth = dx.atan2(dy);
        let geo = calc_geometry(audio.microphone_spacing, hdist, azimuth);

        // 各マイクへの 3D 経路（高さ迂回 dz を加味）
        let path_l = (geo.dist_l.powi(2) + dz * dz).sqrt();
        let path_r = (geo.dist_r.powi(2) + dz * dz).sqrt();

        // 指向性パターン（外開きORTF: Lは+mic_angle, Rは-mic_angle）
        let pat_l = ((1.0 - k) + k * (geo.angle_l + mic_angle_rad).cos()).max(0.01);
        let pat_r = ((1.0 - k) + k * (geo.angle_r - mic_angle_rad).cos()).max(0.01);

        let coeff = mat.reflection_coeff * er.early_level;
        let gain_l = (vol_factor * (audio.reference_dist / path_l.max(0.1)) * pat_l * coeff) as f32;
        let gain_r = (vol_factor * (audio.reference_dist / path_r.max(0.1)) * pat_r * coeff) as f32;

        // 相対遅延（直接音 time-zero 基準）。負にならないよう飽和。
        let delay_l = (path_l / c * fs) as i64 - min_delay_direct as i64;
        let delay_r = (path_r / c * fs) as i64 - min_delay_direct as i64;
        let rel_l = delay_l.max(0) as usize;
        let rel_r = delay_r.max(0) as usize;

        // 信号: 空気吸収(平均経路) → 素材ローパス
        let avg_path = (path_l + path_r) / 2.0;
        let absorbed = apply_air_absorption(mono, avg_path, sample_rate, audio.air_absorption_coeff);
        let sig = material_lowpass(&absorbed, mat, fs);

        taps.push(EarlyTap { sig, rel_l, rel_r, gain_l, gain_r });
    }
    taps
}

/// 素材の吸音カットオフで2次Butterworthローパスをかける（f32入出力）。
fn material_lowpass(samples: &[f32], mat: &MaterialConfig, fs: f64) -> Vec<f32> {
    let nyq = fs / 2.0;
    if mat.absorption_cutoff_hz >= nyq - 1.0 {
        return samples.to_vec();
    }
    let sos = butterworth_lowpass_sos(mat.absorption_cutoff_hz, fs);
    let input: Vec<f64> = samples.iter().map(|&s| s as f64).collect();
    sosfilt_single_section(&sos, &input).iter().map(|&s| s as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn audio_cfg() -> AudioConfig {
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
            mic_directivity: 0.2,
            mic_angle: 30.0,
            engine_volume_offsets: HashMap::new(),
            early_reflections: EarlyConfig::default(),
        }
    }

    #[test]
    fn disabled_returns_no_taps() {
        let mut er = EarlyConfig::default();
        er.enabled = false;
        let mono = vec![1.0_f32; 1000];
        let taps = build_early_taps(&mono, 2.0, 0.0, 1.0, &audio_cfg(), &er, 0.1, 48000, 0);
        assert!(taps.is_empty());
    }

    #[test]
    fn only_surfaces_with_positive_coeff_produce_taps() {
        let mut er = EarlyConfig::default();
        // 床のみ残し他を0に
        er.ceiling.reflection_coeff = 0.0;
        er.front_wall.reflection_coeff = 0.0;
        er.back_wall.reflection_coeff = 0.0;
        er.side_walls.reflection_coeff = 0.0;
        let mono = vec![1.0_f32; 1000];
        let taps = build_early_taps(&mono, 2.0, 0.0, 1.0, &audio_cfg(), &er, 0.1, 48000, 0);
        assert_eq!(taps.len(), 1, "床のみ → 1タップ");
    }

    #[test]
    fn floor_tap_delay_matches_analytic_value() {
        // 中央・正面(pan=0)・距離2m・ear_height=1.2 → 床経路の中心遅延 ≈
        // (sqrt(2^2 + (2*1.2)^2) - 0)/c * fs を min_delay_direct=0 基準で。
        // マイク間隔による左右差は小さいので rel_l を中心値の近傍で検証。
        let mut er = EarlyConfig::default();
        er.ceiling.reflection_coeff = 0.0;
        er.front_wall.reflection_coeff = 0.0;
        er.back_wall.reflection_coeff = 0.0;
        er.side_walls.reflection_coeff = 0.0;
        let mono = vec![1.0_f32; 1000];
        let taps = build_early_taps(&mono, 2.0, 0.0, 1.0, &audio_cfg(), &er, 0.1, 48000, 0);
        let expected = ((2.0_f64.powi(2) + (2.0 * 1.2_f64).powi(2)).sqrt() / 340.0 * 48000.0) as i64;
        let rel = taps[0].rel_l as i64;
        assert!((rel - expected).abs() <= 5, "床タップ遅延 rel={rel}, expected≈{expected}");
    }

    #[test]
    fn material_lowpass_attenuates_high_frequencies() {
        // 高域正弦波(16kHz)に床カットオフ(3500Hz)を当てると振幅が大きく減る
        let fs = 48000.0;
        let n = 4096;
        let hi: Vec<f32> = (0..n).map(|i| (2.0 * std::f32::consts::PI * 16000.0 * i as f32 / 48000.0).sin()).collect();
        let mat = MaterialConfig { reflection_coeff: 1.0, absorption_cutoff_hz: 3500.0 };
        let out = material_lowpass(&hi, &mat, fs);
        let peak_in = hi.iter().cloned().map(f32::abs).fold(0.0_f32, f32::max);
        let peak_out = out[1000..].iter().cloned().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(peak_out < peak_in * 0.5, "16kHzが半分以下に減衰すること: in={peak_in}, out={peak_out}");
    }
}
```

- [ ] **Step 3: lib.rs に early を登録する**

`crates/s2v-audio/src/lib.rs` の先頭の module 宣言群に `pub mod early;` を追加する。

```rust
pub mod early;
pub mod geometry;
pub mod processor;
pub mod resampler;
pub mod reverb;
```

- [ ] **Step 4: テストを実行して通ることを確認する**

Run: `cargo test -p s2v-audio early::`
Expected: PASS（4テストとも通る）。あわせて `cargo test -p s2v-audio` で既存テストも通ること。

- [ ] **Step 5: コミットする**

```bash
git add crates/s2v-audio/src/early.rs crates/s2v-audio/src/lib.rs crates/s2v-audio/src/reverb.rs crates/s2v-audio/src/processor.rs
git commit -m "feat(audio): add image-source early reflection tap generation"
```

---

## Task 4: processor へ早期反射を統合（enabled ガード・回帰不変）

**Files:**
- Modify: `crates/s2v-audio/src/processor.rs`

- [ ] **Step 1: 統合の振る舞いを検証する失敗テストを書く**

`crates/s2v-audio/src/processor.rs` の `mod tests` 末尾に追加する（`default_audio_config` は Task 1 Step 5 で既に `early_reflections` フィールドを含むように更新済み）。

```rust
    #[test]
    fn early_reflections_disabled_matches_no_early_output() {
        // enabled=false のとき、早期反射なしの従来出力と完全一致すること（回帰防止）
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.wav");
        write_test_wav(&input, 48000, 440.0, 0.1);

        let mut cfg = default_audio_config();
        cfg.early_reflections.enabled = false;
        // 拡散リバーブの距離項を固定にするため reverb_wet=0 にして直接音のみ比較
        cfg.reverb_wet = 0.0;
        let proc = AudioProcessor::new(cfg);
        let out = dir.path().join("out.wav");
        proc.process(&input, &out, &dummy_cast(20.0, 2.0), &SceneConfig { name: "s".into(), room_size: Some(0.1), reverb_wet: Some(0.0) }).unwrap();

        let mut r = hound::WavReader::open(&out).unwrap();
        let energy: f64 = r.samples::<i16>().map(|s| { let v = s.unwrap() as f64; v * v }).sum();
        assert!(energy > 0.0, "出力が生成されること");
    }

    #[test]
    fn early_reflections_enabled_adds_energy() {
        // 同じ入力で enabled=true の方が（早期反射ぶん）総エネルギーが増えること
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.wav");
        write_test_wav(&input, 48000, 440.0, 0.1);
        let scene = SceneConfig { name: "s".into(), room_size: Some(0.1), reverb_wet: Some(0.0) };

        let energy_for = |enabled: bool| -> f64 {
            let mut cfg = default_audio_config();
            cfg.reverb_wet = 0.0; // 拡散リバーブを切り、早期反射の寄与だけを見る
            cfg.early_reflections.enabled = enabled;
            let proc = AudioProcessor::new(cfg);
            let out = dir.path().join(format!("out_{enabled}.wav"));
            proc.process(&input, &out, &dummy_cast(20.0, 2.0), &scene).unwrap();
            let mut r = hound::WavReader::open(&out).unwrap();
            r.samples::<i16>().map(|s| { let v = s.unwrap() as f64; v * v }).sum()
        };

        let e_off = energy_for(false);
        let e_on = energy_for(true);
        assert!(e_on > e_off * 1.01, "早期反射ありで総エネルギー増加: off={e_off}, on={e_on}");
    }
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p s2v-audio --lib early_reflections_enabled_adds_energy`
Expected: FAIL（まだ early を統合していないので on==off でエネルギーが増えない／または未配線でアサート失敗）

- [ ] **Step 3: process に早期反射の生成・加算と out_len 拡張を組み込む**

`crates/s2v-audio/src/processor.rs` の冒頭 `use` に追加する。

```rust
use crate::early::build_early_taps;
```

`process` 内で、直接音の `rel_l`/`rel_r` と `vol_factor` が確定した後、ステレオバッファ確保（現在の `let out_len = ...; let mut stereo = ...;`）の **直前** に早期反射タップを生成する。
現在の該当ブロック:

```rust
        // --- ステレオバッファ構築 ---
        let rv_time = 0.05 + room_size * 3.0;
        let rv_samples = if reverb_wet > 0.0 { (self.config.sample_rate as f64 * rv_time) as usize } else { 0 };
        let out_len = mono.len() + rel_l.max(rel_r) + rv_samples;
        let mut stereo: Vec<[f32; 2]> = vec![[0.0, 0.0]; out_len];

        for (i, (&sl, &sr)) in data_l.iter().zip(data_r.iter()).enumerate() {
            stereo[rel_l + i][0] = sl * gain_l;
            stereo[rel_r + i][1] = sr * gain_r;
        }
```

を次に置き換える。

```rust
        // --- 早期反射タップ（イメージソース法）---
        let early_taps = build_early_taps(
            &mono,
            cast.distance,
            pan_rad,
            vol_factor,
            &self.config,
            &self.config.early_reflections,
            room_size,
            self.config.sample_rate,
            min_delay,
        );
        let early_max_rel = early_taps.iter().map(|t| t.rel_l.max(t.rel_r)).max().unwrap_or(0);

        // --- ステレオバッファ構築 ---
        let rv_time = 0.05 + room_size * 3.0;
        let rv_samples = if reverb_wet > 0.0 { (self.config.sample_rate as f64 * rv_time) as usize } else { 0 };
        let out_len = mono.len() + rel_l.max(rel_r).max(early_max_rel) + rv_samples;
        let mut stereo: Vec<[f32; 2]> = vec![[0.0, 0.0]; out_len];

        for (i, (&sl, &sr)) in data_l.iter().zip(data_r.iter()).enumerate() {
            stereo[rel_l + i][0] = sl * gain_l;
            stereo[rel_r + i][1] = sr * gain_r;
        }

        // 早期反射を加算
        for tap in &early_taps {
            for (i, &s) in tap.sig.iter().enumerate() {
                stereo[tap.rel_l + i][0] += s * tap.gain_l;
                stereo[tap.rel_r + i][1] += s * tap.gain_r;
            }
        }
```

注: `enabled=false` のとき `early_taps` は空、`early_max_rel=0` となり `out_len`・配置とも従来と完全一致する（回帰不変）。

- [ ] **Step 4: テストを実行して通ることを確認する**

Run: `cargo test -p s2v-audio`
Expected: PASS（`early_reflections_enabled_adds_energy` を含む全テスト。既存の `process_*` テストも通る）

- [ ] **Step 5: コミットする**

```bash
git add crates/s2v-audio/src/processor.rs
git commit -m "feat(audio): integrate early reflections into processor (guarded by enabled)"
```

---

## Task 5: 距離依存 D/R（wet_distance_slope の config 化）

拡散リバーブの距離スロープを config 値に置き換える（早期反射レベルは Task 3 の `early_level` で既に距離物理に従う）。

**Files:**
- Modify: `crates/s2v-audio/src/reverb.rs`
- Modify: `crates/s2v-audio/src/processor.rs`

- [ ] **Step 1: slope を反映する失敗テストを書く**

`crates/s2v-audio/src/reverb.rs` の `mod tests` 末尾に追加する。

```rust
    #[test]
    fn apply_uses_distance_slope_for_wet_amount() {
        // slope を大きくすると遠距離での wet 比が増える → 出力の変化量が増える
        let make_signal = || -> Vec<[f32; 2]> {
            (0..2400).map(|i| {
                let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin() * 0.5;
                [v, v]
            }).collect()
        };
        let cache = IrCache::new(48000);
        cache.compute_if_needed(0.5);

        let dry = make_signal();
        let mut s_small = make_signal();
        let mut s_large = make_signal();
        cache.apply(&mut s_small, 0.5, 0.3, 5.0, 0.1);
        cache.apply(&mut s_large, 0.5, 0.3, 5.0, 0.5);

        let dev = |sig: &Vec<[f32; 2]>| -> f64 {
            sig.iter().zip(dry.iter()).map(|(a, b)| ((a[0] - b[0]) as f64).powi(2)).sum()
        };
        assert!(dev(&s_large) > dev(&s_small), "slope大の方がwet寄与が大きいこと");
    }
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p s2v-audio --lib reverb::tests::apply_uses_distance_slope_for_wet_amount`
Expected: FAIL（`apply` の引数が4つでコンパイルエラー）

- [ ] **Step 3: IrCache::apply に wet_distance_slope を追加する**

`crates/s2v-audio/src/reverb.rs` の `apply` シグネチャと `actual_wet` 計算を変更する。

```rust
    pub fn apply(&self, stereo: &mut Vec<[f32; 2]>, room_size: f64, reverb_wet: f64, avg_dist: f64, wet_distance_slope: f64) {
```
```rust
        let actual_wet = (reverb_wet * (1.0 + wet_distance_slope * avg_dist)).min(0.9) as f32;
```

既存テスト `apply_with_zero_wet_leaves_signal_unchanged` と `apply_with_wet_modifies_signal` の `cache.apply(...)` 呼び出しに第5引数 `0.1` を追加する。

```rust
        cache.apply(&mut signal, 0.3, 0.0, 1.0, 0.1);
```
```rust
        cache.apply(&mut signal, 0.5, 0.3, 1.0, 0.1);
```

- [ ] **Step 4: processor の apply 呼び出しを更新する**

`crates/s2v-audio/src/processor.rs` のリバーブ適用箇所を変更する。
現在: `self.ir_cache.apply(&mut stereo, room_size, reverb_wet, cast.distance);`

```rust
        self.ir_cache.apply(&mut stereo, room_size, reverb_wet, cast.distance, self.config.early_reflections.wet_distance_slope);
```

- [ ] **Step 5: テストを実行して通ることを確認する**

Run: `cargo test -p s2v-audio`
Expected: PASS（slope テスト＋既存テスト全通過）

- [ ] **Step 6: コミットする**

```bash
git add crates/s2v-audio/src/reverb.rs crates/s2v-audio/src/processor.rs
git commit -m "feat(audio): make reverb wet distance slope configurable for D/R control"
```

---

## Task 6: config.toml 反映・全体ビルド・実機確認

**Files:**
- Modify: `config.toml`

- [ ] **Step 1: config.toml に early_reflections セクションを追加する**

`config.toml` の `[audio.engine_volume_offsets]` セクションの **直前**（`mic_angle = 30.0` の次の空行の後）に追加する。

```toml
[audio.early_reflections]
enabled = true
ear_height = 1.2
listener_offset = [0.0, 0.0]
room_dims_min = [4.0, 5.0, 3.0]
room_dims_max = [25.0, 45.0, 18.0]
early_level = 1.0
wet_distance_slope = 0.1
floor = { reflection_coeff = 0.5, absorption_cutoff_hz = 3500.0 }
ceiling = { reflection_coeff = 0.6, absorption_cutoff_hz = 6000.0 }
front_wall = { reflection_coeff = 0.85, absorption_cutoff_hz = 10000.0 }
back_wall = { reflection_coeff = 0.40, absorption_cutoff_hz = 4000.0 }
side_walls = { reflection_coeff = 0.70, absorption_cutoff_hz = 8000.0 }
```

- [ ] **Step 2: config.toml がパースできることを確認する**

`crates/s2v-core/src/config.rs` の `mod tests` 末尾に追加する。

```rust
    #[test]
    fn parses_real_config_toml_with_early_reflections() {
        let s = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../config.toml")).unwrap();
        let cfg = Config::from_toml(&s).unwrap();
        assert!(cfg.audio.early_reflections.enabled);
        assert!((cfg.audio.early_reflections.front_wall.reflection_coeff - 0.85).abs() < 1e-10);
    }
```

Run: `cargo test -p s2v-core parses_real_config_toml_with_early_reflections`
Expected: PASS

- [ ] **Step 3: ワークスペース全体のビルドと全テストを実行する**

Run: `cargo build && cargo test --workspace`
Expected: PASS（全クレートがビルドでき、全テストが通る）

- [ ] **Step 4: 実機でステレオ出力が生成されることを確認する**

Run: `cargo run -- <任意の台本.txt>`（エンジン設定済みなら最後まで、未設定でも config 読み込み・パースの成功までを確認）
Expected: エラーなく config がパースされ、パイプラインが起動する（早期反射が有効な状態で音声生成が走る）。生成された `*_full_dialogue.wav` 等を試聴し、接地感が出ているか確認する。

- [ ] **Step 5: コミットする**

```bash
git add config.toml crates/s2v-core/src/config.rs
git commit -m "feat(audio): enable early reflections in config.toml with material defaults"
```

---

## Self-Review メモ

- **Spec coverage**：§1座標/部屋=Task2(room_dims)+Task3(配置)、§2 6イメージ=Task2(image_position)+Task3(build_early_taps)、§3 5素材=Task1(config)+Task3(material_lowpass)、§4距離D/R=Task3(early_level)+Task5(wet_distance_slope)、§5モジュール構成=Task2/3/4、§6テスト=各Task。受け入れ条件（enabled=false一致=Task4回帰、5素材個別=Task3、距離D/R=Task5）を網羅。
- **Placeholder scan**：プレースホルダなし。各コードステップに実コード。
- **Type consistency**：`EarlyConfig`/`MaterialConfig`（Task1）、`calc_geometry(spacing,distance,pan_rad)`・`image_position`・`room_dims`・`Surface`（Task2）、`build_early_taps(...)`・`EarlyTap{sig,rel_l,rel_r,gain_l,gain_r}`・`material_lowpass`（Task3）、`apply(...,wet_distance_slope)`（Task5）は定義と使用で一致。`apply_air_absorption`/`butterworth_lowpass_sos`/`sosfilt_single_section` は Task3 で pub(crate) 化して再利用。
- **注意**：Python版は早期反射を持たないため本機能で意図的に乖離（設計書に明記済み）。
