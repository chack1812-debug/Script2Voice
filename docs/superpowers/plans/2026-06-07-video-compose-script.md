# Python版 動画自動合成スクリプト Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rust版Script2Voiceの出力（`full_dialogue.wav` ＋ `[PARAGRAPH]`入り `subtitles.srt`）とシーン画像のマッピング（`scene_map.json`）から、FFmpegでスライドショー形式のMP4動画を自動生成するPython製CLIツールを作る（`claude_code_instruction.md` 実装優先順位2番）。

**Architecture:** 標準ライブラリのみで実装する4つの純粋関数モジュール（SRT解析、scene_map解決、FFmpegコマンド構築）と、それらを束ねるCLIオーケストレーション（`compose_video.py`）に分割する。FFmpegの実行とffprobeによる音声長の取得はサブプロセス呼び出しのみで行い、外部Pythonライブラリは使わない。各モジュールはFFmpeg/ffprobeを実行せずにテストできる純粋関数として設計する。

**Tech Stack:** Python 3.11 (標準ライブラリのみ: argparse, json, re, subprocess, pathlib), pytest, FFmpeg/ffprobe (コマンドライン)

---

## File Structure

- Create: `scripts/video_compose/srt_timing.py` — SRTから`[PARAGRAPH]`マーカーの時刻を抽出し、表示セグメント（開始秒・終了秒）を計算する純粋関数
- Create: `scripts/video_compose/scene_map.py` — `scene_map.json`の読み込みと、セグメント番号→画像パスの解決
- Create: `scripts/video_compose/ffmpeg_cmd.py` — FFmpegコマンド（引数リスト）を構築する純粋関数。実行はしない
- Create: `scripts/video_compose/compose_video.py` — CLIエントリポイント。引数解析、入力ファイル検出、上記モジュールの呼び出し、FFmpeg実行
- Create: `scripts/video_compose/requirements.txt` — 依存関係明示用（現状は標準ライブラリのみのため空＋コメント）
- Create: `scripts/video_compose/tests/conftest.py` — テストから`scripts/video_compose/`直下のモジュールをインポート可能にする
- Create: `scripts/video_compose/tests/test_srt_timing.py`
- Create: `scripts/video_compose/tests/test_scene_map.py`
- Create: `scripts/video_compose/tests/test_ffmpeg_cmd.py`
- Create: `scripts/video_compose/tests/test_compose_video.py`

設計確認済みの仕様（ユーザー承認済み）:
- `scene_map.json`の`index`は「表示セグメント番号」（1始まり）と対応する。`#paragraph`が台本中にN個あれば、SRTには`[PARAGRAPH]`マーカーがN個出力され、表示セグメントはN+1個になる（音声開始→マーカー1、マーカー1→マーカー2、…、マーカーN→音声終端）
- 音声ファイルは`full_dialogue.wav`を優先、なければ`full_dialogue.mp3`にフォールバック
- CLIはRust版の出力ディレクトリを一括指定する形（`python compose_video.py <project_dir> [--scene-map PATH] [--burn-subtitle] [-o OUTPUT]`）

---

## Task 1: SRTタイミング解析モジュール (`srt_timing.py`)

**Files:**
- Create: `scripts/video_compose/tests/conftest.py`
- Create: `scripts/video_compose/srt_timing.py`
- Create: `scripts/video_compose/tests/test_srt_timing.py`

- [ ] **Step 1: テスト用の `conftest.py` を作成する**

`scripts/video_compose/tests/conftest.py` を新規作成:

```python
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
```

これにより、`tests/`配下のテストから`scripts/video_compose/`直下のモジュール（`srt_timing`, `scene_map`, `ffmpeg_cmd`, `compose_video`）をパッケージ化せずにフラットインポートできる。

- [ ] **Step 2: 失敗するテストを書く**

`scripts/video_compose/tests/test_srt_timing.py` を新規作成:

```python
from srt_timing import compute_segments, parse_paragraph_markers


def test_parse_paragraph_markers_extracts_start_times_in_order():
    srt_text = (
        "1\n00:00:00,000 --> 00:00:01,500\nこんにちは\n\n"
        "2\n00:00:01,500 --> 00:00:01,500\n[PARAGRAPH]\n\n"
        "3\n00:00:03,000 --> 00:00:03,800\nさようなら\n\n"
        "4\n00:01:05,250 --> 00:01:05,250\n[PARAGRAPH]\n\n"
    )
    assert parse_paragraph_markers(srt_text) == [1.5, 65.25]


def test_parse_paragraph_markers_returns_empty_list_when_no_markers():
    srt_text = "1\n00:00:00,000 --> 00:00:01,500\nこんにちは\n\n"
    assert parse_paragraph_markers(srt_text) == []


def test_compute_segments_splits_audio_into_marker_count_plus_one_ranges():
    segments = compute_segments([1.5, 65.25], total_duration_s=120.0)
    assert segments == [(0.0, 1.5), (1.5, 65.25), (65.25, 120.0)]


def test_compute_segments_returns_single_segment_when_no_markers():
    segments = compute_segments([], total_duration_s=10.0)
    assert segments == [(0.0, 10.0)]
```

- [ ] **Step 3: テストを実行して失敗を確認する**

Run: `cd "scripts/video_compose" && python -m pytest tests/test_srt_timing.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'srt_timing'`（まだファイルが存在しないため）

- [ ] **Step 4: 最小実装を書く**

`scripts/video_compose/srt_timing.py` を新規作成:

```python
"""SRT字幕ファイルから [PARAGRAPH] マーカーの時刻を抽出し、
スライドショーの各表示セグメント (開始秒, 終了秒) を計算する。"""
import re

_PARAGRAPH_BLOCK_RE = re.compile(
    r"\d+\r?\n"
    r"(\d{2}):(\d{2}):(\d{2}),(\d{3})"
    r" --> "
    r"\d{2}:\d{2}:\d{2},\d{3}\r?\n"
    r"\[PARAGRAPH\]"
)


def parse_paragraph_markers(srt_text: str) -> list[float]:
    """SRTテキストから [PARAGRAPH] エントリの開始時刻 (秒) を出現順に返す。"""
    markers = []
    for match in _PARAGRAPH_BLOCK_RE.finditer(srt_text):
        hours, minutes, seconds, millis = (int(g) for g in match.groups())
        markers.append(hours * 3600 + minutes * 60 + seconds + millis / 1000.0)
    return markers


def compute_segments(marker_times: list[float], total_duration_s: float) -> list[tuple[float, float]]:
    """マーカー時刻のリストと音声総時間から、各表示セグメントの (開始秒, 終了秒) を計算する。

    セグメント数 = マーカー数 + 1。
    セグメント1      : 音声開始 (0.0)        〜 1個目のマーカー
    セグメントk (中間): (k-1)個目のマーカー  〜 k個目のマーカー
    最終セグメント   : 最後のマーカー        〜 音声終端 (total_duration_s)
    """
    boundaries = [0.0, *marker_times, total_duration_s]
    return [(boundaries[i], boundaries[i + 1]) for i in range(len(boundaries) - 1)]
```

- [ ] **Step 5: テストを実行して成功を確認する**

Run: `cd "scripts/video_compose" && python -m pytest tests/test_srt_timing.py -v`
Expected: PASS（4 passed）

- [ ] **Step 6: コミット**

```bash
git add scripts/video_compose/tests/conftest.py scripts/video_compose/srt_timing.py scripts/video_compose/tests/test_srt_timing.py
git commit -m "feat(video-compose): add SRT [PARAGRAPH] marker parsing and segment computation"
```

---

## Task 2: scene_map解決モジュール (`scene_map.py`)

**Files:**
- Create: `scripts/video_compose/scene_map.py`
- Create: `scripts/video_compose/tests/test_scene_map.py`

- [ ] **Step 1: 失敗するテストを書く**

`scripts/video_compose/tests/test_scene_map.py` を新規作成:

```python
import json

from scene_map import load_scene_map, resolve_images


def test_load_scene_map_reads_json_file(tmp_path):
    path = tmp_path / "scene_map.json"
    path.write_text(
        json.dumps({
            "paragraphs": [{"index": 1, "image": "images/scene01.png"}],
            "default_image": "images/default.png",
        }),
        encoding="utf-8",
    )
    scene_map = load_scene_map(path)
    assert scene_map["paragraphs"][0]["image"] == "images/scene01.png"
    assert scene_map["default_image"] == "images/default.png"


def test_resolve_images_maps_index_to_image_path():
    scene_map = {
        "paragraphs": [
            {"index": 1, "image": "images/scene01.png"},
            {"index": 2, "image": "images/scene02.png"},
        ],
        "default_image": "images/default.png",
    }
    assert resolve_images(scene_map, segment_count=2) == [
        "images/scene01.png",
        "images/scene02.png",
    ]


def test_resolve_images_falls_back_to_default_image_for_missing_index():
    scene_map = {
        "paragraphs": [{"index": 1, "image": "images/scene01.png"}],
        "default_image": "images/default.png",
    }
    assert resolve_images(scene_map, segment_count=3) == [
        "images/scene01.png",
        "images/default.png",
        "images/default.png",
    ]
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cd "scripts/video_compose" && python -m pytest tests/test_scene_map.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'scene_map'`

- [ ] **Step 3: 最小実装を書く**

`scripts/video_compose/scene_map.py` を新規作成:

```python
"""scene_map.json を読み込み、表示セグメント番号 (1始まり) に対応する画像パスを解決する。"""
import json
from pathlib import Path


def load_scene_map(path: Path) -> dict:
    """scene_map.json を読み込んで辞書として返す。"""
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def resolve_images(scene_map: dict, segment_count: int) -> list[str]:
    """セグメント番号 1..segment_count に対応する画像パスのリストを返す。

    scene_map["paragraphs"] に対応する index のエントリが無い場合は
    scene_map["default_image"] にフォールバックする
    （段落数 > 画像数でもエラー終了しないための仕様）。
    """
    by_index = {entry["index"]: entry["image"] for entry in scene_map.get("paragraphs", [])}
    default_image = scene_map.get("default_image")
    return [by_index.get(index, default_image) for index in range(1, segment_count + 1)]
```

- [ ] **Step 4: テストを実行して成功を確認する**

Run: `cd "scripts/video_compose" && python -m pytest tests/test_scene_map.py -v`
Expected: PASS（3 passed）

- [ ] **Step 5: コミット**

```bash
git add scripts/video_compose/scene_map.py scripts/video_compose/tests/test_scene_map.py
git commit -m "feat(video-compose): add scene_map.json loading and index-to-image resolution"
```

---

## Task 3: FFmpegコマンド構築モジュール (`ffmpeg_cmd.py`)

**Files:**
- Create: `scripts/video_compose/ffmpeg_cmd.py`
- Create: `scripts/video_compose/tests/test_ffmpeg_cmd.py`

- [ ] **Step 1: 失敗するテストを書く**

`scripts/video_compose/tests/test_ffmpeg_cmd.py` を新規作成:

```python
from pathlib import Path

import pytest

from ffmpeg_cmd import build_command


def _input_paths(cmd):
    """コマンド引数列から -i に渡されたパスを順番に取り出す。"""
    return [cmd[i + 1] for i, arg in enumerate(cmd) if arg == "-i"]


def test_build_command_feeds_audio_then_looped_timed_images():
    cmd = build_command(
        audio_path=Path("project/full_dialogue.wav"),
        images=["images/scene01.png", "images/scene02.png"],
        durations=[1.5, 63.75],
        output_path=Path("project/output.mp4"),
    )
    assert cmd[0] == "ffmpeg"
    inputs = _input_paths(cmd)
    assert Path(inputs[0]) == Path("project/full_dialogue.wav")
    assert Path(inputs[1]) == Path("images/scene01.png")
    assert Path(inputs[2]) == Path("images/scene02.png")
    # 1個目の画像入力の直前に -loop 1 -t <duration> が付与される
    loop_idx = cmd.index("-loop")
    assert cmd[loop_idx:loop_idx + 4] == ["-loop", "1", "-t", "1.500"]


def test_build_command_filter_complex_scales_pads_and_concats_all_images():
    cmd = build_command(
        audio_path=Path("a.wav"),
        images=["s1.png", "s2.png", "s3.png"],
        durations=[1.0, 2.0, 3.0],
        output_path=Path("out.mp4"),
    )
    filter_complex = cmd[cmd.index("-filter_complex") + 1]
    assert "[1:v]scale=1920:1080:force_original_aspect_ratio=decrease" in filter_complex
    assert "[v1][v2][v3]concat=n=3:v=1:a=0[vout]" in filter_complex
    assert cmd[cmd.index("-map") + 1] == "[vout]"
    assert "0:a" in cmd


def test_build_command_appends_subtitles_filter_and_remaps_when_burn_subtitle_given():
    cmd = build_command(
        audio_path=Path("a.wav"),
        images=["s1.png"],
        durations=[5.0],
        output_path=Path("out.mp4"),
        burn_subtitle_path=Path("project/timeline/subtitles.srt"),
    )
    filter_complex = cmd[cmd.index("-filter_complex") + 1]
    assert "[vout]subtitles=" in filter_complex
    assert "[vsub]" in filter_complex
    map_indices = [i for i, arg in enumerate(cmd) if arg == "-map"]
    assert cmd[map_indices[0] + 1] == "[vsub]"


def test_build_command_uses_libx264_crf18_and_shortest_for_high_quality_output():
    cmd = build_command(
        audio_path=Path("a.wav"),
        images=["s1.png"],
        durations=[5.0],
        output_path=Path("out.mp4"),
    )
    assert cmd[cmd.index("-c:v") + 1] == "libx264"
    assert cmd[cmd.index("-crf") + 1] == "18"
    assert "-shortest" in cmd
    assert cmd[-1] == "out.mp4"


def test_build_command_raises_when_images_and_durations_length_mismatch():
    with pytest.raises(ValueError):
        build_command(
            audio_path=Path("a.wav"),
            images=["s1.png", "s2.png"],
            durations=[1.0],
            output_path=Path("out.mp4"),
        )
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cd "scripts/video_compose" && python -m pytest tests/test_ffmpeg_cmd.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'ffmpeg_cmd'`

- [ ] **Step 3: 最小実装を書く**

`scripts/video_compose/ffmpeg_cmd.py` を新規作成:

```python
"""FFmpeg のスライドショー合成コマンドを構築する純粋関数群 (実行はしない)。"""
from pathlib import Path


def _scale_pad_filter(label_in: str, label_out: str, width: int, height: int) -> str:
    return (
        f"[{label_in}]scale={width}:{height}:force_original_aspect_ratio=decrease,"
        f"pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,setsar=1,setpts=PTS-STARTPTS[{label_out}]"
    )


def _escape_subtitles_path(path: Path) -> str:
    """ffmpeg subtitles フィルタ用にパスをエスケープする
    (Windowsのドライブレターのコロンが filtergraph の区切り文字と衝突するための対策)。"""
    escaped = str(path).replace("\\", "/").replace(":", r"\:")
    return f"'{escaped}'"


def build_command(
    audio_path: Path,
    images: list[str],
    durations: list[float],
    output_path: Path,
    *,
    burn_subtitle_path: Path | None = None,
    width: int = 1920,
    height: int = 1080,
    crf: int = 18,
) -> list[str]:
    """ffmpeg コマンドを引数リストとして構築する。

    images[i] を durations[i] 秒間表示するスライドショーを音声に重ねた
    MP4 (libx264/aac, crf指定の高品質設定) を生成するコマンドを返す。
    burn_subtitle_path を指定すると字幕を動画に焼き込む。
    """
    if len(images) != len(durations):
        raise ValueError("images と durations は同じ長さでなければならない")
    if not images:
        raise ValueError("images が空です")

    cmd = ["ffmpeg", "-y", "-i", str(audio_path)]
    for image, duration in zip(images, durations):
        cmd += ["-loop", "1", "-t", f"{duration:.3f}", "-i", str(image)]

    filter_parts = []
    video_labels = []
    for i in range(len(images)):
        in_label = f"{i + 1}:v"
        out_label = f"v{i + 1}"
        filter_parts.append(_scale_pad_filter(in_label, out_label, width, height))
        video_labels.append(f"[{out_label}]")

    concat_inputs = "".join(video_labels)
    filter_parts.append(f"{concat_inputs}concat=n={len(images)}:v=1:a=0[vout]")

    final_video_label = "[vout]"
    if burn_subtitle_path is not None:
        filter_parts.append(f"[vout]subtitles={_escape_subtitles_path(burn_subtitle_path)}[vsub]")
        final_video_label = "[vsub]"

    cmd += ["-filter_complex", ";".join(filter_parts)]
    cmd += ["-map", final_video_label, "-map", "0:a"]
    cmd += ["-c:v", "libx264", "-crf", str(crf), "-c:a", "aac", "-shortest"]
    cmd += [str(output_path)]
    return cmd
```

- [ ] **Step 4: テストを実行して成功を確認する**

Run: `cd "scripts/video_compose" && python -m pytest tests/test_ffmpeg_cmd.py -v`
Expected: PASS（5 passed）

- [ ] **Step 5: コミット**

```bash
git add scripts/video_compose/ffmpeg_cmd.py scripts/video_compose/tests/test_ffmpeg_cmd.py
git commit -m "feat(video-compose): add ffmpeg slideshow command builder with subtitle burn-in support"
```

---

## Task 4: CLIオーケストレーション (`compose_video.py`)

**Files:**
- Create: `scripts/video_compose/compose_video.py`
- Create: `scripts/video_compose/tests/test_compose_video.py`

- [ ] **Step 1: 失敗するテストを書く**

`scripts/video_compose/tests/test_compose_video.py` を新規作成:

```python
from pathlib import Path

import pytest

from compose_video import find_audio_file, parse_args


def test_find_audio_file_prefers_wav_over_mp3(tmp_path):
    (tmp_path / "full_dialogue.wav").write_bytes(b"")
    (tmp_path / "full_dialogue.mp3").write_bytes(b"")
    assert find_audio_file(tmp_path) == tmp_path / "full_dialogue.wav"


def test_find_audio_file_falls_back_to_mp3_when_wav_absent(tmp_path):
    (tmp_path / "full_dialogue.mp3").write_bytes(b"")
    assert find_audio_file(tmp_path) == tmp_path / "full_dialogue.mp3"


def test_find_audio_file_raises_when_neither_format_exists(tmp_path):
    with pytest.raises(FileNotFoundError):
        find_audio_file(tmp_path)


def test_parse_args_defaults_scene_map_and_output_to_none_and_burn_subtitle_to_false():
    args = parse_args(["myproject"])
    assert args.project_dir == Path("myproject")
    assert args.scene_map is None
    assert args.output is None
    assert args.burn_subtitle is False


def test_parse_args_accepts_scene_map_burn_subtitle_and_output_overrides():
    args = parse_args([
        "myproject", "--scene-map", "custom_map.json", "--burn-subtitle", "-o", "final.mp4",
    ])
    assert args.scene_map == Path("custom_map.json")
    assert args.burn_subtitle is True
    assert args.output == Path("final.mp4")
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cd "scripts/video_compose" && python -m pytest tests/test_compose_video.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'compose_video'`

- [ ] **Step 3: 最小実装を書く**

`scripts/video_compose/compose_video.py` を新規作成:

```python
#!/usr/bin/env python3
"""Script2Voice の出力 (音声 + [PARAGRAPH]入りSRT) とシーン画像から動画を自動生成するCLI。

使い方:
    python compose_video.py <project_dir> [--scene-map PATH] [--burn-subtitle] [-o OUTPUT]

<project_dir> には Rust版 Script2Voice の出力一式
(full_dialogue.wav または full_dialogue.mp3、timeline/subtitles.srt) が含まれている前提。
scene_map.json は既定で <project_dir>/scene_map.json を読みに行く。
出力は既定で <project_dir>/output.mp4。
"""
import argparse
import subprocess
import sys
from pathlib import Path

from ffmpeg_cmd import build_command
from scene_map import load_scene_map, resolve_images
from srt_timing import compute_segments, parse_paragraph_markers


def find_audio_file(project_dir: Path) -> Path:
    """project_dir 内の音声ファイルを探す。full_dialogue.wav を優先し、無ければ .mp3 にフォールバックする。"""
    for name in ("full_dialogue.wav", "full_dialogue.mp3"):
        candidate = project_dir / name
        if candidate.exists():
            return candidate
    raise FileNotFoundError(
        f"{project_dir} に full_dialogue.wav / full_dialogue.mp3 が見つかりません"
    )


def probe_duration_seconds(media_path: Path) -> float:
    """ffprobe でメディアファイルの総再生時間 (秒) を取得する。"""
    result = subprocess.run(
        [
            "ffprobe", "-v", "quiet",
            "-show_entries", "format=duration",
            "-of", "csv=p=0",
            str(media_path),
        ],
        capture_output=True, text=True, check=True,
    )
    return float(result.stdout.strip())


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Script2Voiceの出力(音声+[PARAGRAPH]入りSRT)とシーン画像から動画を自動合成する"
    )
    parser.add_argument("project_dir", type=Path, help="Rust版Script2Voiceの出力ディレクトリ")
    parser.add_argument(
        "--scene-map", type=Path, default=None,
        help="scene_map.json のパス (省略時は <project_dir>/scene_map.json)",
    )
    parser.add_argument(
        "--burn-subtitle", action="store_true",
        help="字幕を動画に焼き込む",
    )
    parser.add_argument(
        "-o", "--output", type=Path, default=None,
        help="出力先 MP4 のパス (省略時は <project_dir>/output.mp4)",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    project_dir: Path = args.project_dir
    scene_map_path = args.scene_map or (project_dir / "scene_map.json")
    output_path = args.output or (project_dir / "output.mp4")
    srt_path = project_dir / "timeline" / "subtitles.srt"

    audio_path = find_audio_file(project_dir)
    srt_text = srt_path.read_text(encoding="utf-8")
    markers = parse_paragraph_markers(srt_text)
    total_duration = probe_duration_seconds(audio_path)
    segments = compute_segments(markers, total_duration)

    scene_map = load_scene_map(scene_map_path)
    images = resolve_images(scene_map, segment_count=len(segments))
    durations = [end - start for start, end in segments]

    burn_subtitle_path = srt_path if args.burn_subtitle else None
    cmd = build_command(
        audio_path=audio_path,
        images=images,
        durations=durations,
        output_path=output_path,
        burn_subtitle_path=burn_subtitle_path,
    )
    subprocess.run(cmd, check=True)
    print(f"動画を生成しました: {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 4: テストを実行して成功を確認する**

Run: `cd "scripts/video_compose" && python -m pytest tests/test_compose_video.py -v`
Expected: PASS（5 passed）

念のため、これまでの全モジュールのテストもまとめて実行する:
Run: `cd "scripts/video_compose" && python -m pytest tests/ -v`
Expected: 全件PASS（17 passed）

- [ ] **Step 5: コミット**

```bash
git add scripts/video_compose/compose_video.py scripts/video_compose/tests/test_compose_video.py
git commit -m "feat(video-compose): add CLI orchestration that wires SRT/scene_map/ffmpeg modules together"
```

---

## Task 5: requirements.txt とエンドツーエンドの手動検証

**Files:**
- Create: `scripts/video_compose/requirements.txt`

- [ ] **Step 1: requirements.txt を作成する**

`scripts/video_compose/requirements.txt` を新規作成:

```
# scripts/video_compose は現状、標準ライブラリ (argparse, json, re, subprocess, pathlib) のみで動作する。
# 外部Pythonパッケージへの依存はない。FFmpeg/ffprobeはシステムにインストールされ、PATHが通っている前提。
# 将来的に画像処理(リサイズ等)や高度な字幕レンダリングのために依存を追加する場合はここに追記する。
```

- [ ] **Step 2: 全テストスイートを実行して回帰がないことを確認する**

Run: `cd "scripts/video_compose" && python -m pytest tests/ -v`
Expected: 全件PASS

- [ ] **Step 3: 手動でのエンドツーエンド検証手順を実行する**

これは自動テストではなく、実際にFFmpegを呼び出す手動検証である（テスト用のダミー画像・音声・scene_map.jsonをその場で用意する）。

```bash
# 作業用一時ディレクトリを用意
mkdir -p /tmp/s2v_video_smoke/timeline
cd /tmp/s2v_video_smoke

# 1. テスト用の無音音声 (3秒) を生成
ffmpeg -y -f lavfi -i anullsrc=r=24000:cl=mono -t 3 full_dialogue.wav

# 2. [PARAGRAPH] マーカーを1つ含む最小SRTを用意 (1.5秒地点で区切る -> セグメント2つ)
cat > timeline/subtitles.srt << 'EOF'
1
00:00:00,000 --> 00:00:01,500
こんにちは

2
00:00:01,500 --> 00:00:01,500
[PARAGRAPH]

3
00:00:01,500 --> 00:00:03,000
さようなら

EOF

# 3. テスト用のシーン画像を2枚生成 (16:9, 単色)
ffmpeg -y -f lavfi -i color=c=blue:s=640x360 -frames:v 1 scene01.png
ffmpeg -y -f lavfi -i color=c=red:s=640x360 -frames:v 1 scene02.png

# 4. scene_map.json を用意
cat > scene_map.json << 'EOF'
{
  "paragraphs": [
    { "index": 1, "image": "scene01.png" },
    { "index": 2, "image": "scene02.png" }
  ],
  "default_image": "scene01.png"
}
EOF

# 5. 動画を合成 (字幕焼き込みなし)
python "<repo>/scripts/video_compose/compose_video.py" . -o output_no_burn.mp4

# 6. 動画を合成 (字幕焼き込みあり)
python "<repo>/scripts/video_compose/compose_video.py" . -o output_burn.mp4 --burn-subtitle
```

Expected:
- 手順5・6ともに `動画を生成しました: ...` が表示され、`output_no_burn.mp4` / `output_burn.mp4` が生成される
- `ffprobe output_no_burn.mp4` で再生時間が約3秒、解像度1920x1080であることを確認する: `ffprobe -v quiet -show_entries stream=width,height -show_entries format=duration -of default=noprint_wrappers=1 output_no_burn.mp4`
- `output_burn.mp4` を実際に再生するか、`ffmpeg -i output_burn.mp4 -vf "select=eq(n\,0)" -vframes 1 frame.png` でフレームを書き出して、字幕が焼き込まれていることを目視確認する
- 1.5秒付近で背景色が青→赤に切り替わること（セグメント境界が`[PARAGRAPH]`マーカーの時刻と一致していること）を目視確認する

- [ ] **Step 4: 検証結果をユーザーに報告する**

手動検証の結果（生成されたファイル、再生時間、解像度、切り替えタイミング、字幕焼き込みの有無）をまとめて報告する。問題があれば該当タスクに戻って修正する。

---

## Self-Review チェックリスト

- **仕様カバレッジ**:
  - 「srtを読み込み[PARAGRAPH]エントリのタイムスタンプを抽出」→ Task 1 `parse_paragraph_markers`
  - 「scene_map.jsonを読み込みマッピング生成」→ Task 2 `load_scene_map`/`resolve_images`
  - 「各段落の表示時間を計算（段落Nの開始〜段落N+1の開始、最終段落は音声終端まで）」→ Task 1 `compute_segments`（ユーザー承認済みの「セグメント=マーカー数+1」設計）
  - 「FFmpegのconcat filterで音声+画像スライドショー→MP4」→ Task 3 `build_command`
  - 「字幕焼き込みオプション(--burn-subtitle)」→ Task 3 `burn_subtitle_path`引数 + Task 4 `--burn-subtitle`フラグ
  - 「16:9、1920x1080推奨」「crf 18程度の高品質設定」→ Task 3 `width=1920, height=1080, crf=18`
  - 「段落数＞画像数の場合はdefault_imageで補完しエラー終了しない」→ Task 2 `resolve_images`のフォールバック
- **プレースホルダ**: なし（すべて完全なコード）
- **型/命名の一貫性**: `parse_paragraph_markers`/`compute_segments`/`load_scene_map`/`resolve_images`/`build_command`/`find_audio_file`/`probe_duration_seconds`/`parse_args`/`main` の名前と引数はTask間で統一されている
