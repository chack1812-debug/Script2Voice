# video_compose の Rust 移植 — 設計仕様

**日付:** 2026-07-18
**対象:** `scripts/video_compose/`(Python)を Rust クレート `s2v-video` へ移植し、`script2voice compose` サブコマンドとして統合する

---

## 背景と目的

Script2Voice の動画制作フローは「台本作成 → Script2Voice(音声・字幕生成) → Claude で挿絵と挿入位置(`scene_map.json`)作成 → video_compose で動画合成」の4工程からなる。最終工程 `video_compose` は現在 Python 製 CLI(`scripts/video_compose/`)で、ffmpeg/ffprobe を呼ぶ薄い糊コード(本体 ~340行)である。

このツールを**配布**したい。現状はエンドユーザーに「ffmpeg + Script2Voice(Rust) + **Python + スクリプト一式**」を要求してしまう。video_compose を Rust 化して既存の `script2voice` バイナリに統合すれば、ユーザー必要物は「ffmpeg + `script2voice.exe` 1本」に収束する。

パフォーマンス上の利点はない(律速は ffmpeg のエンコード)。移植の唯一かつ十分な動機は**ツールチェーンの単一化と配布の単純化**である。

## ゴール / 非ゴール

**ゴール:**
- `scripts/video_compose/` の全機能を Rust クレート `s2v-video` へ移植する
- `script2voice compose <project_dir> [オプション]` サブコマンドとして呼べるようにする
- 既存の音声生成起動 `script2voice 台本.txt [--config] [--strict]` を**完全に後方互換**で維持する
- Python 版の検証セマンティクス・日本語エラーメッセージ・review.txt 由来の3修正を維持する
- 移植後に Python 一式を削除し、これを参照する外部スキル/手順書を書き換える

**非ゴール:**
- ffmpeg/ffprobe のバンドル(引き続き PATH 上の外部依存とする)
- 出力 MP4 の品質・パラメータ変更(現行の libx264/crf18/yuv420p/faststart/aac を踏襲)
- video_compose の機能追加(純粋な移植に限定する)

---

## アーキテクチャ

### クレート構成

新クレート `crates/s2v-video`(ライブラリ)を新設し、Python の4モジュールを1:1で対応させる。既存ワークスペースの「ロジックはクレート、`src/main.rs` は薄い」方針に従う。

```
crates/s2v-video/
  Cargo.toml
  src/
    lib.rs          # モジュール宣言・再エクスポート・ComposeOptions
    srt_timing.rs   # ← srt_timing.py
    scene_map.rs    # ← scene_map.py
    ffmpeg_cmd.rs   # ← ffmpeg_cmd.py
    compose.rs      # ← compose_video.py(オーケストレーション)
```

`Cargo.toml` の依存はすべてワークスペースに既存:`serde` / `serde_json`(scene_map.json)、`regex`(SRT 解析)、`anyhow`(エラー)。

### モジュール別インターフェース

#### `srt_timing.rs`(← `srt_timing.py`)
- `parse_paragraph_markers(srt_text: &str) -> Vec<f64>`
  SRT テキストから `[PARAGRAPH]` エントリの開始時刻(秒)を出現順に返す。Python の正規表現
  `\d+\r?\n(\d{2}):(\d{2}):(\d{2}),(\d{3}) --> \d{2}:\d{2}:\d{2},\d{3}\r?\n\[PARAGRAPH\]`
  を `regex` クレートで移植する。
- `compute_segments(marker_times: &[f64], total_duration_s: f64) -> anyhow::Result<Vec<(f64, f64)>>`
  各表示セグメントの (開始秒, 終了秒) を計算する。マーカーは「0以上・単調増加・総時間以下」を満たさなければ `Err`(review.txt 指摘の負区間対策)。セグメント数 = マーカー数 + 1。

#### `scene_map.rs`(← `scene_map.py`)
- 型:`AssetKind`(`Image` | `Video`)、`Asset { kind: AssetKind, path: String, source_duration: Option<f64> }`
- `load_scene_map(path: &Path) -> anyhow::Result<SceneMap>`:serde で `scene_map.json` を読む。
  - 新形式(`type` + `path`)と旧形式(`image` キーのみ、常に image 扱い)の両方を受理する。
- `resolve_assets(scene_map: &SceneMap, segment_count: usize) -> anyhow::Result<Vec<Asset>>`
  - セグメント 1..=segment_count に対応するアセットを返す。対応エントリが無ければ `default_image` にフォールバック。
  - `Err` にする条件:index の重複 / segment_count 範囲外 / type が image/video 以外 / 解決先が無い(エントリも default_image も無い)。
- `resolve_asset_paths(assets: Vec<Asset>, base_dir: &Path) -> Vec<Asset>`
  相対パスを `scene_map.json` の置かれたディレクトリ基準の絶対パスへ揃える(review.txt 指摘の CWD 依存対策)。絶対パスはそのまま。
- `validate_assets_exist(assets: &[Asset]) -> anyhow::Result<()>`
  参照先が実在しなければ、欠落パスを列挙して `Err`。

#### `ffmpeg_cmd.rs`(← `ffmpeg_cmd.py`)
- `build_command(opts) -> anyhow::Result<Vec<String>>`(実行しない純粋関数)
  - 入力:`audio_path`、`assets: &[Asset]`、`durations: &[f64]`、`output_path`、`burn_subtitle_path: Option<&Path>`、`width=1920`、`height=1080`、`crf=18`。
  - assets と durations の長さ不一致・空・duration<=0 は `Err`。
  - 画像は `-loop 1 -t <dur>`、動画クリップは `-t <dur>`。動画で `source_duration < dur` のとき `tpad=stop_mode=clone:stop_duration=<deficit>` で最終フレーム静止、`>=` ならトリミング。
  - `scale=...:force_original_aspect_ratio=decrease,pad=...,setsar=1,setpts=PTS-STARTPTS` → `concat=n=N:v=1:a=0[vout]`。
  - `burn_subtitle_path` 指定時は `subtitles=<escaped>` を付与し map を差し替え。Windows のドライブレターのコロンをエスケープする `_escape_subtitles_path` 相当を移植。
  - 出力オプション:`-c:v libx264 -crf 18 -pix_fmt yuv420p -c:a aac -shortest -movflags +faststart`。

#### `compose.rs`(← `compose_video.py` の `main`)
- `run(opts: ComposeOptions) -> anyhow::Result<()>`
  1. `find_audio_file`:`full_dialogue.wav` を優先、無ければ `.mp3`。両方無ければ `Err`。
  2. `<project_dir>/timeline/subtitles.srt` を読む → `parse_paragraph_markers`。
  3. `probe_duration_seconds`:ffprobe を `std::process::Command` で呼び総再生時間を得る。
  4. `compute_segments` → `load_scene_map` → `resolve_assets` → `resolve_asset_paths` → `validate_assets_exist`。
  5. 動画アセットは各 `source_duration` を ffprobe で取得。
  6. `build_command` → ffmpeg を実行。完了メッセージを表示。
- `ComposeOptions { project_dir, scene_map: Option<PathBuf>, burn_subtitle: bool, output: Option<PathBuf> }`。
  既定:scene_map は `<project_dir>/scene_map.json`、output は `<project_dir>/output.mp4`。

### CLI 統合(`src/main.rs`)

clap の「デフォルトコマンド + 任意サブコマンド」定石を使い、後方互換を保つ。

```rust
#[derive(Parser)]
#[command(name = "script2voice", version,
          args_conflicts_with_subcommands = true,
          subcommand_negates_reqs = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    generate: GenerateArgs,   // 既存の scripts / config / strict をこちらへ移す
}

#[derive(Subcommand)]
enum Command {
    /// 音声・字幕とシーン画像から動画を合成する
    Compose(ComposeArgs),
}
```

- `command` が `None` のとき:従来どおり `generate`(音声生成バッチ)を実行。
- `command` が `Some(Compose(..))` のとき:`s2v_video::compose::run(..)` を実行。
- `subcommand_negates_reqs = true` により、サブコマンド指定時は `scripts` の必須制約が外れる。
- `compose` は config.toml 不要。生成用の `--config`/`--strict` は compose に波及しない。

**既知の制約(許容):** `compose` という名前の台本ファイルはサブコマンドに隠れる。実運用で想定しない稀ケースのため許容する。

---

## テスト戦略(TDD)

既存 pytest(`scripts/video_compose/tests/`、~365行)を Rust テストへ 1:1 移植する。

- **純粋ロジック(`srt_timing` / `scene_map` / `ffmpeg_cmd`)**:各モジュールの `#[cfg(test)]` に移植。ffmpeg 不要で全網羅する。
  - `test_ffmpeg_cmd.py` の各ケース(音声→ループ画像、scale/pad/concat、字幕焼き込み、libx264/crf18/shortest、yuv420p/faststart、長さ不一致・duration<=0 エラー、動画クリップのループ無し・トリミング・tpad 静止、画像動画混在)を移植。
  - `test_scene_map.py` / `test_srt_timing.py` の検証系ケースも移植。
- **オーケストレーション(`compose.rs` の ffprobe/ffmpeg 実行部)**:単体テスト対象外。
  - `find_audio_file` のファイル探索ロジックは単体テスト可能なので移植する。
  - ffprobe/ffmpeg を実際に呼ぶ部分は `/verify` で実データ(既存の `D:/UDS/YouTube/三人寄れば・・・/20260718` 等)に対し、Rust 版と Python 版の出力 MP4 を突き合わせて等価性を確認する。

移植中は既存 Python を残し、Rust 版の等価性が取れてから削除する(下記「後始末」)。

---

## 移植後の後始末とスキル書き換え(本スコープに含む)

Rust 版の等価性確認後に実施する。

| 対象 | 変更 |
|---|---|
| `scripts/video_compose/`(本リポジトリ) | ディレクトリごと削除 |
| 本リポジトリ `.claude/settings.local.json` | `compose_video.py` を呼ぶ python 許可行を撤去 |
| `D:/UDS/YouTube/三人寄れば・・・/.claude/skills/composing-episode-video/SKILL.md`(30行目付近) | `py -3.11 "...compose_video.py" "<日付>" --burn-subtitle` → `"C:\Program Files\Script2Voice\script2voice.exe" compose "<日付>" --burn-subtitle`。概要文の `compose_video.py` 言及も更新。 |
| `D:/UDS/YouTube/三人寄れば・・・/CLAUDE.md`(118行目付近) | `compose_video.py` の言及を compose サブコマンドに更新 |
| `D:/UDS/YouTube/三人寄れば・・・/.claude/settings.local.json` | python compose 許可(32/52/94行付近)を exe compose 許可へ差し替え |

`claude_code_instruction.md`(本リポジトリの歴史的な引き継ぎ文書)と `docs/superpowers/plans/2026-06-07-video-compose-script.md`(旧計画)は歴史的記録として原則据え置く。ただし現行手順を誤認させる恐れがある場合は補足を追記してよい。

---

## 導入(手動ステップ)

ビルド後、生成された `script2voice.exe` を `C:\Program Files\Script2Voice\` へ上書きコピーする(スキルが参照する固定パス)。この配置はユーザーが行う手順として提示し、エージェントは代行しない。

---

## 検証・完了条件

- `cargo test --workspace --all-targets` がグリーン(移植した `s2v-video` のテストを含む)。
- `cargo clippy --workspace` に新規警告なし。
- 実データで `script2voice compose <dir> --burn-subtitle` を実行し、`output.mp4` が生成され、Python 版と同等(duration が音声実尺とほぼ一致、video: h264 1920x1080 / audio: aac)であることを `/verify` で確認。
- 後方互換:`script2voice 台本.txt` および `script2voice 台本.txt --strict` が従来どおり動作する。
- 外部スキル/手順書が Rust 版コマンドを指すよう書き換わっている。
