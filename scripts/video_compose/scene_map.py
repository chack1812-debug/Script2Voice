"""scene_map.json を読み込み、表示セグメント番号 (1始まり) に対応するアセット(画像/動画)を解決する。"""
import json
from pathlib import Path


def load_scene_map(path: Path) -> dict:
    """scene_map.json を読み込んで辞書として返す。"""
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def _normalize_entry(entry: dict) -> dict:
    """scene_map.jsonの1エントリを {"type": "image"|"video", "path": str} に正規化する。

    新形式(type+path)と旧形式("image"キーのみ、常にtype="image"扱い)の両方を受け付ける。
    """
    if "path" in entry:
        return {"type": entry.get("type", "image"), "path": entry["path"]}
    return {"type": "image", "path": entry["image"]}


def resolve_assets(scene_map: dict, segment_count: int) -> list[dict]:
    """セグメント番号 1..segment_count に対応するアセット({"type","path"})のリストを返す。

    scene_map["paragraphs"] に対応する index のエントリが無い場合は
    scene_map["default_image"] (静止画)にフォールバックする
    （段落数 > アセット数でもエラー終了しないための仕様）。
    """
    by_index = {entry["index"]: _normalize_entry(entry) for entry in scene_map.get("paragraphs", [])}
    default_image = scene_map.get("default_image")
    default_asset = {"type": "image", "path": default_image} if default_image is not None else None
    return [by_index.get(index, default_asset) for index in range(1, segment_count + 1)]
