# video_compose Rust 移植 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Python 製 `scripts/video_compose/` を Rust クレート `s2v-video` へ移植し、`script2voice compose` サブコマンドとして統合する(既存の音声生成起動は後方互換維持)。

**Architecture:** Python 4モジュール(srt_timing / scene_map / ffmpeg_cmd / compose_video)を新クレート `crates/s2v-video` の4モジュールへ 1:1 移植する。純粋ロジック(SRT解析・scene_map解決・ffmpegコマンド構築)は ffmpeg 非依存でユニットテストし、ffprobe/ffmpeg 実行部はオーケストレーション層に閉じる。`src/main.rs` は clap の「デフォルトコマンド + 任意サブコマンド」定石で compose を足す。

**Tech Stack:** Rust 2021 / clap 4(derive)/ serde + serde_json / regex / anyhow / tempfile(dev)。ffmpeg・ffprobe は PATH 上の外部プロセス。

## Global Constraints

- Python 版の**検証セマンティクスと日本語エラーメッセージ**を維持する(review.txt 由来の3修正:scene_map の相対パス基準=json配置ディレクトリ、compute_segments の負区間/非単調拒否、resolve_assets の未解決検出)。
- 出力 MP4 パラメータは現行踏襲:`-c:v libx264 -crf 18 -pix_fmt yuv420p -c:a aac -shortest -movflags +faststart`、解像度 1920x1080。
- ffmpeg/ffprobe はバンドルしない(PATH 前提)。
- 後方互換:`script2voice <台本...> [--config] [--strict]` は従来どおり動作すること。
- 依存はワークスペース既存のもの優先(anyhow/serde は `.workspace = true`)。
- コミットはタスクごと。テストは各モジュールの `#[cfg(test)]` に置く。

---

## File Structure

```
crates/s2v-video/
  Cargo.toml        # 新規
  src/
    lib.rs          # モジュール宣言・再エクスポート
    srt_timing.rs   # parse_paragraph_markers / compute_segments + tests
    scene_map.rs    # AssetKind/Asset/SceneMap, load/resolve/validate + tests
    ffmpeg_cmd.rs   # build_command(純粋) + tests
    compose.rs      # ComposeOptions, find_audio_file(+test), probe/run
Cargo.toml          # workspace members に s2v-video 追加、root deps に path 依存追加
src/main.rs         # clap 再構成 + compose サブコマンド分岐 + tests 更新
```

---

### Task 1: `s2v-video` クレートの雛形を作る

**Files:**
- Create: `crates/s2v-video/Cargo.toml`
- Create: `crates/s2v-video/src/lib.rs`
- Modify: `Cargo.toml`(workspace members)

**Interfaces:**
- Produces: クレート `s2v-video`(この時点では空モジュール4つを宣言)。

- [ ] **Step 1: Cargo.toml を作成**

`crates/s2v-video/Cargo.toml`:
```toml
[package]
name = "s2v-video"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow.workspace = true
serde = { workspace = true }
serde_json = "1"
regex = "1"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: 空の lib.rs を作成**

`crates/s2v-video/src/lib.rs`:
```rust
//! Script2Voice の音声・字幕・シーン画像から動画を合成するロジック。
//! Python 版 scripts/video_compose を移植したもの。
pub mod compose;
pub mod ffmpeg_cmd;
pub mod scene_map;
pub mod srt_timing;

pub use compose::ComposeOptions;
pub use scene_map::{Asset, AssetKind};
```

- [ ] **Step 3: 各モジュールの空ファイルを作成**

`crates/s2v-video/src/srt_timing.rs`、`scene_map.rs`、`ffmpeg_cmd.rs`、`compose.rs` を空(`//! placeholder` 1行)で作成する。

- [ ] **Step 4: workspace members に登録**

`Cargo.toml` の members に `"crates/s2v-video"` を追加する:
```toml
members = [".", "crates/s2v-core", "crates/s2v-engines", "crates/s2v-audio", "crates/s2v-export", "crates/s2v-gui", "crates/s2v-video"]
```

- [ ] **Step 5: ビルド確認**

Run: `cargo build -p s2v-video`
Expected: 成功(警告のみ許容)。

- [ ] **Step 6: Commit**

```bash
git add crates/s2v-video Cargo.toml
git commit -m "feat(s2v-video): scaffold crate for video_compose port"
```

---

### Task 2: `srt_timing` モジュール(SRT解析とセグメント計算)

**Files:**
- Modify: `crates/s2v-video/src/srt_timing.rs`

**Interfaces:**
- Produces:
  - `pub fn parse_paragraph_markers(srt_text: &str) -> Vec<f64>`
  - `pub fn compute_segments(marker_times: &[f64], total_duration_s: f64) -> anyhow::Result<Vec<(f64, f64)>>`

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-video/src/srt_timing.rs` の末尾に:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_start_times_in_order() {
        let srt = "1\n00:00:00,000 --> 00:00:01,500\nこんにちは\n\n\
                   2\n00:00:01,500 --> 00:00:01,500\n[PARAGRAPH]\n\n\
                   3\n00:00:03,000 --> 00:00:03,800\nさようなら\n\n\
                   4\n00:01:05,250 --> 00:01:05,250\n[PARAGRAPH]\n\n";
        assert_eq!(parse_paragraph_markers(srt), vec![1.5, 65.25]);
    }

    #[test]
    fn parse_returns_empty_when_no_markers() {
        let srt = "1\n00:00:00,000 --> 00:00:01,500\nこんにちは\n\n";
        assert!(parse_paragraph_markers(srt).is_empty());
    }

    #[test]
    fn segments_split_into_marker_count_plus_one() {
        let seg = compute_segments(&[1.5, 65.25], 120.0).unwrap();
        assert_eq!(seg, vec![(0.0, 1.5), (1.5, 65.25), (65.25, 120.0)]);
    }

    #[test]
    fn segments_single_when_no_markers() {
        assert_eq!(compute_segments(&[], 10.0).unwrap(), vec![(0.0, 10.0)]);
    }

    #[test]
    fn segments_reject_non_monotonic() {
        let e = compute_segments(&[5.0, 3.0], 10.0).unwrap_err();
        assert!(e.to_string().contains("単調増加"));
    }

    #[test]
    fn segments_reject_duplicate() {
        let e = compute_segments(&[3.0, 3.0], 10.0).unwrap_err();
        assert!(e.to_string().contains("単調増加"));
    }

    #[test]
    fn segments_reject_negative() {
        let e = compute_segments(&[-1.0], 10.0).unwrap_err();
        assert!(e.to_string().contains("負"));
    }

    #[test]
    fn segments_reject_beyond_total() {
        let e = compute_segments(&[15.0], 10.0).unwrap_err();
        assert!(e.to_string().contains("総時間"));
    }

    #[test]
    fn segments_reject_marker_at_zero() {
        let e = compute_segments(&[0.0, 5.0], 10.0).unwrap_err();
        assert!(e.to_string().contains("単調増加"));
    }
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p s2v-video srt_timing`
Expected: コンパイルエラー(関数未定義)。

- [ ] **Step 3: 実装を書く**

`crates/s2v-video/src/srt_timing.rs` の先頭に:
```rust
//! SRT字幕から [PARAGRAPH] マーカー時刻を抽出し、各表示セグメント(開始秒,終了秒)を計算する。
use std::sync::OnceLock;

use regex::Regex;

fn paragraph_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\d+\r?\n(\d{2}):(\d{2}):(\d{2}),(\d{3}) --> \d{2}:\d{2}:\d{2},\d{3}\r?\n\[PARAGRAPH\]",
        )
        .expect("PARAGRAPH 正規表現が不正")
    })
}

/// SRTテキストから [PARAGRAPH] エントリの開始時刻(秒)を出現順に返す。
pub fn parse_paragraph_markers(srt_text: &str) -> Vec<f64> {
    let mut markers = Vec::new();
    for cap in paragraph_re().captures_iter(srt_text) {
        let h: f64 = cap[1].parse().unwrap();
        let m: f64 = cap[2].parse().unwrap();
        let s: f64 = cap[3].parse().unwrap();
        let ms: f64 = cap[4].parse().unwrap();
        markers.push(h * 3600.0 + m * 60.0 + s + ms / 1000.0);
    }
    markers
}

/// マーカー時刻と音声総時間から各表示セグメントの (開始秒, 終了秒) を計算する。
/// マーカーは「0より大きい・単調増加・総時間以下」を満たさなければエラー。
pub fn compute_segments(
    marker_times: &[f64],
    total_duration_s: f64,
) -> anyhow::Result<Vec<(f64, f64)>> {
    let mut prev = 0.0_f64;
    for (i, &t) in marker_times.iter().enumerate() {
        let n = i + 1;
        if t < 0.0 {
            anyhow::bail!("{n}番目の[PARAGRAPH]マーカー時刻が負です: {t}");
        }
        if t <= prev {
            anyhow::bail!(
                "{n}番目の[PARAGRAPH]マーカー時刻が単調増加していません: {t} は直前の境界 {prev} 以下です"
            );
        }
        if t > total_duration_s {
            anyhow::bail!(
                "{n}番目の[PARAGRAPH]マーカー時刻が音声総時間を超えています: {t} > {total_duration_s}"
            );
        }
        prev = t;
    }
    let mut boundaries = Vec::with_capacity(marker_times.len() + 2);
    boundaries.push(0.0);
    boundaries.extend_from_slice(marker_times);
    boundaries.push(total_duration_s);
    Ok(boundaries.windows(2).map(|w| (w[0], w[1])).collect())
}
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p s2v-video srt_timing`
Expected: PASS(9件)。

- [ ] **Step 5: Commit**

```bash
git add crates/s2v-video/src/srt_timing.rs
git commit -m "feat(s2v-video): port srt_timing (paragraph markers + segments)"
```

---

### Task 3: `scene_map` モジュール(scene_map.json 解決)

**Files:**
- Modify: `crates/s2v-video/src/scene_map.rs`

**Interfaces:**
- Produces:
  - `pub enum AssetKind { Image, Video }`(derive: Debug, Clone, Copy, PartialEq, Eq)
  - `pub struct Asset { pub kind: AssetKind, pub path: String, pub source_duration: Option<f64> }`(derive: Debug, Clone, PartialEq)
  - `pub struct SceneMap { pub paragraphs: Vec<ParagraphEntry>, pub default_image: Option<String> }`
  - `pub fn load_scene_map(path: &std::path::Path) -> anyhow::Result<SceneMap>`
  - `pub fn resolve_assets(scene_map: &SceneMap, segment_count: usize) -> anyhow::Result<Vec<Asset>>`
  - `pub fn resolve_asset_paths(assets: Vec<Asset>, base_dir: &std::path::Path) -> Vec<Asset>`
  - `pub fn validate_assets_exist(assets: &[Asset]) -> anyhow::Result<()>`

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-video/src/scene_map.rs` の末尾に:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn img(p: &str) -> Asset {
        Asset { kind: AssetKind::Image, path: p.into(), source_duration: None }
    }
    fn vid(p: &str) -> Asset {
        Asset { kind: AssetKind::Video, path: p.into(), source_duration: None }
    }
    fn sm_from(json: &str) -> SceneMap {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn load_reads_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scene_map.json");
        std::fs::write(&path, r#"{"paragraphs":[{"index":1,"image":"images/scene01.png"}],"default_image":"images/default.png"}"#).unwrap();
        let sm = load_scene_map(&path).unwrap();
        assert_eq!(sm.default_image.as_deref(), Some("images/default.png"));
    }

    #[test]
    fn resolve_legacy_image_form() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"image":"images/scene01.png"},{"index":2,"image":"images/scene02.png"}],"default_image":"images/default.png"}"#);
        assert_eq!(
            resolve_assets(&sm, 2).unwrap(),
            vec![img("images/scene01.png"), img("images/scene02.png")]
        );
    }

    #[test]
    fn resolve_type_path_form_with_video() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"type":"video","path":"assets/p01.mp4"},{"index":2,"path":"assets/p02.png"}],"default_image":"assets/default.png"}"#);
        assert_eq!(
            resolve_assets(&sm, 2).unwrap(),
            vec![vid("assets/p01.mp4"), img("assets/p02.png")]
        );
    }

    #[test]
    fn resolve_falls_back_to_default_for_missing_index() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"image":"images/scene01.png"}],"default_image":"images/default.png"}"#);
        assert_eq!(
            resolve_assets(&sm, 3).unwrap(),
            vec![img("images/scene01.png"), img("images/default.png"), img("images/default.png")]
        );
    }

    #[test]
    fn resolve_errors_when_default_missing_for_gap() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"image":"a.png"}]}"#);
        let e = resolve_assets(&sm, 2).unwrap_err();
        assert!(e.to_string().contains("2"));
    }

    #[test]
    fn resolve_errors_on_duplicate_index() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"image":"a.png"},{"index":1,"image":"b.png"}],"default_image":"d.png"}"#);
        let e = resolve_assets(&sm, 1).unwrap_err();
        assert!(e.to_string().contains("重複"));
    }

    #[test]
    fn resolve_errors_when_index_out_of_range() {
        let sm = sm_from(r#"{"paragraphs":[{"index":5,"image":"a.png"}],"default_image":"d.png"}"#);
        let e = resolve_assets(&sm, 2).unwrap_err();
        assert!(e.to_string().contains("5"));
    }

    #[test]
    fn resolve_errors_on_invalid_type() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"type":"audio","path":"a.mp3"}],"default_image":"d.png"}"#);
        let e = resolve_assets(&sm, 1).unwrap_err();
        assert!(e.to_string().contains("type"));
    }

    #[test]
    fn validate_errors_with_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present.png");
        std::fs::write(&present, b"").unwrap();
        let missing = dir.path().join("missing.png");
        let assets = vec![
            img(present.to_str().unwrap()),
            img(missing.to_str().unwrap()),
        ];
        let e = validate_assets_exist(&assets).unwrap_err();
        assert!(e.to_string().contains("missing.png"));
    }

    #[test]
    fn validate_passes_when_all_exist() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present.png");
        std::fs::write(&present, b"").unwrap();
        validate_assets_exist(&[img(present.to_str().unwrap())]).unwrap();
    }

    #[test]
    fn resolve_paths_makes_relative_to_base_dir() {
        let base = Path::new("/project/subdir");
        let resolved = resolve_asset_paths(
            vec![img("images/scene01.png"), vid("assets/p01.mp4")],
            base,
        );
        assert_eq!(resolved[0].path, base.join("images/scene01.png").to_string_lossy());
        assert_eq!(resolved[1].path, base.join("assets/p01.mp4").to_string_lossy());
    }

    #[test]
    fn resolve_paths_leaves_absolute_untouched() {
        let base = Path::new("/project/subdir");
        let absolute = Path::new("/elsewhere/default.png").to_string_lossy().into_owned();
        let resolved = resolve_asset_paths(vec![img(&absolute)], base);
        assert_eq!(resolved[0].path, absolute);
    }

    #[test]
    fn resolve_paths_preserves_other_fields() {
        let base = Path::new("/project");
        let asset = Asset { kind: AssetKind::Video, path: "p01.mp4".into(), source_duration: Some(5.0) };
        let resolved = resolve_asset_paths(vec![asset], base);
        assert_eq!(resolved[0].kind, AssetKind::Video);
        assert_eq!(resolved[0].source_duration, Some(5.0));
    }
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p s2v-video scene_map`
Expected: コンパイルエラー(型・関数未定義)。

- [ ] **Step 3: 実装を書く**

`crates/s2v-video/src/scene_map.rs` の先頭に:
```rust
//! scene_map.json を読み込み、表示セグメント番号(1始まり)に対応するアセットを解決する。
use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Image,
    Video,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub kind: AssetKind,
    pub path: String,
    pub source_duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct ParagraphEntry {
    pub index: i64,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    pub path: Option<String>,
    pub image: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SceneMap {
    #[serde(default)]
    pub paragraphs: Vec<ParagraphEntry>,
    #[serde(default)]
    pub default_image: Option<String>,
}

/// scene_map.json を読み込む。
pub fn load_scene_map(path: &Path) -> anyhow::Result<SceneMap> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("scene_map.json を読めません {}: {e}", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

/// scene_map.json の1エントリを (type文字列, path) に正規化する。
/// 新形式(type+path)と旧形式(imageキーのみ、常にimage扱い)の両方を受ける。
fn normalize_entry(entry: &ParagraphEntry) -> anyhow::Result<(String, String)> {
    if let Some(path) = &entry.path {
        Ok((entry.type_.clone().unwrap_or_else(|| "image".to_string()), path.clone()))
    } else if let Some(image) = &entry.image {
        Ok(("image".to_string(), image.clone()))
    } else {
        anyhow::bail!("scene_map.json: 段落番号 {} に path も image もありません", entry.index)
    }
}

/// セグメント番号 1..=segment_count に対応するアセット列を返す。
pub fn resolve_assets(scene_map: &SceneMap, segment_count: usize) -> anyhow::Result<Vec<Asset>> {
    let mut by_index: HashMap<i64, (String, String)> = HashMap::new();
    for entry in &scene_map.paragraphs {
        if by_index.contains_key(&entry.index) {
            anyhow::bail!("scene_map.json: 段落番号 {} が重複しています", entry.index);
        }
        by_index.insert(entry.index, normalize_entry(entry)?);
    }

    for (index, (type_str, _)) in &by_index {
        if !(1..=segment_count as i64).contains(index) {
            anyhow::bail!(
                "scene_map.json: 段落番号 {index} はSRTの段落数(1..{segment_count})の範囲外です"
            );
        }
        if type_str != "image" && type_str != "video" {
            anyhow::bail!(
                "scene_map.json: 段落番号 {index} の type が不正です: {type_str:?} (有効な値: image, video)"
            );
        }
    }

    let default_asset = scene_map.default_image.as_ref().map(|p| Asset {
        kind: AssetKind::Image,
        path: p.clone(),
        source_duration: None,
    });

    let mut result = Vec::with_capacity(segment_count);
    for index in 1..=segment_count as i64 {
        let asset = if let Some((type_str, path)) = by_index.get(&index) {
            Asset {
                kind: if type_str == "video" { AssetKind::Video } else { AssetKind::Image },
                path: path.clone(),
                source_duration: None,
            }
        } else if let Some(d) = &default_asset {
            d.clone()
        } else {
            anyhow::bail!(
                "scene_map.json: 段落番号 {index} に対応するアセットが無く、default_image も設定されていません"
            );
        };
        result.push(asset);
    }
    Ok(result)
}

/// 相対パスを base_dir(scene_map.json の置かれたディレクトリ)基準の絶対パスへ揃える。
pub fn resolve_asset_paths(assets: Vec<Asset>, base_dir: &Path) -> Vec<Asset> {
    assets
        .into_iter()
        .map(|a| {
            let p = Path::new(&a.path);
            let path = if p.is_absolute() {
                a.path.clone()
            } else {
                base_dir.join(p).to_string_lossy().into_owned()
            };
            Asset { path, ..a }
        })
        .collect()
}

/// 解決済みアセットの参照先がすべて実在することを検証する。
pub fn validate_assets_exist(assets: &[Asset]) -> anyhow::Result<()> {
    let missing: Vec<&str> = assets
        .iter()
        .filter(|a| !Path::new(&a.path).exists())
        .map(|a| a.path.as_str())
        .collect();
    if !missing.is_empty() {
        anyhow::bail!("scene_map.json が参照するアセットが見つかりません: {}", missing.join(", "));
    }
    Ok(())
}
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p s2v-video scene_map`
Expected: PASS(13件)。

- [ ] **Step 5: Commit**

```bash
git add crates/s2v-video/src/scene_map.rs
git commit -m "feat(s2v-video): port scene_map (asset resolution + validation)"
```

---

### Task 4: `ffmpeg_cmd` モジュール(ffmpeg コマンド構築)

**Files:**
- Modify: `crates/s2v-video/src/ffmpeg_cmd.rs`

**Interfaces:**
- Consumes: `crate::scene_map::{Asset, AssetKind}`
- Produces:
  - `pub fn build_command(audio_path: &Path, assets: &[Asset], durations: &[f64], output_path: &Path, burn_subtitle_path: Option<&Path>) -> anyhow::Result<Vec<String>>`
  - モジュール定数 `WIDTH=1920`、`HEIGHT=1080`、`CRF=18`(Python の kwargs は未使用のため定数化=YAGNI)。

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-video/src/ffmpeg_cmd.rs` の末尾に:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_map::{Asset, AssetKind};
    use std::path::Path;

    fn img(p: &str) -> Asset {
        Asset { kind: AssetKind::Image, path: p.into(), source_duration: None }
    }
    fn vid(p: &str, d: f64) -> Asset {
        Asset { kind: AssetKind::Video, path: p.into(), source_duration: Some(d) }
    }
    fn filter_of(cmd: &[String]) -> &str {
        let i = cmd.iter().position(|a| a == "-filter_complex").unwrap();
        &cmd[i + 1]
    }
    fn input_paths(cmd: &[String]) -> Vec<&str> {
        cmd.iter().enumerate().filter(|(_, a)| *a == "-i").map(|(i, _)| cmd[i + 1].as_str()).collect()
    }

    #[test]
    fn feeds_audio_then_looped_timed_images() {
        let cmd = build_command(
            Path::new("project/full_dialogue.wav"),
            &[img("images/scene01.png"), img("images/scene02.png")],
            &[1.5, 63.75],
            Path::new("project/output.mp4"),
            None,
        )
        .unwrap();
        assert_eq!(cmd[0], "ffmpeg");
        let inputs = input_paths(&cmd);
        assert_eq!(inputs[0], "project/full_dialogue.wav");
        assert_eq!(inputs[1], "images/scene01.png");
        let loop_idx = cmd.iter().position(|a| a == "-loop").unwrap();
        assert_eq!(&cmd[loop_idx..loop_idx + 4], &["-loop", "1", "-t", "1.500"]);
    }

    #[test]
    fn filter_scales_pads_and_concats_all_images() {
        let cmd = build_command(
            Path::new("a.wav"),
            &[img("s1.png"), img("s2.png"), img("s3.png")],
            &[1.0, 2.0, 3.0],
            Path::new("out.mp4"),
            None,
        )
        .unwrap();
        let f = filter_of(&cmd);
        assert!(f.contains("[1:v]scale=1920:1080:force_original_aspect_ratio=decrease"));
        assert!(f.contains("[v1][v2][v3]concat=n=3:v=1:a=0[vout]"));
        let map_idx = cmd.iter().position(|a| a == "-map").unwrap();
        assert_eq!(cmd[map_idx + 1], "[vout]");
        assert!(cmd.iter().any(|a| a == "0:a"));
    }

    #[test]
    fn appends_subtitles_filter_and_remaps_when_burn_given() {
        let cmd = build_command(
            Path::new("a.wav"),
            &[img("s1.png")],
            &[5.0],
            Path::new("out.mp4"),
            Some(Path::new("project/timeline/subtitles.srt")),
        )
        .unwrap();
        let f = filter_of(&cmd);
        assert!(f.contains("[vout]subtitles="));
        assert!(f.contains("[vsub]"));
        let map_idx = cmd.iter().position(|a| a == "-map").unwrap();
        assert_eq!(cmd[map_idx + 1], "[vsub]");
    }

    #[test]
    fn uses_libx264_crf18_shortest() {
        let cmd = build_command(Path::new("a.wav"), &[img("s1.png")], &[5.0], Path::new("out.mp4"), None).unwrap();
        let cv = cmd.iter().position(|a| a == "-c:v").unwrap();
        assert_eq!(cmd[cv + 1], "libx264");
        let crf = cmd.iter().position(|a| a == "-crf").unwrap();
        assert_eq!(cmd[crf + 1], "18");
        assert!(cmd.iter().any(|a| a == "-shortest"));
        assert_eq!(cmd.last().unwrap(), "out.mp4");
    }

    #[test]
    fn forces_yuv420p_and_faststart() {
        let cmd = build_command(Path::new("a.wav"), &[img("s1.png")], &[5.0], Path::new("out.mp4"), None).unwrap();
        let pf = cmd.iter().position(|a| a == "-pix_fmt").unwrap();
        assert_eq!(cmd[pf + 1], "yuv420p");
        let mf = cmd.iter().position(|a| a == "-movflags").unwrap();
        assert_eq!(cmd[mf + 1], "+faststart");
    }

    #[test]
    fn errors_on_length_mismatch() {
        let e = build_command(Path::new("a.wav"), &[img("s1.png"), img("s2.png")], &[1.0], Path::new("out.mp4"), None).unwrap_err();
        assert!(e.to_string().contains("同じ長さ"));
    }

    #[test]
    fn errors_on_zero_or_negative_duration() {
        let e = build_command(Path::new("a.wav"), &[img("s1.png"), img("s2.png")], &[1.0, 0.0], Path::new("out.mp4"), None).unwrap_err();
        assert!(e.to_string().contains("duration"));
        let e = build_command(Path::new("a.wav"), &[img("s1.png")], &[-1.0], Path::new("out.mp4"), None).unwrap_err();
        assert!(e.to_string().contains("duration"));
    }

    #[test]
    fn feeds_video_clip_without_loop_flag() {
        let cmd = build_command(Path::new("a.wav"), &[vid("assets/p01.mp4", 10.0)], &[5.0], Path::new("out.mp4"), None).unwrap();
        assert!(!cmd.iter().any(|a| a == "-loop"));
        let t = cmd.iter().position(|a| a == "-t").unwrap();
        assert_eq!(&cmd[t..t + 4], &["-t", "5.000", "-i", "assets/p01.mp4"]);
    }

    #[test]
    fn trims_video_longer_than_duration_without_tpad() {
        let cmd = build_command(Path::new("a.wav"), &[vid("assets/p01.mp4", 10.0)], &[5.0], Path::new("out.mp4"), None).unwrap();
        assert!(!filter_of(&cmd).contains("tpad"));
    }

    #[test]
    fn freezes_last_frame_for_video_shorter_than_duration() {
        let cmd = build_command(Path::new("a.wav"), &[vid("assets/p01.mp4", 3.0)], &[5.0], Path::new("out.mp4"), None).unwrap();
        let f = filter_of(&cmd);
        assert!(f.contains("tpad=stop_mode=clone:stop_duration=2.000"));
        assert!(f.contains("[v1pre]tpad=stop_mode=clone:stop_duration=2.000[v1]"));
    }

    #[test]
    fn mixes_image_and_video_in_concat() {
        let cmd = build_command(Path::new("a.wav"), &[img("s1.png"), vid("assets/p02.mp4", 8.0)], &[1.0, 2.0], Path::new("out.mp4"), None).unwrap();
        assert!(filter_of(&cmd).contains("[v1][v2]concat=n=2:v=1:a=0[vout]"));
    }
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p s2v-video ffmpeg_cmd`
Expected: コンパイルエラー(build_command 未定義)。

- [ ] **Step 3: 実装を書く**

`crates/s2v-video/src/ffmpeg_cmd.rs` の先頭に:
```rust
//! FFmpeg のスライドショー合成コマンドを構築する純粋関数(実行はしない)。
use std::path::Path;

use crate::scene_map::{Asset, AssetKind};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const CRF: u32 = 18;

fn scale_pad_filter(label_in: &str, label_out: &str) -> String {
    format!(
        "[{label_in}]scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=decrease,\
         pad={WIDTH}:{HEIGHT}:(ow-iw)/2:(oh-ih)/2,setsar=1,setpts=PTS-STARTPTS[{label_out}]"
    )
}

/// ffmpeg subtitles フィルタ用にパスをエスケープする(Windowsのドライブレターのコロン対策)。
fn escape_subtitles_path(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\\', "/").replace(':', "\\:");
    format!("'{escaped}'")
}

/// ffmpeg コマンドを引数リストとして構築する。
/// 画像は静止ループ、動画クリップは source_duration が duration 以上ならトリミング、
/// 未満なら不足分を最終フレーム静止(tpad)で埋める。burn_subtitle_path 指定で字幕焼き込み。
pub fn build_command(
    audio_path: &Path,
    assets: &[Asset],
    durations: &[f64],
    output_path: &Path,
    burn_subtitle_path: Option<&Path>,
) -> anyhow::Result<Vec<String>> {
    if assets.len() != durations.len() {
        anyhow::bail!("assets と durations は同じ長さでなければならない");
    }
    if assets.is_empty() {
        anyhow::bail!("assets が空です");
    }
    for (i, &d) in durations.iter().enumerate() {
        if d <= 0.0 {
            anyhow::bail!("durations[{i}] は0より大きくなければなりません: {d}");
        }
    }

    let mut cmd: Vec<String> = vec!["ffmpeg".into(), "-y".into(), "-i".into(), audio_path.to_string_lossy().into_owned()];
    for (asset, &duration) in assets.iter().zip(durations) {
        match asset.kind {
            AssetKind::Video => {
                cmd.push("-t".into());
                cmd.push(format!("{duration:.3}"));
                cmd.push("-i".into());
                cmd.push(asset.path.clone());
            }
            AssetKind::Image => {
                cmd.push("-loop".into());
                cmd.push("1".into());
                cmd.push("-t".into());
                cmd.push(format!("{duration:.3}"));
                cmd.push("-i".into());
                cmd.push(asset.path.clone());
            }
        }
    }

    let mut filter_parts: Vec<String> = Vec::new();
    let mut video_labels: Vec<String> = Vec::new();
    for (i, (asset, &duration)) in assets.iter().zip(durations).enumerate() {
        let in_label = format!("{}:v", i + 1);
        let out_label = format!("v{}", i + 1);
        let source_duration = if asset.kind == AssetKind::Video { asset.source_duration } else { None };
        let deficit = source_duration.map(|s| duration - s).unwrap_or(0.0);
        if deficit > 0.0 {
            let scaled = format!("v{}pre", i + 1);
            filter_parts.push(scale_pad_filter(&in_label, &scaled));
            filter_parts.push(format!(
                "[{scaled}]tpad=stop_mode=clone:stop_duration={deficit:.3}[{out_label}]"
            ));
        } else {
            filter_parts.push(scale_pad_filter(&in_label, &out_label));
        }
        video_labels.push(format!("[{out_label}]"));
    }

    let concat_inputs = video_labels.join("");
    filter_parts.push(format!("{concat_inputs}concat=n={}:v=1:a=0[vout]", assets.len()));

    let mut final_video_label = "[vout]".to_string();
    if let Some(sub) = burn_subtitle_path {
        filter_parts.push(format!("[vout]subtitles={}[vsub]", escape_subtitles_path(sub)));
        final_video_label = "[vsub]".to_string();
    }

    cmd.push("-filter_complex".into());
    cmd.push(filter_parts.join(";"));
    cmd.push("-map".into());
    cmd.push(final_video_label);
    cmd.push("-map".into());
    cmd.push("0:a".into());
    cmd.extend(["-c:v", "libx264", "-crf"].iter().map(|s| s.to_string()));
    cmd.push(CRF.to_string());
    cmd.extend(["-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", "-movflags", "+faststart"].iter().map(|s| s.to_string()));
    cmd.push(output_path.to_string_lossy().into_owned());
    Ok(cmd)
}
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p s2v-video ffmpeg_cmd`
Expected: PASS(11件)。

- [ ] **Step 5: Commit**

```bash
git add crates/s2v-video/src/ffmpeg_cmd.rs
git commit -m "feat(s2v-video): port ffmpeg_cmd (slideshow command builder)"
```

---

### Task 5: `compose` モジュール(オーケストレーション)

**Files:**
- Modify: `crates/s2v-video/src/compose.rs`

**Interfaces:**
- Consumes: `crate::srt_timing`, `crate::scene_map`, `crate::ffmpeg_cmd::build_command`
- Produces:
  - `pub struct ComposeOptions { pub project_dir: PathBuf, pub scene_map: Option<PathBuf>, pub burn_subtitle: bool, pub output: Option<PathBuf> }`
  - `pub fn run(opts: &ComposeOptions) -> anyhow::Result<()>`
  - `fn find_audio_file(project_dir: &Path) -> anyhow::Result<PathBuf>`(テスト対象)
  - `fn probe_duration_seconds(media_path: &Path) -> anyhow::Result<f64>`(ffprobe 実行、テスト対象外)

- [ ] **Step 1: 失敗するテストを書く(find_audio_file のみ)**

`crates/s2v-video/src/compose.rs` の末尾に:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_audio_prefers_wav_over_mp3() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("full_dialogue.wav"), b"").unwrap();
        std::fs::write(dir.path().join("full_dialogue.mp3"), b"").unwrap();
        assert_eq!(find_audio_file(dir.path()).unwrap(), dir.path().join("full_dialogue.wav"));
    }

    #[test]
    fn find_audio_falls_back_to_mp3() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("full_dialogue.mp3"), b"").unwrap();
        assert_eq!(find_audio_file(dir.path()).unwrap(), dir.path().join("full_dialogue.mp3"));
    }

    #[test]
    fn find_audio_errors_when_neither_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_audio_file(dir.path()).is_err());
    }
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p s2v-video compose`
Expected: コンパイルエラー(find_audio_file 未定義)。

- [ ] **Step 3: 実装を書く**

`crates/s2v-video/src/compose.rs` の先頭に:
```rust
//! Script2Voice の出力(音声 + [PARAGRAPH]入りSRT)とシーン画像から動画を生成する。
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::ffmpeg_cmd::build_command;
use crate::scene_map::{load_scene_map, resolve_asset_paths, resolve_assets, validate_assets_exist, AssetKind};
use crate::srt_timing::{compute_segments, parse_paragraph_markers};

/// compose サブコマンドのオプション。
pub struct ComposeOptions {
    pub project_dir: PathBuf,
    pub scene_map: Option<PathBuf>,
    pub burn_subtitle: bool,
    pub output: Option<PathBuf>,
}

/// project_dir 内の音声ファイルを探す。full_dialogue.wav を優先、無ければ .mp3。
fn find_audio_file(project_dir: &Path) -> anyhow::Result<PathBuf> {
    for name in ["full_dialogue.wav", "full_dialogue.mp3"] {
        let candidate = project_dir.join(name);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("{} に full_dialogue.wav / full_dialogue.mp3 が見つかりません", project_dir.display())
}

/// ffprobe でメディアの総再生時間(秒)を取得する。
fn probe_duration_seconds(media_path: &Path) -> anyhow::Result<f64> {
    let output = std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(media_path)
        .output()
        .with_context(|| format!("ffprobe の起動に失敗しました: {}", media_path.display()))?;
    if !output.status.success() {
        anyhow::bail!("ffprobe が失敗しました: {}", media_path.display());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let dur: f64 = text
        .trim()
        .parse()
        .with_context(|| format!("ffprobe 出力を数値に変換できません: {text:?}"))?;
    Ok(dur)
}

/// 動画を合成する。
pub fn run(opts: &ComposeOptions) -> anyhow::Result<()> {
    let project_dir = &opts.project_dir;
    let scene_map_path = opts
        .scene_map
        .clone()
        .unwrap_or_else(|| project_dir.join("scene_map.json"));
    let output_path = opts
        .output
        .clone()
        .unwrap_or_else(|| project_dir.join("output.mp4"));
    let srt_path = project_dir.join("timeline").join("subtitles.srt");

    let audio_path = find_audio_file(project_dir)?;
    let srt_text = std::fs::read_to_string(&srt_path)
        .with_context(|| format!("字幕を読めません: {}", srt_path.display()))?;
    let markers = parse_paragraph_markers(&srt_text);
    let total_duration = probe_duration_seconds(&audio_path)?;
    let segments = compute_segments(&markers, total_duration)?;

    let scene_map = load_scene_map(&scene_map_path)?;
    let mut assets = resolve_assets(&scene_map, segments.len())?;
    let base_dir = scene_map_path
        .canonicalize()
        .unwrap_or_else(|_| scene_map_path.clone());
    let base_dir = base_dir.parent().unwrap_or(Path::new("."));
    assets = resolve_asset_paths(assets, base_dir);
    validate_assets_exist(&assets)?;
    for asset in assets.iter_mut() {
        if asset.kind == AssetKind::Video {
            asset.source_duration = Some(probe_duration_seconds(Path::new(&asset.path))?);
        }
    }

    let durations: Vec<f64> = segments.iter().map(|(s, e)| e - s).collect();
    let burn = if opts.burn_subtitle { Some(srt_path.as_path()) } else { None };
    let cmd = build_command(&audio_path, &assets, &durations, &output_path, burn)?;

    let status = std::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .status()
        .context("ffmpeg の起動に失敗しました")?;
    if !status.success() {
        anyhow::bail!("ffmpeg が失敗しました (exit={:?})", status.code());
    }
    println!("動画を生成しました: {}", output_path.display());
    Ok(())
}
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p s2v-video`
Expected: PASS(全モジュール、find_audio 3件含む)。

- [ ] **Step 5: Commit**

```bash
git add crates/s2v-video/src/compose.rs
git commit -m "feat(s2v-video): port compose orchestration (ffprobe + ffmpeg run)"
```

---

### Task 6: CLI 統合(`compose` サブコマンド + 後方互換)

**Files:**
- Modify: `Cargo.toml`(root deps に `s2v-video` 追加)
- Modify: `src/main.rs`(clap 再構成・分岐・テスト更新)

**Interfaces:**
- Consumes: `s2v_video::compose::run`、`s2v_video::ComposeOptions`

- [ ] **Step 1: root Cargo.toml に依存追加**

`Cargo.toml` の `[dependencies]` に追加:
```toml
s2v-video   = { path = "crates/s2v-video" }
```

- [ ] **Step 2: main.rs の Cli を再構成する(失敗するテストを含む)**

`src/main.rs` の `#[derive(Parser)] struct Cli { ... }` ブロック(19-33行付近)を次で置き換える:
```rust
#[derive(Parser)]
#[command(name = "script2voice", version, about = "台本から音声・字幕・タイムラインを生成する")]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    generate: GenerateArgs,
}

/// 音声・字幕生成(デフォルト動作)の引数。
#[derive(clap::Args)]
struct GenerateArgs {
    /// 台本ファイルまたはフォルダ（複数指定可。フォルダは直下の .txt を名前順に処理）
    #[arg(required = true, num_args = 1..)]
    scripts: Vec<PathBuf>,

    /// 設定ファイル (config.toml) のパス。省略時は実行ファイルと同じディレクトリの config.toml を使用する
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// パース警告(未定義キャストの飲み込みなど)が1件でもある台本を失敗として扱う
    #[arg(long)]
    strict: bool,
}

#[derive(clap::Subcommand)]
enum Command {
    /// 音声・字幕とシーン画像(scene_map.json)から動画を合成する
    Compose(ComposeArgs),
}

/// 動画合成サブコマンドの引数。
#[derive(clap::Args)]
struct ComposeArgs {
    /// Script2Voice の出力ディレクトリ
    project_dir: PathBuf,

    /// scene_map.json のパス (省略時は <project_dir>/scene_map.json)
    #[arg(long)]
    scene_map: Option<PathBuf>,

    /// 字幕を動画に焼き込む
    #[arg(long)]
    burn_subtitle: bool,

    /// 出力先 MP4 (省略時は <project_dir>/output.mp4)
    #[arg(short, long)]
    output: Option<PathBuf>,
}
```

- [ ] **Step 3: main() 冒頭に compose 分岐を追加する**

`src/main.rs` の `async fn main()` 内、`let cli = Cli::parse();` の直後に次を挿入する:
```rust
    if let Some(Command::Compose(args)) = cli.command {
        return s2v_video::compose::run(&s2v_video::ComposeOptions {
            project_dir: args.project_dir,
            scene_map: args.scene_map,
            burn_subtitle: args.burn_subtitle,
            output: args.output,
        });
    }
```
(compose は同期関数。`#[tokio::main]` 内から直接呼んで良い。ログ初期化前に返すのを避けるため、この分岐は `init_logging()` より後に置いてもよいが、compose は run.log を作らないため `Cli::parse()` 直後で問題ない。)

- [ ] **Step 4: 既存 generate フローを cli.generate 参照へ書き換える**

`src/main.rs` の以下3箇所を修正する:
- `expand_script_args(&cli.scripts)` → `expand_script_args(&cli.generate.scripts)`
- `resolve_config_path(cli.config.clone(), ...)` → `resolve_config_path(cli.generate.config.clone(), ...)`
- `parse_all(&scripts, cli.strict)` → `parse_all(&scripts, cli.generate.strict)`

- [ ] **Step 5: 既存 CLI テストを新構造へ更新する**

`src/main.rs` の `mod tests` 内、以下を書き換える:
```rust
    #[test]
    fn parses_multiple_script_paths() {
        let cli = Cli::try_parse_from(["script2voice", "a.txt", "b.txt"]).unwrap();
        assert_eq!(cli.generate.scripts, vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]);
        assert_eq!(cli.generate.config, None);
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_custom_config_path() {
        let cli = Cli::try_parse_from(["script2voice", "script.txt", "--config", "custom.toml"]).unwrap();
        assert_eq!(cli.generate.config, Some(std::path::PathBuf::from("custom.toml")));
    }

    #[test]
    fn strict_flag_defaults_to_false_and_can_be_set() {
        let cli = Cli::try_parse_from(["script2voice", "a.txt"]).unwrap();
        assert!(!cli.generate.strict);
        let cli = Cli::try_parse_from(["script2voice", "a.txt", "--strict"]).unwrap();
        assert!(cli.generate.strict);
    }
```
(`fails_without_script_argument` はそのまま:サブコマンド無し・台本無しは依然エラー。)

- [ ] **Step 6: compose サブコマンドの新テストを追加する**

`src/main.rs` の `mod tests` に追加:
```rust
    #[test]
    fn parses_compose_subcommand_with_defaults() {
        let cli = Cli::try_parse_from(["script2voice", "compose", "myproject"]).unwrap();
        match cli.command {
            Some(Command::Compose(a)) => {
                assert_eq!(a.project_dir, PathBuf::from("myproject"));
                assert_eq!(a.scene_map, None);
                assert_eq!(a.output, None);
                assert!(!a.burn_subtitle);
            }
            _ => panic!("compose サブコマンドとして解釈されるべき"),
        }
    }

    #[test]
    fn parses_compose_subcommand_with_overrides() {
        let cli = Cli::try_parse_from([
            "script2voice", "compose", "myproject",
            "--scene-map", "custom_map.json", "--burn-subtitle", "-o", "final.mp4",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Compose(a)) => {
                assert_eq!(a.scene_map, Some(PathBuf::from("custom_map.json")));
                assert!(a.burn_subtitle);
                assert_eq!(a.output, Some(PathBuf::from("final.mp4")));
            }
            _ => panic!("compose サブコマンドとして解釈されるべき"),
        }
    }

    #[test]
    fn bare_scripts_still_parse_as_generate() {
        let cli = Cli::try_parse_from(["script2voice", "台本.txt"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.generate.scripts, vec![PathBuf::from("台本.txt")]);
    }
```

- [ ] **Step 7: テストとビルドを確認**

Run: `cargo test --workspace --all-targets`
Expected: PASS(既存 + 新規 compose テスト含む全件)。ビルド警告なし。

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml src/main.rs
git commit -m "feat(cli): add 'compose' subcommand backed by s2v-video (keeps default generate)"
```

---

### Task 7: 等価性確認 → Python 一式削除 → 本リポジトリ設定更新

**Files:**
- Delete: `scripts/video_compose/`(ディレクトリごと)
- Modify: `.claude/settings.local.json`(compose_video.py の python 許可行を撤去)

**Interfaces:** なし(検証と後始末)。

- [ ] **Step 1: clippy を確認**

Run: `cargo clippy --workspace --all-targets`
Expected: 新規警告なし。あれば修正してから続行。

- [ ] **Step 2: 実データで等価性を確認(/verify)**

`superpowers` の verify スキル(または手動)で、実在プロジェクトに対し Rust 版 compose を実行する。例:
```bash
cargo run --release -- compose "D:/UDS/YouTube/三人寄れば・・・/20260718" --burn-subtitle -o "D:/UDS/YouTube/三人寄れば・・・/20260718/output_rust.mp4"
```
確認:
```bash
ffprobe -v quiet -show_entries format=duration -of csv=p=0 ".../output_rust.mp4"
ffprobe -v error -show_entries stream=codec_type,codec_name,width,height -of default=noprint_wrappers=0 ".../output_rust.mp4"
```
Expected: duration が `full_dialogue.wav` の実尺とほぼ一致。video: h264 1920x1080 / audio: aac。既存 Python 版出力(あれば)と同等。
※ 実データが無い場合は、`full_dialogue.wav` + `timeline/subtitles.srt` + `assets/*.png` + `scene_map.json` を持つ最小プロジェクトを一時作成して実行する。

- [ ] **Step 3: 等価性 OK を確認後、Python 一式を削除**

```bash
git rm -r scripts/video_compose
```

- [ ] **Step 4: 本リポジトリ .claude/settings.local.json から python 許可を撤去**

`.claude/settings.local.json` の以下の行(73/75/76/163 付近)を削除する:
- `"Bash(python -m pytest tests/test_compose_video.py -v)"`
- `"Bash(python \"/d/UDS/Script2Voice-Rust版/scripts/video_compose/compose_video.py\" . -o output_no_burn.mp4)"`
- `"Bash(python \"/d/UDS/Script2Voice-Rust版/scripts/video_compose/compose_video.py\" . -o output_burn.mp4 --burn-subtitle)"`
- `"Bash(python -m pytest scripts/video_compose/tests -q)"`

(JSON 配列の要素削除。末尾カンマに注意して有効な JSON を保つこと。)

- [ ] **Step 5: ビルド確認(削除後もワークスペースが健全)**

Run: `cargo test --workspace --all-targets`
Expected: PASS(全件)。

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "chore(video_compose): remove Python impl after Rust port (equivalence verified)"
```

---

### Task 8: 外部スキル・手順書を Rust 版コマンドへ書き換える

**Files(別リポジトリ `D:/UDS/YouTube`):**
- Modify: `三人寄れば・・・/.claude/skills/composing-episode-video/SKILL.md`
- Modify: `三人寄れば・・・/CLAUDE.md`
- Modify: `三人寄れば・・・/.claude/settings.local.json`

**Interfaces:** なし(ドキュメント/設定の整合)。

> **注記:** これは別 git リポジトリ(`D:/UDS/YouTube`)への変更。コミットは当該リポジトリで行う。

- [ ] **Step 1: SKILL.md の合成コマンドを差し替える**

`三人寄れば・・・/.claude/skills/composing-episode-video/SKILL.md`:
- 30行目付近の
  ```
  py -3.11 "D:/UDS/Script2Voice-Rust版/scripts/video_compose/compose_video.py" "<日付>" --burn-subtitle
  ```
  を
  ```
  "C:\Program Files\Script2Voice\script2voice.exe" compose "<日付>" --burn-subtitle
  ```
  に置き換える。
- 概要文(10行目付近)の「`compose_video.py`で1本のmp4に合成する」を「`script2voice.exe compose`で1本のmp4に合成する」に更新する。

- [ ] **Step 2: CLAUDE.md の言及を更新する**

`三人寄れば・・・/CLAUDE.md` 118行目付近「`composing-episode-video`スキルに従って`compose_video.py`で最終動画（`output.mp4`）を合成する」の `compose_video.py` を `script2voice.exe compose` に更新する。

- [ ] **Step 3: settings.local.json の python 許可を exe 許可へ差し替える**

`三人寄れば・・・/.claude/settings.local.json` の 32/52/94行付近にある
`compose_video.py` を呼ぶ python/`py -3.11` 許可行を削除し、代わりに次を追加する:
```json
"Bash(\"C:\\Program Files\\Script2Voice\\script2voice.exe\" compose *)"
```
(有効な JSON を保つこと。)

- [ ] **Step 4: 動作確認(スキル手順の一通し)**

`script2voice.exe` を `C:\Program Files\Script2Voice\` へ配置済みの状態で、SKILL.md の手順どおり
`"C:\Program Files\Script2Voice\script2voice.exe" compose "<日付>" --burn-subtitle` を実行し、
`<日付>/output.mp4` が生成されることを確認する。

- [ ] **Step 5: Commit(YouTube リポジトリ)**

```bash
cd "D:/UDS/YouTube"
git add "三人寄れば・・・/.claude/skills/composing-episode-video/SKILL.md" "三人寄れば・・・/CLAUDE.md" "三人寄れば・・・/.claude/settings.local.json"
git commit -m "chore(skill): point video composition to script2voice compose (Rust)"
```

---

## 導入(手動ステップ・計画外の最終作業)

ビルドした `target/release/script2voice.exe` を `C:\Program Files\Script2Voice\script2voice.exe` へ上書きコピーする(スキルが参照する固定パス)。これはユーザーが実施する。

---

## 完了条件(Definition of Done)

- `cargo test --workspace --all-targets` グリーン(移植テスト全件含む)。
- `cargo clippy --workspace --all-targets` 新規警告なし。
- `script2voice compose <dir> --burn-subtitle` が実データで `output.mp4` を生成し、Python 版と同等。
- `script2voice <台本>` / `--strict` が従来どおり動作(後方互換)。
- `scripts/video_compose/` 削除済み、両リポジトリの設定・スキルが Rust 版コマンドを参照。
