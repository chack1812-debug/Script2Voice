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
