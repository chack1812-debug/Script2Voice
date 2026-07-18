//! scene_map.json を読み込み、表示セグメント番号(1始まり)に対応するアセットを解決する。
use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Image,
    Video,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub kind: AssetKind,
    pub path: String,
    pub source_duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct ParagraphEntry {
    pub index: i64,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    pub path: Option<String>,
    pub image: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SceneMap {
    #[serde(default)]
    pub paragraphs: Vec<ParagraphEntry>,
    #[serde(default)]
    pub default_image: Option<String>,
}

/// scene_map.json を読み込む。
pub fn load_scene_map(path: &Path) -> anyhow::Result<SceneMap> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("scene_map.json を読めません {}: {e}", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

/// scene_map.json の1エントリを (type文字列, path) に正規化する。
/// 新形式(type+path)と旧形式(imageキーのみ、常にimage扱い)の両方を受ける。
fn normalize_entry(entry: &ParagraphEntry) -> anyhow::Result<(String, String)> {
    if let Some(path) = &entry.path {
        Ok((entry.type_.clone().unwrap_or_else(|| "image".to_string()), path.clone()))
    } else if let Some(image) = &entry.image {
        Ok(("image".to_string(), image.clone()))
    } else {
        anyhow::bail!("scene_map.json: 段落番号 {} に path も image もありません", entry.index)
    }
}

/// セグメント番号 1..=segment_count に対応するアセット列を返す。
pub fn resolve_assets(scene_map: &SceneMap, segment_count: usize) -> anyhow::Result<Vec<Asset>> {
    let mut by_index: HashMap<i64, (String, String)> = HashMap::new();
    for entry in &scene_map.paragraphs {
        if by_index.contains_key(&entry.index) {
            anyhow::bail!("scene_map.json: 段落番号 {} が重複しています", entry.index);
        }
        by_index.insert(entry.index, normalize_entry(entry)?);
    }

    for (index, (type_str, _)) in &by_index {
        if !(1..=segment_count as i64).contains(index) {
            anyhow::bail!(
                "scene_map.json: 段落番号 {index} はSRTの段落数(1..{segment_count})の範囲外です"
            );
        }
        if type_str != "image" && type_str != "video" {
            anyhow::bail!(
                "scene_map.json: 段落番号 {index} の type が不正です: {type_str:?} (有効な値: image, video)"
            );
        }
    }

    let default_asset = scene_map.default_image.as_ref().map(|p| Asset {
        kind: AssetKind::Image,
        path: p.clone(),
        source_duration: None,
    });

    let mut result = Vec::with_capacity(segment_count);
    for index in 1..=segment_count as i64 {
        let asset = if let Some((type_str, path)) = by_index.get(&index) {
            Asset {
                kind: if type_str == "video" { AssetKind::Video } else { AssetKind::Image },
                path: path.clone(),
                source_duration: None,
            }
        } else if let Some(d) = &default_asset {
            d.clone()
        } else {
            anyhow::bail!(
                "scene_map.json: 段落番号 {index} に対応するアセットが無く、default_image も設定されていません"
            );
        };
        result.push(asset);
    }
    Ok(result)
}

/// 相対パスを base_dir(scene_map.json の置かれたディレクトリ)基準の絶対パスへ揃える。
pub fn resolve_asset_paths(assets: Vec<Asset>, base_dir: &Path) -> Vec<Asset> {
    assets
        .into_iter()
        .map(|a| {
            let p = Path::new(&a.path);
            let path = if p.is_absolute() {
                a.path.clone()
            } else {
                base_dir.join(p).to_string_lossy().into_owned()
            };
            Asset { path, ..a }
        })
        .collect()
}

/// 解決済みアセットの参照先がすべて実在することを検証する。
pub fn validate_assets_exist(assets: &[Asset]) -> anyhow::Result<()> {
    let missing: Vec<&str> = assets
        .iter()
        .filter(|a| !Path::new(&a.path).exists())
        .map(|a| a.path.as_str())
        .collect();
    if !missing.is_empty() {
        anyhow::bail!("scene_map.json が参照するアセットが見つかりません: {}", missing.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn img(p: &str) -> Asset {
        Asset { kind: AssetKind::Image, path: p.into(), source_duration: None }
    }
    fn vid(p: &str) -> Asset {
        Asset { kind: AssetKind::Video, path: p.into(), source_duration: None }
    }
    fn sm_from(json: &str) -> SceneMap {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn load_reads_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scene_map.json");
        std::fs::write(&path, r#"{"paragraphs":[{"index":1,"image":"images/scene01.png"}],"default_image":"images/default.png"}"#).unwrap();
        let sm = load_scene_map(&path).unwrap();
        assert_eq!(sm.default_image.as_deref(), Some("images/default.png"));
    }

    #[test]
    fn resolve_legacy_image_form() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"image":"images/scene01.png"},{"index":2,"image":"images/scene02.png"}],"default_image":"images/default.png"}"#);
        assert_eq!(
            resolve_assets(&sm, 2).unwrap(),
            vec![img("images/scene01.png"), img("images/scene02.png")]
        );
    }

    #[test]
    fn resolve_type_path_form_with_video() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"type":"video","path":"assets/p01.mp4"},{"index":2,"path":"assets/p02.png"}],"default_image":"assets/default.png"}"#);
        assert_eq!(
            resolve_assets(&sm, 2).unwrap(),
            vec![vid("assets/p01.mp4"), img("assets/p02.png")]
        );
    }

    #[test]
    fn resolve_falls_back_to_default_for_missing_index() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"image":"images/scene01.png"}],"default_image":"images/default.png"}"#);
        assert_eq!(
            resolve_assets(&sm, 3).unwrap(),
            vec![img("images/scene01.png"), img("images/default.png"), img("images/default.png")]
        );
    }

    #[test]
    fn resolve_errors_when_default_missing_for_gap() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"image":"a.png"}]}"#);
        let e = resolve_assets(&sm, 2).unwrap_err();
        assert!(e.to_string().contains("2"));
    }

    #[test]
    fn resolve_errors_on_duplicate_index() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"image":"a.png"},{"index":1,"image":"b.png"}],"default_image":"d.png"}"#);
        let e = resolve_assets(&sm, 1).unwrap_err();
        assert!(e.to_string().contains("重複"));
    }

    #[test]
    fn resolve_errors_when_index_out_of_range() {
        let sm = sm_from(r#"{"paragraphs":[{"index":5,"image":"a.png"}],"default_image":"d.png"}"#);
        let e = resolve_assets(&sm, 2).unwrap_err();
        assert!(e.to_string().contains("5"));
    }

    #[test]
    fn resolve_errors_on_invalid_type() {
        let sm = sm_from(r#"{"paragraphs":[{"index":1,"type":"audio","path":"a.mp3"}],"default_image":"d.png"}"#);
        let e = resolve_assets(&sm, 1).unwrap_err();
        assert!(e.to_string().contains("type"));
    }

    #[test]
    fn validate_errors_with_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present.png");
        std::fs::write(&present, b"").unwrap();
        let missing = dir.path().join("missing.png");
        let assets = vec![
            img(present.to_str().unwrap()),
            img(missing.to_str().unwrap()),
        ];
        let e = validate_assets_exist(&assets).unwrap_err();
        assert!(e.to_string().contains("missing.png"));
    }

    #[test]
    fn validate_passes_when_all_exist() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present.png");
        std::fs::write(&present, b"").unwrap();
        validate_assets_exist(&[img(present.to_str().unwrap())]).unwrap();
    }

    #[test]
    fn resolve_paths_makes_relative_to_base_dir() {
        let base = Path::new("/project/subdir");
        let resolved = resolve_asset_paths(
            vec![img("images/scene01.png"), vid("assets/p01.mp4")],
            base,
        );
        assert_eq!(resolved[0].path, base.join("images/scene01.png").to_string_lossy());
        assert_eq!(resolved[1].path, base.join("assets/p01.mp4").to_string_lossy());
    }

    #[test]
    fn resolve_paths_leaves_absolute_untouched() {
        let base = Path::new("/project/subdir");
        let absolute = Path::new("/elsewhere/default.png").to_string_lossy().into_owned();
        let resolved = resolve_asset_paths(vec![img(&absolute)], base);
        assert_eq!(resolved[0].path, absolute);
    }

    #[test]
    fn resolve_paths_preserves_other_fields() {
        let base = Path::new("/project");
        let asset = Asset { kind: AssetKind::Video, path: "p01.mp4".into(), source_duration: Some(5.0) };
        let resolved = resolve_asset_paths(vec![asset], base);
        assert_eq!(resolved[0].kind, AssetKind::Video);
        assert_eq!(resolved[0].source_duration, Some(5.0));
    }
}
