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
