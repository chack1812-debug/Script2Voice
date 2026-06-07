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
