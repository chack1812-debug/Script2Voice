use std::path::Path;

/// 音響ラボのプリセット1件。すべて省略可（省略項目は現在のスライダー値を維持）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Preset {
    pub name: String,
    pub room_w: Option<f64>,
    pub room_d: Option<f64>,
    pub room_h: Option<f64>,
    pub listener_dx: Option<f64>,
    pub listener_dy: Option<f64>,
    pub listener_z: Option<f64>,
    pub reverb_wet: Option<f64>,
    pub pan: Option<f64>,
    pub distance: Option<f64>,
    pub height: Option<f64>,
}

#[derive(serde::Deserialize)]
struct PresetFile {
    #[serde(default)]
    preset: Vec<Preset>,
}

/// presets.toml が無くても使える組込みプリセット。
pub fn builtin_presets() -> Vec<Preset> {
    fn p(name: &str, w: f64, d: f64, h: f64, dy: f64, z: f64, wet: f64) -> Preset {
        Preset {
            name: name.into(),
            room_w: Some(w), room_d: Some(d), room_h: Some(h),
            listener_dx: Some(0.0), listener_dy: Some(dy), listener_z: Some(z),
            reverb_wet: Some(wet),
            pan: None, distance: None, height: None,
        }
    }
    vec![
        p("ラジオスタジオ", 4.0, 5.0, 3.0, 0.0, 1.2, 1.0),
        p("会議室", 8.0, 12.0, 2.7, 0.0, 1.2, 1.0),
        p("2000席ホール", 25.0, 45.0, 18.0, -15.0, 1.1, 1.0),
        p("屋外風（残響なし）", 50.0, 50.0, 30.0, 0.0, 1.6, 0.0),
    ]
}

/// 組込み＋presets.toml の内容を返す。第2要素は警告（破損時のみ）。
pub fn load_presets(path: &Path) -> (Vec<Preset>, Option<String>) {
    let mut presets = builtin_presets();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return (presets, None), // ファイル無しは正常系
    };
    match toml::from_str::<PresetFile>(&text) {
        Ok(file) => {
            presets.extend(file.preset);
            (presets, None)
        }
        Err(e) => (presets, Some(format!("presets.toml の読み込みに失敗（組込みのみ使用）: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn loads_presets_from_toml_after_builtins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "[[preset]]\nname = \"テスト部屋\"\nroom_w = 7.0\nroom_d = 8.0\nroom_h = 3.5\n").unwrap();
        drop(f);
        let (presets, warn) = load_presets(&path);
        assert!(warn.is_none());
        assert!(presets.len() > builtin_presets().len());
        let p = presets.iter().find(|p| p.name == "テスト部屋").unwrap();
        assert_eq!(p.room_w, Some(7.0));
        assert_eq!(p.reverb_wet, None);
    }

    #[test]
    fn missing_file_returns_builtins_without_warning() {
        let (presets, warn) = load_presets(Path::new("Z:/no/such/presets.toml"));
        assert_eq!(presets, builtin_presets());
        assert!(warn.is_none());
    }

    #[test]
    fn broken_toml_returns_builtins_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.toml");
        std::fs::write(&path, "[[preset]\nname=壊れてる").unwrap();
        let (presets, warn) = load_presets(&path);
        assert_eq!(presets, builtin_presets());
        assert!(warn.is_some());
    }
}
