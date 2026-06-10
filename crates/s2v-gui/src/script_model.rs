use s2v_core::{Cast, ParseWarning, Scene, SceneConfig, ScriptItem, ScriptParser};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// 行リスト1行分（プレビューに必要な情報を自己完結で持つ）。
#[derive(Debug, Clone)]
pub struct PreviewLine {
    /// 台本全体での speech 通し番号（1始まり = voice_NNNN と一致）
    pub no: usize,
    pub scene_name: String,
    pub cast_name: String,
    pub display_text: String,
    pub text: String,
    /// 行内臨時パラメータ適用済みの実効 Cast
    pub cast: Cast,
    pub scene_config: SceneConfig,
}

pub struct ScriptModel {
    pub path: PathBuf,
    pub lines: Vec<PreviewLine>,
    pub warnings: Vec<ParseWarning>,
    pub scenes: Vec<Scene>,
}

pub fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// 台本を読み込み、行リスト＋警告を構築する。失敗時はメッセージを返す（呼び出し側で前回モデルを保持）。
pub fn load(path: &Path) -> Result<ScriptModel, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("台本を読み込めません: {e}"))?;
    let mut parser = ScriptParser::new();
    let scenes = parser
        .parse_str(strip_bom(&text))
        .map_err(|e| format!("台本の解析に失敗: {e}"))?;
    let warnings = parser.warnings().to_vec();

    let mut lines = Vec::new();
    let mut no = 0usize;
    for scene in &scenes {
        for item in &scene.items {
            let ScriptItem::Speech {
                cast_name,
                text,
                display_text,
                offset_params,
                scene_config,
            } = item
            else {
                continue;
            };
            let Some(cast) = scene.casts.get(cast_name) else {
                continue;
            };
            no += 1;
            lines.push(PreviewLine {
                no,
                scene_name: scene.config.name.clone(),
                cast_name: cast_name.clone(),
                display_text: display_text.clone(),
                text: text.clone(),
                cast: cast.with_offsets(offset_params),
                scene_config: scene_config.clone(),
            });
        }
    }
    Ok(ScriptModel {
        path: path.to_path_buf(),
        lines,
        warnings,
        scenes,
    })
}

/// mtime ポーリングによるファイル変更検知。
pub struct WatchedFile {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    last_check: Instant,
    interval: Duration,
}

impl WatchedFile {
    pub fn new(path: PathBuf) -> Self {
        Self::with_interval(path, Duration::from_millis(500))
    }

    pub fn with_interval(path: PathBuf, interval: Duration) -> Self {
        let last_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Self {
            path,
            last_mtime,
            last_check: Instant::now(),
            interval,
        }
    }

    /// 変更があれば true（interval 間隔でのみ実チェック）。
    pub fn poll(&mut self) -> bool {
        if self.last_check.elapsed() < self.interval {
            return false;
        }
        self.last_check = Instant::now();
        let Ok(mtime) = std::fs::metadata(&self.path).and_then(|m| m.modified()) else {
            return false; // 一時的に消えている（エディタの保存中など）は無視
        };
        if self.last_mtime != Some(mtime) {
            self.last_mtime = Some(mtime);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SRC: &str = "@scene 一 room_size=0.3\n@cast\nA:話者:ノーマル,voicevox,pan=-30\n@script\nA:こんにちは\nA(pan=15):やあ\n誰か:無視される\n";

    #[test]
    fn strip_bom_removes_leading_bom_only() {
        assert_eq!(strip_bom("\u{feff}@scene"), "@scene");
        assert_eq!(strip_bom("@scene"), "@scene");
    }

    #[test]
    fn load_builds_lines_with_effective_cast_and_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("台本.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "\u{feff}{SRC}").unwrap(); // BOM 付きでも読めること
        drop(f);
        let m = load(&path).unwrap();
        assert_eq!(m.lines.len(), 2);
        assert_eq!(m.lines[0].no, 1);
        assert_eq!(m.lines[0].scene_name, "一");
        assert_eq!(m.lines[0].cast.pan, -30.0);
        assert_eq!(m.lines[1].cast.pan, -15.0, "行内 pan=15 は加算オフセット");
        assert_eq!(m.lines[1].scene_config.room_size, Some(0.3));
        assert_eq!(m.warnings.len(), 1, "未定義キャスト警告");
    }

    #[test]
    fn watched_file_reports_change_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("台本.txt");
        std::fs::write(&path, "a").unwrap();
        let mut w = WatchedFile::with_interval(path.clone(), Duration::ZERO);
        assert!(!w.poll(), "初回登録時は変更扱いしない");
        // mtime を未来に更新して変更を模擬
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(2)).unwrap();
        drop(f);
        assert!(w.poll());
        assert!(!w.poll(), "同じ mtime では再通知しない");
    }
}
