# Claude Code 引き継ぎ指示書
# Script2Voice → 動画自動合成パイプライン

---

## プロジェクト概要

Script2Voice（音声合成ツール）の出力（音声ファイル＋.srt字幕）と、
台本から生成したシーン画像を組み合わせて、YouTube向けMP4動画を自動生成するパイプラインを構築する。

---

## 現状

- **Script2Voice** はPython版・Rust版の2実装がある。今後は**Rust版を中心にメンテ**する。
- 台本フォーマット：独自形式（`@scene`、`@cast`、`@script`ブロック、`#paragraph`タグ等）
- 現在の出力：音声ファイル（WAV/MP3）＋ .srt字幕ファイル

---

## 全体の2工程構成

```
【工程1：コンテンツ準備】（人間＋Claude支援）

  台本.txt
      ↓
  Claude → 段落単位でシーン分割＆画像プロンプト生成（英語）
      ↓
  ChatGPT等で画像生成（手動）
      ↓
  scene_map.json を手動作成（段落番号→画像ファイルのマッピング）

【工程2：動画合成】（スクリプト自動処理）

  output.srt（[PARAGRAPH]入り）＋ scene_map.json ＋ 音声ファイル
      ↓
  Pythonスクリプト（FFmpegラッパー）
      ↓
  output.mp4
```

---

## 今回実装する機能

### 機能1：srtへのparagraphタイミング埋め込み（Rust版Script2Voiceの改修）

台本の`#paragraph`タグが出現するタイミングを、srtファイルに以下の形式で挿入する。

**挿入形式**：
```
N
HH:MM:SS,mmm --> HH:MM:SS,mmm
[PARAGRAPH]
```

- タイムスタンプは`#paragraph`直前のセリフの終了時刻と同じ値を使う（ゼロ秒エントリ）
- 連番Nは通常の字幕と連続した番号にする
- テキストは`[PARAGRAPH]`固定とする

**実装箇所**：Rust版Script2VoiceのSRT出力モジュール

---

### 機能2：動画自動合成スクリプト（新規作成・Python）

#### 入力ファイル

| ファイル | 内容 |
|----------|------|
| `output.wav` / `output.mp3` | Script2Voice生成の音声ファイル |
| `output.srt` | `[PARAGRAPH]`エントリ入りの字幕ファイル |
| `scene_map.json` | 段落番号→画像ファイルパスのマッピング |

#### scene_map.jsonの形式

```json
{
  "paragraphs": [
    { "index": 1, "image": "images/scene01.png" },
    { "index": 2, "image": "images/scene02.png" },
    { "index": 3, "image": "images/scene03.png" }
  ],
  "default_image": "images/default.png"
}
```

- `index`はsrtの`[PARAGRAPH]`エントリの出現順（1始まり）
- `default_image`は対応画像がない段落に使うフォールバック画像

#### 処理フロー

1. srtを読み込み、`[PARAGRAPH]`エントリのタイムスタンプを抽出してリスト化
2. scene_map.jsonを読み込み、段落番号→画像パスのマッピングを生成
3. 各段落の表示時間を計算（段落Nの開始〜段落N+1の開始、最終段落は音声終端まで）
4. FFmpegのconcat filterを使って音声＋画像スライドショー→MP4を生成
5. 字幕（srt）を焼き込むオプションも用意する（`--burn-subtitle`フラグ）

#### 出力

- `output.mp4`（16:9、1920×1080推奨）
- 字幕焼き込みあり/なしを選択可能

#### FFmpegコマンドのイメージ

```bash
ffmpeg \
  -i output.wav \
  -loop 1 -i scene01.png -loop 1 -i scene02.png ... \
  -filter_complex "
    [1:v]scale=1920:1080,setpts=PTS-STARTPTS[v1];
    [2:v]scale=1920:1080,setpts=PTS-STARTPTS[v2];
    ...
    [v1][v2]...concat=n=N:v=1:a=0[vout]
  " \
  -map "[vout]" -map 0:a \
  -c:v libx264 -c:a aac -shortest \
  output.mp4
```

---

## 実装優先順位

1. **Rust版Script2VoiceのSRT出力に`[PARAGRAPH]`エントリを追加**（最優先）
2. **Pythonで動画合成スクリプトを作成**（FFmpegラッパー）

---

## 環境

- OS：Windows 11
- FFmpeg：インストール済み（パスが通っていること前提）
- Python：3.x系
- Rust：toolchain インストール済み

---

## 補足・制約

- 画像はChatGPT等で生成したPNG/JPEG（16:9）を想定
- 音声とのズレが出ないよう、タイムスタンプの精度はミリ秒単位で維持すること
- 段落数＞画像数の場合は`default_image`で補完する（エラー終了しない）
- 最終的にFilmoraで細かい編集をする可能性があるため、合成MP4は高品質設定で出力すること（`-crf 18`程度）