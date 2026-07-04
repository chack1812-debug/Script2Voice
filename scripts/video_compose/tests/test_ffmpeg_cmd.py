from pathlib import Path

import pytest

from ffmpeg_cmd import build_command


def _input_paths(cmd):
    """コマンド引数列から -i に渡されたパスを順番に取り出す。"""
    return [cmd[i + 1] for i, arg in enumerate(cmd) if arg == "-i"]


def _image(path):
    return {"type": "image", "path": path}


def _video(path, source_duration):
    return {"type": "video", "path": path, "source_duration": source_duration}


def test_build_command_feeds_audio_then_looped_timed_images():
    cmd = build_command(
        audio_path=Path("project/full_dialogue.wav"),
        assets=[_image("images/scene01.png"), _image("images/scene02.png")],
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
        assets=[_image("s1.png"), _image("s2.png"), _image("s3.png")],
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
        assets=[_image("s1.png")],
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
        assets=[_image("s1.png")],
        durations=[5.0],
        output_path=Path("out.mp4"),
    )
    assert cmd[cmd.index("-c:v") + 1] == "libx264"
    assert cmd[cmd.index("-crf") + 1] == "18"
    assert "-shortest" in cmd
    assert cmd[-1] == "out.mp4"


def test_build_command_forces_yuv420p_and_faststart_for_player_compatibility():
    cmd = build_command(
        audio_path=Path("a.wav"),
        assets=[_image("s1.png")],
        durations=[5.0],
        output_path=Path("out.mp4"),
    )
    assert cmd[cmd.index("-pix_fmt") + 1] == "yuv420p"
    assert "-movflags" in cmd
    assert cmd[cmd.index("-movflags") + 1] == "+faststart"


def test_build_command_raises_when_assets_and_durations_length_mismatch():
    with pytest.raises(ValueError):
        build_command(
            audio_path=Path("a.wav"),
            assets=[_image("s1.png"), _image("s2.png")],
            durations=[1.0],
            output_path=Path("out.mp4"),
        )


def test_build_command_feeds_video_clip_without_loop_flag():
    cmd = build_command(
        audio_path=Path("a.wav"),
        assets=[_video("assets/p01.mp4", source_duration=10.0)],
        durations=[5.0],
        output_path=Path("out.mp4"),
    )
    assert "-loop" not in cmd
    t_idx = cmd.index("-t")
    assert cmd[t_idx:t_idx + 4] == ["-t", "5.000", "-i", "assets/p01.mp4"]


def test_build_command_trims_video_clip_longer_than_duration_without_tpad():
    cmd = build_command(
        audio_path=Path("a.wav"),
        assets=[_video("assets/p01.mp4", source_duration=10.0)],
        durations=[5.0],
        output_path=Path("out.mp4"),
    )
    filter_complex = cmd[cmd.index("-filter_complex") + 1]
    assert "tpad" not in filter_complex


def test_build_command_freezes_last_frame_for_video_clip_shorter_than_duration():
    cmd = build_command(
        audio_path=Path("a.wav"),
        assets=[_video("assets/p01.mp4", source_duration=3.0)],
        durations=[5.0],
        output_path=Path("out.mp4"),
    )
    filter_complex = cmd[cmd.index("-filter_complex") + 1]
    assert "tpad=stop_mode=clone:stop_duration=2.000" in filter_complex
    assert "[v1pre]tpad=stop_mode=clone:stop_duration=2.000[v1]" in filter_complex


def test_build_command_mixes_image_and_video_assets_in_concat():
    cmd = build_command(
        audio_path=Path("a.wav"),
        assets=[_image("s1.png"), _video("assets/p02.mp4", source_duration=8.0)],
        durations=[1.0, 2.0],
        output_path=Path("out.mp4"),
    )
    filter_complex = cmd[cmd.index("-filter_complex") + 1]
    assert "[v1][v2]concat=n=2:v=1:a=0[vout]" in filter_complex
