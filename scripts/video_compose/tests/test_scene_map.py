import json
from pathlib import Path

import pytest

from scene_map import load_scene_map, resolve_asset_paths, resolve_assets, validate_assets_exist


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


def test_resolve_assets_maps_index_to_image_path_legacy_form():
    scene_map = {
        "paragraphs": [
            {"index": 1, "image": "images/scene01.png"},
            {"index": 2, "image": "images/scene02.png"},
        ],
        "default_image": "images/default.png",
    }
    assert resolve_assets(scene_map, segment_count=2) == [
        {"type": "image", "path": "images/scene01.png"},
        {"type": "image", "path": "images/scene02.png"},
    ]


def test_resolve_assets_supports_type_and_path_form_with_video():
    scene_map = {
        "paragraphs": [
            {"index": 1, "type": "video", "path": "assets/p01.mp4"},
            {"index": 2, "path": "assets/p02.png"},
        ],
        "default_image": "assets/default.png",
    }
    assert resolve_assets(scene_map, segment_count=2) == [
        {"type": "video", "path": "assets/p01.mp4"},
        {"type": "image", "path": "assets/p02.png"},
    ]


def test_resolve_assets_falls_back_to_default_image_for_missing_index():
    scene_map = {
        "paragraphs": [{"index": 1, "image": "images/scene01.png"}],
        "default_image": "images/default.png",
    }
    assert resolve_assets(scene_map, segment_count=3) == [
        {"type": "image", "path": "images/scene01.png"},
        {"type": "image", "path": "images/default.png"},
        {"type": "image", "path": "images/default.png"},
    ]


def test_resolve_assets_raises_value_error_when_default_image_missing_for_gap():
    # 実際に発生した再現ケース: 段落2に対応するアセットが無く、default_imageも未設定。
    # 従来はNoneがそのままリストへ混入し、後段でTypeErrorとして曖昧にクラッシュしていた。
    scene_map = {"paragraphs": [{"index": 1, "image": "a.png"}]}
    with pytest.raises(ValueError, match="2"):
        resolve_assets(scene_map, segment_count=2)


def test_resolve_assets_raises_value_error_on_duplicate_index():
    scene_map = {
        "paragraphs": [
            {"index": 1, "image": "a.png"},
            {"index": 1, "image": "b.png"},
        ],
        "default_image": "d.png",
    }
    with pytest.raises(ValueError, match="重複"):
        resolve_assets(scene_map, segment_count=1)


def test_resolve_assets_raises_value_error_when_index_out_of_segment_range():
    scene_map = {
        "paragraphs": [{"index": 5, "image": "a.png"}],
        "default_image": "d.png",
    }
    with pytest.raises(ValueError, match="5"):
        resolve_assets(scene_map, segment_count=2)


def test_resolve_assets_raises_value_error_on_invalid_type():
    scene_map = {
        "paragraphs": [{"index": 1, "type": "audio", "path": "a.mp3"}],
        "default_image": "d.png",
    }
    with pytest.raises(ValueError, match="type"):
        resolve_assets(scene_map, segment_count=1)


def test_validate_assets_exist_raises_file_not_found_with_missing_paths(tmp_path):
    present = tmp_path / "present.png"
    present.write_bytes(b"")
    assets = [
        {"type": "image", "path": str(present)},
        {"type": "image", "path": str(tmp_path / "missing.png")},
    ]
    with pytest.raises(FileNotFoundError, match="missing.png"):
        validate_assets_exist(assets)


def test_validate_assets_exist_passes_when_all_paths_exist(tmp_path):
    present = tmp_path / "present.png"
    present.write_bytes(b"")
    validate_assets_exist([{"type": "image", "path": str(present)}])


def test_resolve_asset_paths_makes_relative_paths_relative_to_scene_map_dir():
    # review.txt指摘: 相対パスの基準がプロジェクトディレクトリではなくプロセスのCWDになっていた。
    # scene_map.jsonの置かれたディレクトリを基準にすべき。
    base_dir = Path("/project/subdir")
    assets = [
        {"type": "image", "path": "images/scene01.png"},
        {"type": "video", "path": "assets/p01.mp4"},
    ]
    resolved = resolve_asset_paths(assets, base_dir)
    assert resolved[0]["path"] == str(base_dir / "images/scene01.png")
    assert resolved[1]["path"] == str(base_dir / "assets/p01.mp4")


def test_resolve_asset_paths_leaves_absolute_paths_untouched():
    base_dir = Path("/project/subdir")
    absolute = str(Path("/elsewhere/default.png"))
    assets = [{"type": "image", "path": absolute}]
    resolved = resolve_asset_paths(assets, base_dir)
    assert resolved[0]["path"] == absolute


def test_resolve_asset_paths_preserves_other_fields():
    base_dir = Path("/project")
    assets = [{"type": "video", "path": "p01.mp4", "source_duration": 5.0}]
    resolved = resolve_asset_paths(assets, base_dir)
    assert resolved[0]["type"] == "video"
    assert resolved[0]["source_duration"] == 5.0
