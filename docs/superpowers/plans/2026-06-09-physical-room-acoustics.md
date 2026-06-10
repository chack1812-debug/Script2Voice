# 物理ベースの部屋音響 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 台本(scene)で部屋寸法・聴取者位置を指定可能にし、素材×寸法から拡散リバーブ(残響長・残響量)を物理導出して早期反射と残響を単一の部屋モデルに統一する。

**Architecture:** scene+config から単一の `RoomGeometry` と `ReverbParams`(rt60/pre_delay/wet_base) を解決する純粋関数を新規 `acoustics.rs` に置き、早期反射(`build_early_taps`)と拡散リバーブ(`IrCache`)の両方をそれで駆動する。`reverb_wet` は物理基準値 `wet_base` への倍率(既定1.0)。

**Tech Stack:** Rust / s2v-core(serde/toml/parser) / s2v-audio(hound, realfft) — 新規依存なし

設計書: `docs/superpowers/specs/2026-06-09-physical-room-acoustics-design.md`

**コンパイル順の注意（重要）:** `IrCache` と `build_early_taps` のシグネチャ変更は同一クレート(s2v-audio)内の `processor.rs` を、さらに `prewarm` 系はルート crate の `lib.rs` を同時に壊す。よって reverb/early/processor/lib の付け替えは **Task 3 に統合**し、Task 3 完了時に `cargo test --workspace` を緑にする（Task 3 内の中間状態はコンパイル不能でよいが、コミットは緑化後の末尾で1回行う）。Task 1・2 は加算的で単独緑。

---

## Task 1: SceneConfig 拡張と scene 構文（room_w/d/h, listener_dx/dy）

**Files:**
- Modify: `crates/s2v-core/src/types.rs`
- Modify: `crates/s2v-core/src/parser.rs`
- Modify: `crates/s2v-audio/src/processor.rs`（テストの SceneConfig literal を関数更新化）

- [ ] **Step 1: パーサの失敗テストを書く** — `crates/s2v-core/src/parser.rs` の `mod tests` 末尾に追加:

```rust
    #[test]
    fn scene_header_parses_room_dims_and_listener() {
        let p = ScriptParser::new();
        let sc = p.parse_scene_header("ホール room_w=25 room_d=45 room_h=18 listener_dx=0 listener_dy=-15");
        assert_eq!(sc.name, "ホール");
        assert_eq!(sc.room_w, Some(25.0));
        assert_eq!(sc.room_d, Some(45.0));
        assert_eq!(sc.room_h, Some(18.0));
        assert_eq!(sc.listener_dx, Some(0.0));
        assert_eq!(sc.listener_dy, Some(-15.0));
    }

    #[test]
    fn scene_header_room_dims_default_none() {
        let p = ScriptParser::new();
        let sc = p.parse_scene_header("小部屋 room_size=0.1");
        assert_eq!(sc.room_w, None);
        assert_eq!(sc.listener_dx, None);
        assert_eq!(sc.room_size, Some(0.1));
    }
```

- [ ] **Step 2: 失敗確認** — Run: `cargo test -p s2v-core scene_header_parses_room_dims_and_listener` → FAIL（フィールド未定義でコンパイルエラー）。

- [ ] **Step 3: SceneConfig にフィールド追加 + new 更新** — `crates/s2v-core/src/types.rs` の `SceneConfig` と `new`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SceneConfig {
    pub name: String,
    /// 省略時は `None`。実効値は処理時に AudioConfig の値へフォールバックする。
    pub room_size: Option<f64>,
    pub reverb_wet: Option<f64>,
    /// 部屋寸法[m]。3つすべて指定されたとき room_size より優先される。
    #[serde(default)]
    pub room_w: Option<f64>,
    #[serde(default)]
    pub room_d: Option<f64>,
    #[serde(default)]
    pub room_h: Option<f64>,
    /// 聴取者(マイクペア中心)の部屋中央からのオフセット[m]。省略時は config の listener_offset。
    #[serde(default)]
    pub listener_dx: Option<f64>,
    #[serde(default)]
    pub listener_dy: Option<f64>,
}

impl SceneConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            room_size: None,
            reverb_wet: None,
            room_w: None,
            room_d: None,
            room_h: None,
            listener_dx: None,
            listener_dy: None,
        }
    }
}
```

`types.rs` の `mod tests` の `scene_config_custom_values` 内 literal を関数更新構文へ:
```rust
        let sc = SceneConfig { room_size: Some(0.8), reverb_wet: Some(0.3), ..SceneConfig::new("広場") };
```

- [ ] **Step 4: parse_scene_header に新キーを追加し literal を関数更新化** — `crates/s2v-core/src/parser.rs` の `parse_scene_header` の戻り値:

```rust
        SceneConfig {
            room_size: params.get("room_size").copied(),
            reverb_wet: params.get("reverb_wet").copied(),
            room_w: params.get("room_w").copied(),
            room_d: params.get("room_d").copied(),
            room_h: params.get("room_h").copied(),
            listener_dx: params.get("listener_dx").copied(),
            listener_dy: params.get("listener_dy").copied(),
            ..SceneConfig::new(name)
        }
```

parser.rs:222 付近の `scene_config: crate::types::SceneConfig { name: String::new(), room_size: None, reverb_wet: None }` を:
```rust
            scene_config: crate::types::SceneConfig::new(String::new()),
```

- [ ] **Step 5: s2v-audio の SceneConfig literal を関数更新化** — `crates/s2v-audio/src/processor.rs` テスト内の各 `SceneConfig { name, room_size, reverb_wet }` を関数更新構文へ:
```rust
        SceneConfig { room_size: Some(0.1), reverb_wet: Some(0.3), ..SceneConfig::new("テスト") }
```
```rust
        let scene = SceneConfig { room_size: None, reverb_wet: None, ..SceneConfig::new("テスト") };
```
```rust
        let scene = SceneConfig { room_size: Some(0.8), reverb_wet: Some(0.2), ..SceneConfig::new("テスト") };
```
（`resolve_reverb_params_prefers_scene_over_config_default` と `..._prefers_cast_over_scene_and_config` の2箇所が Some(0.8)/Some(0.2)）
process 統合テストの2箇所:
```rust
        ..., &SceneConfig { room_size: Some(0.1), reverb_wet: Some(0.0), ..SceneConfig::new("s") }).unwrap();
```
```rust
        let scene = SceneConfig { room_size: Some(0.1), reverb_wet: Some(0.0), ..SceneConfig::new("s") };
```

- [ ] **Step 6: テスト通過確認** — Run: `cargo test -p s2v-core scene_header && cargo test -p s2v-audio` → PASS。

- [ ] **Step 7: コミット**
```bash
git add crates/s2v-core/src/types.rs crates/s2v-core/src/parser.rs crates/s2v-audio/src/processor.rs
git commit -m "feat(core): add room dims and listener offset to scene config and parser"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Task 2: acoustics.rs — 部屋ジオメトリ解決と物理残響パラメータ

**Files:**
- Create: `crates/s2v-audio/src/acoustics.rs`
- Modify: `crates/s2v-audio/src/lib.rs`

- [ ] **Step 1: acoustics.rs を作成し失敗テストを書く** — `crates/s2v-audio/src/acoustics.rs` を新規作成:

```rust
//! 部屋ジオメトリの解決と、寸法×素材からの物理残響パラメータ(Sabine)算出。

use s2v_core::{EarlyConfig, SceneConfig};

use crate::geometry::room_dims;

/// 解決済みの部屋ジオメトリ。早期反射と拡散リバーブの両方に渡す。
#[derive(Clone, Copy, Debug)]
pub struct RoomGeometry {
    pub dims: [f64; 3],
    pub listener_offset: [f64; 2],
}

/// scene と config から部屋寸法・聴取オフセットを解決する。
pub fn resolve_room_geometry(scene: &SceneConfig, er: &EarlyConfig, fallback_room_size: f64) -> RoomGeometry {
    let dims = match (scene.room_w, scene.room_d, scene.room_h) {
        (Some(w), Some(d), Some(h)) => [w, d, h],
        _ => {
            let rs = scene.room_size.unwrap_or(fallback_room_size);
            room_dims(rs, er.room_dims_min, er.room_dims_max)
        }
    };
    let listener_offset = match (scene.listener_dx, scene.listener_dy) {
        (None, None) => er.listener_offset,
        (dx, dy) => [dx.unwrap_or(0.0), dy.unwrap_or(0.0)],
    };
    RoomGeometry { dims, listener_offset }
}

/// 拡散リバーブの物理パラメータ。
#[derive(Clone, Copy, Debug)]
pub struct ReverbParams {
    pub rt60: f64,
    pub pre_delay: usize,
    pub wet_base: f64,
}

/// 寸法×素材(反射率)から Sabine の RT60・平均自由行程プリディレイ・wet基準値を算出する。
pub fn compute_reverb_params(dims: [f64; 3], er: &EarlyConfig, sound_speed: f64, sample_rate: u32) -> ReverbParams {
    let [w, d, h] = dims;
    let s_floor = w * d;
    let s_ceiling = w * d;
    let s_front = w * h;
    let s_back = w * h;
    let s_side = 2.0 * d * h;
    let total_area = s_floor + s_ceiling + s_front + s_back + s_side;

    // 振幅反射係数 coeff → エネルギー吸音率 α = 1 - coeff^2
    let alpha = |coeff: f64| 1.0 - coeff * coeff;
    let total_absorption = s_floor * alpha(er.floor.reflection_coeff)
        + s_ceiling * alpha(er.ceiling.reflection_coeff)
        + s_front * alpha(er.front_wall.reflection_coeff)
        + s_back * alpha(er.back_wall.reflection_coeff)
        + s_side * alpha(er.side_walls.reflection_coeff);

    let volume = w * d * h;
    let rt60 = (0.161 * volume / total_absorption.max(1e-6)).clamp(0.05, 12.0);

    let mfp = 4.0 * volume / total_area.max(1e-6);
    let pre_delay = ((sample_rate as f64) * (mfp / sound_speed)) as usize;

    let avg_alpha = total_absorption / total_area.max(1e-6);
    let wet_base = (1.0 - avg_alpha).clamp(0.0, 1.0);

    ReverbParams { rt60, pre_delay, wet_base }
}

#[cfg(test)]
mod tests {
    use super::*;
    use s2v_core::MaterialConfig;

    fn er_uniform(coeff: f64) -> EarlyConfig {
        let mut er = EarlyConfig::default();
        let m = MaterialConfig { reflection_coeff: coeff, absorption_cutoff_hz: 24000.0 };
        er.floor = m.clone();
        er.ceiling = m.clone();
        er.front_wall = m.clone();
        er.back_wall = m.clone();
        er.side_walls = m;
        er
    }

    #[test]
    fn resolve_prefers_scene_room_dims_over_room_size() {
        let er = EarlyConfig::default();
        let scene = SceneConfig { room_w: Some(10.0), room_d: Some(20.0), room_h: Some(5.0), room_size: Some(0.0), ..SceneConfig::new("x") };
        let geo = resolve_room_geometry(&scene, &er, 0.5);
        assert_eq!(geo.dims, [10.0, 20.0, 5.0]);
    }

    #[test]
    fn resolve_falls_back_to_room_size_interpolation() {
        let er = EarlyConfig::default();
        let scene = SceneConfig { room_size: Some(0.0), ..SceneConfig::new("x") };
        let geo = resolve_room_geometry(&scene, &er, 0.5);
        assert_eq!(geo.dims, er.room_dims_min);
    }

    #[test]
    fn resolve_listener_uses_scene_then_config() {
        let mut er = EarlyConfig::default();
        er.listener_offset = [1.0, 2.0];
        let scene_none = SceneConfig::new("x");
        assert_eq!(resolve_room_geometry(&scene_none, &er, 0.5).listener_offset, [1.0, 2.0]);
        let scene_set = SceneConfig { listener_dx: Some(-3.0), listener_dy: Some(4.0), ..SceneConfig::new("x") };
        assert_eq!(resolve_room_geometry(&scene_set, &er, 0.5).listener_offset, [-3.0, 4.0]);
    }

    #[test]
    fn rt60_matches_sabine_for_known_room() {
        let er = er_uniform(0.7);
        let rp = compute_reverb_params([10.0, 20.0, 5.0], &er, 340.0, 48000);
        let s = 2.0 * (10.0 * 20.0) + 2.0 * (10.0 * 5.0) + 2.0 * (20.0 * 5.0);
        let a = s * (1.0 - 0.7_f64 * 0.7);
        let expected = 0.161 * (10.0 * 20.0 * 5.0) / a;
        assert!((rp.rt60 - expected).abs() < 1e-9, "rt60={}, expected={}", rp.rt60, expected);
    }

    #[test]
    fn rt60_clamped_high_when_no_absorption() {
        let er = er_uniform(1.0);
        let rp = compute_reverb_params([10.0, 20.0, 5.0], &er, 340.0, 48000);
        assert!((rp.rt60 - 12.0).abs() < 1e-9);
    }

    #[test]
    fn wet_base_zero_when_fully_absorptive_and_high_when_reflective() {
        let absorptive = compute_reverb_params([10.0, 20.0, 5.0], &er_uniform(0.0), 340.0, 48000);
        let reflective = compute_reverb_params([10.0, 20.0, 5.0], &er_uniform(1.0), 340.0, 48000);
        assert!(absorptive.wet_base < 0.01, "全面吸音で wet_base≈0, got {}", absorptive.wet_base);
        assert!(reflective.wet_base > 0.99, "全面反射で wet_base≈1, got {}", reflective.wet_base);
    }

    #[test]
    fn outdoor_walls_zero_gives_short_rt60_and_low_wet() {
        let mut er = er_uniform(0.0);
        er.floor = MaterialConfig { reflection_coeff: 0.5, absorption_cutoff_hz: 3500.0 };
        let rp = compute_reverb_params([20.0, 20.0, 10.0], &er, 340.0, 48000);
        assert!(rp.rt60 < 1.0, "屋外的: rt60 短い, got {}", rp.rt60);
        assert!(rp.wet_base < 0.2, "屋外的: wet_base 小さい, got {}", rp.wet_base);
    }
}
```

- [ ] **Step 2: 失敗確認** — Run: `cargo test -p s2v-audio --lib acoustics::` → FAIL（モジュール未登録）。

- [ ] **Step 3: lib.rs に登録** — `crates/s2v-audio/src/lib.rs` の module 宣言群先頭に `pub mod acoustics;` を追加（early の前）:
```rust
pub mod acoustics;
pub mod early;
pub mod geometry;
pub mod processor;
pub mod resampler;
pub mod reverb;
```

- [ ] **Step 4: テスト通過確認** — Run: `cargo test -p s2v-audio` → PASS（acoustics 7 + 既存）。

- [ ] **Step 5: コミット**
```bash
git add crates/s2v-audio/src/acoustics.rs crates/s2v-audio/src/lib.rs
git commit -m "feat(audio): add room geometry resolution and Sabine reverb params"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Task 3: 物理モデルへ一括移行（reverb + early + processor + lib）

**重要:** 本タスクは IrCache/早期反射のシグネチャ変更が processor・lib を同時に壊すため、サブステップ間ではコンパイルが通らない。**全サブステップ完了後に `cargo test --workspace` を緑にしてから 1 回コミットする**。

**Files:**
- Modify: `crates/s2v-audio/src/reverb.rs`
- Modify: `crates/s2v-audio/src/early.rs`
- Modify: `crates/s2v-audio/src/processor.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: reverb.rs を (rt60, pre_delay) キーへ再構成**

構造体:
```rust
pub struct IrCache {
    sample_rate: u32,
    cache: Mutex<HashMap<(OrderedFloat<f64>, usize), [Vec<f32>; 2]>>,
}
```
`prewarm`/`compute_if_needed`:
```rust
    pub fn prewarm(&self, params: &[(f64, usize)]) {
        for &(rt60, pre_delay) in params {
            self.compute_if_needed(rt60, pre_delay);
        }
    }

    pub fn compute_if_needed(&self, rt60: f64, pre_delay: usize) {
        let key = (OrderedFloat(round4(rt60)), pre_delay);
        let mut cache = self.cache.lock().unwrap();
        if cache.contains_key(&key) {
            return;
        }
        let ir = build_ir(rt60, pre_delay, self.sample_rate);
        cache.insert(key, ir);
    }
```
`apply`:
```rust
    pub fn apply(
        &self,
        stereo: &mut Vec<[f32; 2]>,
        rt60: f64,
        pre_delay: usize,
        reverb_wet: f64,
        wet_base: f64,
        avg_dist: f64,
        wet_distance_slope: f64,
    ) {
        let actual_wet = (reverb_wet * wet_base * (1.0 + wet_distance_slope * avg_dist)).min(0.9) as f32;
        if actual_wet <= 0.0 || stereo.is_empty() {
            return;
        }
        let key = (OrderedFloat(round4(rt60)), pre_delay);
        let cache = self.cache.lock().unwrap();
        let Some(ir) = cache.get(&key) else { return };

        for ch in 0..2 {
            let dry: Vec<f32> = stereo.iter().map(|s| s[ch]).collect();
            let wet = fft_convolve(&dry, &ir[ch]);
            let dry_peak = dry.iter().cloned().map(f32::abs).fold(0.0_f32, f32::max);
            let wet_slice = &wet[..dry.len()];
            let wet_peak = wet_slice.iter().cloned().map(f32::abs).fold(1e-6_f32, f32::max);
            let wet_norm_factor = if dry_peak > 0.0 { (dry_peak * 0.4) / wet_peak } else { 0.0 };
            for (i, s) in stereo.iter_mut().enumerate() {
                let w = wet_slice[i] * wet_norm_factor;
                s[ch] = (1.0 - actual_wet) * s[ch] + actual_wet * w;
            }
        }
    }
```
`build_ir`:
```rust
fn build_ir(rt60: f64, pre_delay: usize, sample_rate: u32) -> [Vec<f32>; 2] {
    let fs = sample_rate as f64;
    let rv_time = rt60;
    let n = (fs * rv_time) as usize;

    let seed = (round4(rt60) * 10000.0) as u64 & 0xFFFF_FFFF;
    let mut rng = SmallRng::seed_from_u64(seed);

    let sos = butterworth_lowpass_sos(1800.0, fs);

    let decay: Vec<f64> = (0..n)
        .map(|i| { let t = i as f64 / fs; (-6.91 * t / rv_time).exp() })
        .collect();

    std::array::from_fn(|_| {
        let noise: Vec<f64> = StandardNormal.sample_iter(&mut rng).take(n).collect();
        let filtered = sosfilt_single_section(&sos, &noise);
        let mut ir: Vec<f32> = vec![0.0; pre_delay];
        ir.extend(filtered.iter().zip(decay.iter()).map(|(s, d)| (s * d) as f32));
        ir
    })
}
```

reverb.rs の既存テストを新 API へ更新:
- `ir_cache_builds_entry`: `compute_if_needed(1.0, 240)`、`guard.contains_key(&(OrderedFloat(round4(1.0)), 240))`。
- `prewarm_fills_multiple_entries`: `prewarm(&[(0.5, 240), (1.0, 240), (2.0, 480)])`、`assert_eq!(guard.len(), 3)`。
- `ir_has_two_channels`: `compute_if_needed(1.0, 240)`、`guard[&(OrderedFloat(round4(1.0)), 240)]`。
- `apply_with_zero_wet_leaves_signal_unchanged`: `compute_if_needed(1.0, 240)`、`apply(&mut signal, 1.0, 240, 0.0, 0.8, 1.0, 0.1)`。
- `apply_with_wet_modifies_signal`: `compute_if_needed(1.0, 240)`、`apply(&mut signal, 1.0, 240, 1.0, 0.8, 1.0, 0.1)`。
- `apply_uses_distance_slope_for_wet_amount` / `apply_increases_wet_with_distance`: `compute_if_needed(1.0, 240)`、各 `apply(&mut s_*, 1.0, 240, 0.3, 0.8, <dist>, <slope>)`（比較ロジックは維持）。
- `build_ir_is_deterministic`: `build_ir(1.0, 240, 48000)`（両呼び出し）。

新規テスト2件を `mod tests` 末尾に追加:
```rust
    #[test]
    fn compute_and_apply_with_rt60_predelay_key() {
        let cache = IrCache::new(48000);
        cache.compute_if_needed(1.5, 480);
        let mut signal: Vec<[f32; 2]> = (0..4800)
            .map(|i| { let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin() * 0.5; [v, v] })
            .collect();
        let before = signal[2000][0];
        cache.apply(&mut signal, 1.5, 480, 1.0, 0.8, 1.0, 0.1);
        assert!(signal.iter().any(|s| (s[0] - before).abs() > 1e-4), "apply は信号を変化させること");
    }

    #[test]
    fn apply_wet_base_zero_leaves_signal_unchanged() {
        let cache = IrCache::new(48000);
        cache.compute_if_needed(1.0, 240);
        let original: Vec<[f32; 2]> = (0..200).map(|i| [i as f32 * 0.01, i as f32 * 0.01]).collect();
        let mut signal = original.clone();
        cache.apply(&mut signal, 1.0, 240, 1.0, 0.0, 1.0, 0.1);
        for (a, b) in original.iter().zip(signal.iter()) { assert!((a[0] - b[0]).abs() < 1e-6); }
    }
```

- [ ] **Step 2: early.rs を RoomGeometry 駆動へ**

import を変更: `use crate::acoustics::RoomGeometry;` を追加し、`use crate::geometry::{...}` から `room_dims` を外す（`calc_geometry, directivity_pattern, image_position, Surface` は残す）。

`build_early_taps` のシグネチャ（`room_size: f64` → `geo: &RoomGeometry`）:
```rust
pub fn build_early_taps(
    mono: &[f32],
    distance: f64,
    pan_rad: f64,
    vol_factor: f64,
    audio: &AudioConfig,
    er: &EarlyConfig,
    geo: &RoomGeometry,
    sample_rate: u32,
    min_delay_direct: usize,
) -> Vec<EarlyTap> {
```
本体: `let dims = room_dims(room_size, er.room_dims_min, er.room_dims_max);` → `let dims = geo.dims;`。`er.listener_offset[0]`/`[1]` を `geo.listener_offset[0]`/`[1]` に置換。他は不変。

early.rs テスト更新: `mod tests` にヘルパー追加し各呼び出しを更新:
```rust
    fn geo_for(room_size: f64, er: &EarlyConfig) -> RoomGeometry {
        RoomGeometry {
            dims: crate::geometry::room_dims(room_size, er.room_dims_min, er.room_dims_max),
            listener_offset: er.listener_offset,
        }
    }
```
各テストの `build_early_taps(&mono, dist, pan, vol, &audio_cfg(), &er, 0.1, 48000, 0)` を
`build_early_taps(&mono, dist, pan, vol, &audio_cfg(), &er, &geo_for(0.1, &er), 48000, 0)` に変更（`disabled_returns_no_taps`, `only_surfaces_with_positive_coeff_produce_taps`, `floor_tap_delay_matches_analytic_value`, `panned_source_produces_left_right_asymmetric_taps`, `front_wall_reflection_coeff_scales_tap_gain`。`er` がローカルに無いテストは先に `let er = EarlyConfig::default();` を用意）。

- [ ] **Step 3: processor.rs を物理モデルへ配線**

冒頭 import に追加: `use crate::acoustics::{compute_reverb_params, resolve_room_geometry, ReverbParams, RoomGeometry};`

`process` 内（`resolve_reverb_params` の直後、旧 `self.ir_cache.compute_if_needed(room_size);` を置換）:
```rust
        let (room_size, reverb_wet) = resolve_reverb_params(cast, scene, self.config.room_size, self.config.reverb_wet);
        let geo: RoomGeometry = resolve_room_geometry(scene, &self.config.early_reflections, room_size);
        let rp: ReverbParams = compute_reverb_params(geo.dims, &self.config.early_reflections, self.config.sound_speed, self.config.sample_rate);
        self.ir_cache.compute_if_needed(rp.rt60, rp.pre_delay);
```
早期反射呼び出しの第7引数を `room_size` → `&geo`:
```rust
        let early_taps = build_early_taps(
            &mono, cast.distance, pan_rad, vol_factor,
            &self.config, &self.config.early_reflections,
            &geo, self.config.sample_rate, min_delay,
        );
```
残響テールのバッファ長（旧 `rv_time`/`rv_samples` ブロックを置換）:
```rust
        let reverb_active = reverb_wet > 0.0 && rp.wet_base > 0.0;
        let rv_samples = if reverb_active {
            (self.config.sample_rate as f64 * rp.rt60) as usize + rp.pre_delay
        } else { 0 };
```
リバーブ適用:
```rust
        self.ir_cache.apply(&mut stereo, rp.rt60, rp.pre_delay, reverb_wet, rp.wet_base, cast.distance, self.config.early_reflections.wet_distance_slope);
```
補助メソッド: 旧 `prewarm_ir_cache` を削除し、次の2メソッドを追加（`config_room_size`/`config_sample_rate` は維持）:
```rust
    /// scene と解決済み room_size から拡散リバーブの (rt60, pre_delay) を算出する。
    pub fn reverb_params_for(&self, scene: &SceneConfig, fallback_room_size: f64) -> (f64, usize) {
        let geo = resolve_room_geometry(scene, &self.config.early_reflections, fallback_room_size);
        let rp = compute_reverb_params(geo.dims, &self.config.early_reflections, self.config.sound_speed, self.config.sample_rate);
        (rp.rt60, rp.pre_delay)
    }

    /// (rt60, pre_delay) の集合で IR キャッシュを事前計算する。
    pub fn prewarm_reverb(&self, params: &[(f64, usize)]) {
        self.ir_cache.prewarm(params);
    }
```

- [ ] **Step 4: src/lib.rs のプリウォームを差し替え** — 現在の `room_sizes`/`prewarm_ir_cache` ブロックを:
```rust
        let reverb_params: Vec<(f64, usize)> = tasks.iter()
            .map(|(_, _, t)| {
                let rs = t.cast.params.get("room_size").and_then(|v| v.as_f64())
                    .or(t.scene_config.room_size)
                    .unwrap_or(self.audio_processor.config_room_size());
                self.audio_processor.reverb_params_for(&t.scene_config, rs)
            })
            .collect();
        self.audio_processor.prewarm_reverb(&reverb_params);
```

- [ ] **Step 5: 物理残響の統合テストを追加** — `processor.rs` の `mod tests` 末尾に:
```rust
    #[test]
    fn outdoor_scene_processes_with_near_dry_reverb() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.wav");
        write_noise_wav(&input, 48000, 0.1);
        let mut cfg = default_audio_config();
        cfg.early_reflections.enabled = false;
        cfg.early_reflections.ceiling.reflection_coeff = 0.0;
        cfg.early_reflections.front_wall.reflection_coeff = 0.0;
        cfg.early_reflections.back_wall.reflection_coeff = 0.0;
        cfg.early_reflections.side_walls.reflection_coeff = 0.0;
        cfg.early_reflections.floor.reflection_coeff = 0.5;
        let proc = AudioProcessor::new(cfg);
        let scene = SceneConfig { room_w: Some(30.0), room_d: Some(30.0), room_h: Some(15.0), reverb_wet: Some(1.0), ..SceneConfig::new("屋外") };
        let n = proc.process(&input, &dir.path().join("outdoor.wav"), &dummy_cast(0.0, 1.0), &scene).unwrap();
        assert!(n > 0, "屋外 scene でも処理が成功し出力が生成されること");
    }

    #[test]
    fn scene_room_dims_affect_reverb_params() {
        let proc = AudioProcessor::new(default_audio_config());
        let small = SceneConfig { room_w: Some(4.0), room_d: Some(5.0), room_h: Some(3.0), ..SceneConfig::new("小") };
        let big = SceneConfig { room_w: Some(25.0), room_d: Some(45.0), room_h: Some(18.0), ..SceneConfig::new("大") };
        let (rt_small, _) = proc.reverb_params_for(&small, 0.1);
        let (rt_big, _) = proc.reverb_params_for(&big, 0.1);
        assert!(rt_big > rt_small, "大きい部屋ほど残響長が長い: small={rt_small}, big={rt_big}");
    }
```

- [ ] **Step 6: 全体ビルドと全テスト** — Run: `cargo test --workspace` → 全 PASS（s2v-audio の reverb/early/acoustics/processor 統合と root crate を含む。process 系既存テストは残響挙動が変わるが energy>0/非対称/disabled の不変条件で通る）。失敗時は該当箇所を修正してから次へ。

- [ ] **Step 7: コミット**
```bash
git add crates/s2v-audio/src/reverb.rs crates/s2v-audio/src/early.rs crates/s2v-audio/src/processor.rs src/lib.rs
git commit -m "feat(audio): drive early reflections and reverb from physical room model"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Task 4: config.toml の reverb_wet をスケーラ既定へ

**Files:**
- Modify: `config.toml`
- Modify: `crates/s2v-core/src/config.rs`

- [ ] **Step 1: config.toml の reverb_wet を 1.0 に** — `config.toml` `[audio]` の `reverb_wet = 0.7` を `reverb_wet = 1.0` に変更（意味が「絶対量」→「物理基準値への倍率」に変わったため）。

- [ ] **Step 2: 実 config.toml のパース確認テスト** — `crates/s2v-core/src/config.rs` の `mod tests` 末尾に追加:
```rust
    #[test]
    fn real_config_reverb_wet_is_scaler_default() {
        let s = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../config.toml")).unwrap();
        let cfg = Config::from_toml(&s).unwrap();
        assert!((cfg.audio.reverb_wet - 1.0).abs() < 1e-10, "reverb_wet 既定はスケーラ 1.0");
    }
```
Run: `cargo test -p s2v-core real_config_reverb_wet_is_scaler_default` → PASS。

- [ ] **Step 3: 全体テスト** — Run: `cargo test --workspace` → PASS。

- [ ] **Step 4: コミット**
```bash
git add config.toml crates/s2v-core/src/config.rs
git commit -m "feat(audio): default reverb_wet to 1.0 as physical-baseline scaler"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Task 5: 全体ビルド・リリース確認（コミット不要）

- [ ] **Step 1: ワークスペース全体テスト** — Run: `cargo test --workspace` → 全 PASS（pass/fail 件数を記録）。
- [ ] **Step 2: リリースビルド** — Run: `cargo build --release` → 成功確認。
- [ ] **Step 3: スモーク** — Run: `cargo run -- <任意の台本.txt>`（`@scene` に `room_w/d/h` を含む台本で config パース・パイプライン起動を確認。エンジン未設定でも config 読み込み成功まで確認できる）。問題があれば該当タスクへ戻る。

---

## Self-Review メモ

- **Spec coverage**: §1=Task1(SceneConfig/parser)+Task2(resolve_room_geometry)。§2 RT60/pre_delay=Task2+Task3(build_ir)。§3 wet_base スケーラ=Task2+Task3(apply)+Task4(reverb_wet=1.0)。§4 scene構文=Task1。§5 IrCache再キー/prewarm=Task3。§6 早期反射RoomGeometry駆動=Task3。§7 屋外=Task2/Task3(test)。§8 互換=Task1(関数更新)+Task2/3(fallback)。§9 テスト=各Task。
- **Placeholder scan**: なし。各コードステップに実コード。
- **Type consistency**: `RoomGeometry{dims,listener_offset}` / `ReverbParams{rt60,pre_delay,wet_base}` / `resolve_room_geometry(scene,er,fallback_room_size)` / `compute_reverb_params(dims,er,sound_speed,sample_rate)`（Task2）、`IrCache::compute_if_needed(rt60,pre_delay)` / `prewarm(&[(f64,usize)])` / `apply(stereo,rt60,pre_delay,reverb_wet,wet_base,avg_dist,wet_distance_slope)` / `build_ir(rt60,pre_delay,fs)`（Task3）、`build_early_taps(...,&RoomGeometry,...)`（Task3）、`reverb_params_for(scene,fallback_room_size)->(f64,usize)` / `prewarm_reverb(&[(f64,usize)])`（Task3）は定義と使用で一致。
- **コンパイル順**: API変更が同一クレート/ルートを壊すため reverb+early+processor+lib を Task3 に統合し、末尾で `cargo test --workspace` 緑化後にコミット。Task1/2 は加算的で単独緑。
