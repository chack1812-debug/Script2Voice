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


_VALID_ASSET_TYPES = ("image", "video")


def resolve_assets(scene_map: dict, segment_count: int) -> list[dict]:
    """セグメント番号 1..segment_count に対応するアセット({"type","path"})のリストを返す。

    scene_map["paragraphs"] に対応する index のエントリが無い場合は
    scene_map["default_image"] (静止画)にフォールバックする
    （段落数 > アセット数でもエラー終了しないための仕様）。

    設定ミスを実行前に検出するため、以下はすべて ValueError にする
    （曖昧な TypeError で後段のffmpegコマンド構築が失敗するのを防ぐため）:
    - index の重複
    - segment_count の範囲外を指す index
    - type が "image"/"video" 以外
    - index に対応するエントリも default_image も無い(=解決先が無い)
    """
    paragraphs = scene_map.get("paragraphs", [])
    by_index: dict[int, dict] = {}
    for entry in paragraphs:
        index = entry["index"]
        if index in by_index:
            raise ValueError(f"scene_map.json: 段落番号 {index} が重複しています")
        by_index[index] = _normalize_entry(entry)

    for index, asset in by_index.items():
        if not (1 <= index <= segment_count):
            raise ValueError(
                f"scene_map.json: 段落番号 {index} はSRTの段落数(1..{segment_count})の範囲外です"
            )
        if asset["type"] not in _VALID_ASSET_TYPES:
            raise ValueError(
                f"scene_map.json: 段落番号 {index} の type が不正です: {asset['type']!r} "
                f"(有効な値: {', '.join(_VALID_ASSET_TYPES)})"
            )

    default_image = scene_map.get("default_image")
    default_asset = {"type": "image", "path": default_image} if default_image is not None else None

    result = []
    for index in range(1, segment_count + 1):
        asset = by_index.get(index, default_asset)
        if asset is None:
            raise ValueError(
                f"scene_map.json: 段落番号 {index} に対応するアセットが無く、"
                "default_image も設定されていません"
            )
        result.append(asset)
    return result


def resolve_asset_paths(assets: list[dict], base_dir: Path) -> list[dict]:
    """アセットの相対パスを、scene_map.jsonの置かれたディレクトリ(base_dir)基準の絶対パスへ揃える。

    従来はプロセスのカレントディレクトリ基準のまま`asset["path"]`を使っていたため、
    scene_map.jsonをプロジェクトディレクトリ以外の場所から起動すると解決に失敗していた
    (review.txt指摘)。絶対パスはそのまま変更しない。
    """
    resolved = []
    for asset in assets:
        path = Path(asset["path"])
        if not path.is_absolute():
            path = base_dir / path
        resolved.append({**asset, "path": str(path)})
    return resolved


def validate_assets_exist(assets: list[dict]) -> None:
    """解決済みアセットが指すパスがすべて実在することを検証する。

    ffmpeg にそのまま渡すと不可解な失敗になるため、ここで実行前にまとめて検出する。
    """
    missing = [asset["path"] for asset in assets if not Path(asset["path"]).exists()]
    if missing:
        raise FileNotFoundError(
            "scene_map.json が参照するアセットが見つかりません: " + ", ".join(missing)
        )
