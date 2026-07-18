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
from scene_map import load_scene_map, resolve_asset_paths, resolve_assets, validate_assets_exist
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
    assets = resolve_assets(scene_map, segment_count=len(segments))
    assets = resolve_asset_paths(assets, scene_map_path.resolve().parent)
    validate_assets_exist(assets)
    for asset in assets:
        if asset["type"] == "video":
            asset["source_duration"] = probe_duration_seconds(Path(asset["path"]))
    durations = [end - start for start, end in segments]

    burn_subtitle_path = srt_path if args.burn_subtitle else None
    cmd = build_command(
        audio_path=audio_path,
        assets=assets,
        durations=durations,
        output_path=output_path,
        burn_subtitle_path=burn_subtitle_path,
    )
    subprocess.run(cmd, check=True)
    print(f"動画を生成しました: {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
