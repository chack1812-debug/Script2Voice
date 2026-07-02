# 動画製作エンジン: 4段階パイプライン設計

## 背景・目的

Script2Voice は現在「台本→音声＋`[PARAGRAPH]`入りSRT」までを担い、`scripts/video_compose/`（Python/FFmpegラッパー）が「音声＋SRT＋手動`scene_map.json`（段落→静止画）」から静止画スライドショーMP4を合成している（`claude_code_instruction.md`の機能1・2として実装済み、bd: s2v-avw, s2v-cus）。

動画製作をより本格化するにあたり、以下を検討・合意した。

- AI画像/動画生成は品質・キャラクター一貫性の安定化が難しい。当面は**生成そのものをScript2Voiceの外（人間＋Claudeでプロンプト作成、Higgsfield等の外部ツールで生成）に切り離す**。
- 台本フォーマット（`@cast`/`@scene`）にはキャラの見た目やシーンの情景を表す自由記述フィールドが一切存在しない（音声合成・空間音響パラメータのみ）。よって「キャラ一貫性のための固定プロンプト」は台本フォーマットの拡張ではなく、**別建てのサイドカーファイル**として持つ。

## 全体アーキテクチャ（4段階パイプライン）

```
Stage1: Script2Voice(Rust, 既存 + @cast/@scene自由記述拡張)
  台本.txt (@castに外見描写, @sceneに情景描写を自由記述で埋め込み)
    → full_dialogue.wav/.mp3
    → timeline/subtitles.srt ([PARAGRAPH]マーカー入り)

Stage2: プロンプト生成 (Claude, スキーマ設計のみ・実装は後続)
  台本.txt (@cast/@sceneの自由記述込み、これ一本で完結)
    → prompts.json (paragraph毎の生成プロンプト)

Stage3: 画像/動画生成 (人間 + Higgsfield等の外部ツール, 完全手動)
  prompts.json
    → assets/p01.png, assets/p02.mp4, ... (実ファイル)
    → scene_map.json (段落→アセットのマッピング、人間が作成)

Stage4: 統合 (scripts/video_compose 拡張, 今回実装)
  full_dialogue.wav + timeline/subtitles.srt + scene_map.json
    → output.mp4 (画像/動画クリップ混在対応)
```

> **2026-07-02追記**: Stage2入力だったcharacters_visual.json/scenes_visual.jsonサイドカー方式は廃止し、`@cast`/`@scene`の自由記述に一本化した。詳細は `2026-07-02-script-visual-description-extension-design.md` を参照。以下のディレクトリ構成・ファイル形式・責務表・後続タスクの記述はサイドカー方式時点のものであり、上記追記により一部が置き換わっている。

### ディレクトリ構成 (project_dir 配下、既存video_composeの規約を踏襲)

```
<project_dir>/
  full_dialogue.wav / .mp3      # Stage1 出力 (既存)
  timeline/subtitles.srt        # Stage1 出力 (既存)
  characters_visual.json        # Stage2入力 (新規)
  scenes_visual.json            # Stage2入力 (新規)
  prompts.json                  # Stage2出力/Stage3入力 (新規)
  assets/                       # Stage3出力: 生成済み画像/動画クリップ (新規)
  scene_map.json                # Stage3出力/Stage4入力 (既存、スキーマ拡張)
  output.mp4                    # Stage4出力 (既存)
```

### 各段階の責務

| Stage | 主体 | 入力 | 出力 | 状態 |
|---|---|---|---|---|
| 1 | Script2Voice(Rust) | 台本.txt | 音声+SRT | 既存・変更なし |
| 2 | Claude | 台本.txt, characters_visual.json, scenes_visual.json | prompts.json | スキーマのみ設計済み、実装は後続 |
| 3 | 人間+Higgsfield等 | prompts.json | assets/, scene_map.json | 完全手動、対象外 |
| 4 | video_compose(Python) | 音声, SRT, scene_map.json | output.mp4 | **実装済み**(本設計書と同時) |

## ファイル形式

### characters_visual.json (Stage2入力・新規)

キーは台本`@cast`の役名と一致させる。

```json
{
  "characters": {
    "司会": { "appearance": "young female announcer, short black bob hair, red blazer", "style_keywords": ["anime style", "flat lighting"] }
  }
}
```

### scenes_visual.json (Stage2入力・新規)

キーは台本`@scene`名と一致させる。

```json
{
  "scenes": {
    "01_オープニング": { "location": "modern tech conference stage, blue LED backdrop", "mood": "energetic, bright" }
  }
}
```

### prompts.json (Stage2出力・新規)

Claudeが台本＋上記2ファイルの固定アンカーを合成して生成する。

```json
{
  "paragraphs": [
    { "index": 1, "scene": "01_オープニング", "cast": ["司会", "解説"], "asset_type": "image", "prompt": "固定アンカー+段落固有描写を合成した最終プロンプト", "source_text": "参考用の元セリフ抜粋" }
  ]
}
```

### scene_map.json (Stage3出力/Stage4入力・既存スキーマ拡張、後方互換あり)

```json
{
  "paragraphs": [
    { "index": 1, "type": "image", "path": "assets/p01.png" },
    { "index": 2, "type": "video", "path": "assets/p02.mp4" },
    { "index": 3, "image": "assets/p03.png" }
  ],
  "default_image": "assets/default.png"
}
```

- `type`+`path`が新形式。`type`省略時は`"image"`扱い。
- 旧形式(`"image"`キーのみ)も引き続きサポートし、`type: "image"`として正規化する。
- `default_image`によるフォールバックは静止画のみ（用途を複雑化しない）。

## Stage4実装: `scripts/video_compose/` 動画クリップ対応

### 尺不一致の処理方針

- クリップが表示時間より**長い場合**: 先頭から`durations[i]`秒だけ使用（`-t`オプションでトリミング、既存の画像と同じ扱い）。
- クリップが表示時間より**短い場合**: 不足分を**最終フレームで静止**して埋める（ffmpegの`tpad=stop_mode=clone:stop_duration=<不足秒>`フィルタ）。ループ再生は不採用（ループ点で不自然な跳躍が出るため）。

### 変更箇所

- `scripts/video_compose/scene_map.py`: `resolve_images` → `resolve_assets(scene_map, segment_count) -> list[dict]`（`{"type": "image"|"video", "path": str}`）に変更。新旧スキーマを`_normalize_entry`で正規化。
- `scripts/video_compose/compose_video.py`: `resolve_assets`呼び出しに変更。`type == "video"`のアセットに対し既存の汎用`probe_duration_seconds()`でクリップ自身の長さを取得し`source_duration`として付与。
- `scripts/video_compose/ffmpeg_cmd.py`: `build_command`の引数を`images: list[str]`から`assets: list[dict]`に変更。
  - 画像: 既存通り`-loop 1 -t <duration> -i <path>`。
  - 動画: `-loop`なしで`-t <duration> -i <path>`（`-t`は入力オプションとして機能し、長い場合は自動トリミング）。
  - `source_duration < duration`の場合のみ、スケール/パッド後に`tpad=stop_mode=clone:stop_duration=<不足秒>`を追加適用して最終フレームを複製。
  - concat以降（`concat=n=N:v=1:a=0[vout]`、字幕焼き込み、コーデック指定）は変更なし。クリップ内蔵音声は元々`a=0`でマップされないため、ナレーション音声との衝突は発生しない。

### 検証結果

- `python -m pytest`（`scripts/video_compose`）: 22件全PASS（既存17件+新規5件: 動画入力の`-loop`省略、トリミング時に`tpad`が付与されないこと、尺不足時に`tpad=stop_mode=clone`が付与されること、画像/動画混在時のconcat、`resolve_assets`の新旧スキーマ・video対応・フォールバック）。
- 手動E2E検証: 無音WAV(6秒)+`[PARAGRAPH]`マーカー1個(1.0秒地点)+静止画(1枚)+動画クリップ(3秒尺、表示枠5秒=不足分2秒)を用意し`compose_video.py`を実行。エラーなく完走し、`output.mp4`(1920x1080, H.264/AAC, 全長6.0秒=音声長と一致)を確認。

## 後続タスク（本設計書の範囲外、beadsに登録）

- Stage2: Claudeによる`prompts.json`生成手順/スキルの実装
- Stage3: `characters_visual.json` / `scenes_visual.json`の実データ作成（人間作業）
