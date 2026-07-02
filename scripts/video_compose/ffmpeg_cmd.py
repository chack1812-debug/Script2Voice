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
    assets: list[dict],
    durations: list[float],
    output_path: Path,
    *,
    burn_subtitle_path: Path | None = None,
    width: int = 1920,
    height: int = 1080,
    crf: int = 18,
) -> list[str]:
    """ffmpeg コマンドを引数リストとして構築する。

    assets[i] (`{"type": "image"|"video", "path": str, "source_duration"?: float}`) を
    durations[i] 秒間表示するスライドショーを音声に重ねた
    MP4 (libx264/aac, crf指定の高品質設定) を生成するコマンドを返す。
    画像は静止画としてループ表示する。動画クリップは source_duration が
    durations[i] 以上ならトリミング、未満なら不足分を最終フレームで静止して埋める。
    burn_subtitle_path を指定すると字幕を動画に焼き込む。
    """
    if len(assets) != len(durations):
        raise ValueError("assets と durations は同じ長さでなければならない")
    if not assets:
        raise ValueError("assets が空です")

    cmd = ["ffmpeg", "-y", "-i", str(audio_path)]
    for asset, duration in zip(assets, durations):
        if asset["type"] == "video":
            cmd += ["-t", f"{duration:.3f}", "-i", str(asset["path"])]
        else:
            cmd += ["-loop", "1", "-t", f"{duration:.3f}", "-i", str(asset["path"])]

    filter_parts = []
    video_labels = []
    for i, (asset, duration) in enumerate(zip(assets, durations)):
        in_label = f"{i + 1}:v"
        out_label = f"v{i + 1}"
        source_duration = asset.get("source_duration") if asset["type"] == "video" else None
        deficit = duration - source_duration if source_duration is not None else 0.0
        if deficit > 0:
            scaled_label = f"v{i + 1}pre"
            filter_parts.append(_scale_pad_filter(in_label, scaled_label, width, height))
            filter_parts.append(f"[{scaled_label}]tpad=stop_mode=clone:stop_duration={deficit:.3f}[{out_label}]")
        else:
            filter_parts.append(_scale_pad_filter(in_label, out_label, width, height))
        video_labels.append(f"[{out_label}]")

    concat_inputs = "".join(video_labels)
    filter_parts.append(f"{concat_inputs}concat=n={len(assets)}:v=1:a=0[vout]")

    final_video_label = "[vout]"
    if burn_subtitle_path is not None:
        filter_parts.append(f"[vout]subtitles={_escape_subtitles_path(burn_subtitle_path)}[vsub]")
        final_video_label = "[vsub]"

    cmd += ["-filter_complex", ";".join(filter_parts)]
    cmd += ["-map", final_video_label, "-map", "0:a"]
    cmd += ["-c:v", "libx264", "-crf", str(crf), "-c:a", "aac", "-shortest"]
    cmd += [str(output_path)]
    return cmd
