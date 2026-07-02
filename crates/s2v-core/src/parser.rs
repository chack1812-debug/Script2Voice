use std::collections::HashMap;

use serde_json::Value;

use crate::cast::Cast;
use crate::types::{PauseConfig, Scene, SceneConfig, ScriptCommand, ScriptItem};

/// パース中に検出した非致命的な問題（行は無視されるがパース自体は続行）。
#[derive(Debug, Clone, PartialEq)]
pub struct ParseWarning {
    /// 1始まりの行番号
    pub line_no: usize,
    pub message: String,
}

pub struct ScriptParser {
    casts: HashMap<String, Cast>,
    pause_config: PauseConfig,
    asset_config: HashMap<String, String>,
    warnings: Vec<ParseWarning>,
    /// `@cast`セクションで定義行を読んだ直後から、空行または次のセクションが来るまでの間、
    /// 収集中のキャスト名を保持する（自由記述の宛先を追跡するための状態）。
    pending_cast_name: Option<String>,
    /// `pending_cast_name`が`Some`の間に集めた自由記述の行（順序どおり）。
    pending_cast_appearance: Vec<String>,
}

impl ScriptParser {
    pub fn new() -> Self {
        Self {
            casts: HashMap::new(),
            pause_config: PauseConfig::default(),
            asset_config: HashMap::new(),
            warnings: Vec::new(),
            pending_cast_name: None,
            pending_cast_appearance: Vec::new(),
        }
    }

    /// パース中に検出した非致命的な警告（未定義キャストなど）を返す。
    pub fn warnings(&self) -> &[ParseWarning] {
        &self.warnings
    }

    pub fn parse_str(&mut self, text: &str) -> anyhow::Result<Vec<Scene>> {
        self.warnings.clear();
        self.pending_cast_name = None;
        self.pending_cast_appearance.clear();
        let mut scenes: Vec<Scene> = Vec::new();
        let mut current_scene: Option<Scene> = None;
        let mut section = "";

        for (idx, line) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = line.trim();
            if line.is_empty() {
                self.flush_pending_cast();
                continue;
            }

            if line.starts_with('@') {
                self.flush_pending_cast();
                if line.starts_with("@scene") {
                    if let Some(mut s) = current_scene.take() {
                        s.casts = self.casts.clone();
                        Self::fill_items_scene_config(&mut s);
                        scenes.push(s);
                    }
                    let scene_raw = line["@scene".len()..].trim();
                    let scene = self.parse_scene_header(scene_raw);
                    let mut s = Scene::new(scene);
                    s.pause_config = self.pause_config.clone();
                    current_scene = Some(s);
                    section = "@scene";
                } else if line.starts_with("@pause") {
                    section = "@pause";
                } else if line.starts_with("@asset") {
                    section = "@asset";
                } else if line.starts_with("@cast") {
                    section = "@cast";
                } else if line.starts_with("@script") {
                    section = "@script";
                    if let Some(ref mut s) = current_scene {
                        s.pause_config = self.pause_config.clone();
                    }
                }
                continue;
            }

            match section {
                "@pause" => self.parse_pause_line(line),
                "@asset" => self.parse_asset_line(line),
                "@cast" => {
                    if self.pending_cast_name.is_some() {
                        self.pending_cast_appearance.push(line.to_string());
                    } else {
                        self.parse_cast_line(line);
                    }
                }
                "@scene" => {
                    if let Some(ref mut s) = current_scene {
                        match s.config.description {
                            Some(ref mut desc) => {
                                desc.push('\n');
                                desc.push_str(line);
                            }
                            None => s.config.description = Some(line.to_string()),
                        }
                    }
                }
                "@script" => {
                    if let Some(ref mut scene) = current_scene {
                        if let Some(item) = self.parse_script_line(line, line_no) {
                            scene.items.push(item);
                        }
                    }
                }
                _ => {}
            }
        }

        self.flush_pending_cast();

        if let Some(mut s) = current_scene {
            s.casts = self.casts.clone();
            Self::fill_items_scene_config(&mut s);
            scenes.push(s);
        }

        Ok(scenes)
    }

    /// シーン確定時に、各 Speech アイテム埋め込みの scene_config へ Scene.config を転写する。
    /// (parse_script_line の時点ではシーン文脈を持たないため、ここで確定値を反映する)
    fn fill_items_scene_config(s: &mut Scene) {
        let cfg = s.config.clone();
        for item in s.items.iter_mut() {
            if let crate::types::ScriptItem::Speech { scene_config, .. } = item {
                *scene_config = cfg.clone();
            }
        }
    }

    /// 収集中のキャストの自由記述をバッファから確定させ、`Cast.appearance`へ書き込む。
    /// 何も収集していない場合は何もしない(空行・セクション境界のたびに無条件で呼んでよい)。
    fn flush_pending_cast(&mut self) {
        if let Some(name) = self.pending_cast_name.take() {
            if !self.pending_cast_appearance.is_empty() {
                if let Some(cast) = self.casts.get_mut(&name) {
                    cast.appearance = Some(self.pending_cast_appearance.join("\n"));
                }
            }
            self.pending_cast_appearance.clear();
        }
    }

    pub fn parse_file(&mut self, path: &std::path::Path) -> anyhow::Result<Vec<Scene>> {
        let text = std::fs::read_to_string(path)?;
        self.parse_str(&text)
    }

    fn parse_scene_header(&self, raw: &str) -> SceneConfig {
        let tokens: Vec<&str> = raw.split_whitespace().collect();
        let mut name_tokens = Vec::new();
        let mut param_tokens = Vec::new();
        for t in &tokens {
            if t.contains('=') {
                param_tokens.push(*t);
            } else {
                name_tokens.push(*t);
            }
        }
        let name = name_tokens.join(" ");
        let params = extract_kv_params(&param_tokens.join(","));
        SceneConfig {
            room_size: params.get("room_size").copied(),
            reverb_wet: params.get("reverb_wet").copied(),
            room_w: params.get("room_w").copied(),
            room_d: params.get("room_d").copied(),
            room_h: params.get("room_h").copied(),
            listener_dx: params.get("listener_dx").copied(),
            listener_dy: params.get("listener_dy").copied(),
            listener_z: params.get("listener_z").copied(),
            ..SceneConfig::new(name)
        }
    }

    fn parse_pause_line(&mut self, line: &str) {
        let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
        if parts.len() != 2 {
            return;
        }
        let key = parts[0].trim();
        let Ok(val) = parts[1].trim().parse::<f64>() else {
            return;
        };
        match key {
            "sentence" | "sentens" => self.pause_config.sentence_ms = val,
            "cast" => self.pause_config.cast_ms = val,
            "paragraph" => self.pause_config.paragraph_ms = val,
            _ => {}
        }
    }

    fn parse_asset_line(&mut self, line: &str) {
        if let Some((k, v)) = line.split_once('=') {
            self.asset_config.insert(k.trim().to_string(), v.trim().to_string());
        }
    }

    fn parse_cast_line(&mut self, line: &str) {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 3 {
            return;
        }
        let name = parts[0].trim().to_string();
        let speaker_name = parts[1].trim().to_string();
        let remain = parts[2].trim();

        let sub: Vec<&str> = remain.splitn(3, ',').collect();
        let style = sub.first().map(|s| s.trim()).unwrap_or("").to_string();
        let engine_type = sub.get(1).map(|s| s.trim()).unwrap_or("").to_string();
        let params_str = sub.get(2).copied().unwrap_or("");

        let mut raw = extract_kv_params(params_str);
        let pan = raw.remove("pan").unwrap_or(0.0);
        let distance = raw.remove("distance").unwrap_or(1.0);
        let volume = raw.remove("volume").unwrap_or(1.0);
        let height = raw.remove("height");

        let mut params: HashMap<String, Value> = raw
            .into_iter()
            .map(|(k, v)| (k, Value::from(v)))
            .collect();
        params.insert("style".to_string(), Value::String(style));

        let cast_key = name.clone();
        self.casts.insert(
            cast_key.clone(),
            Cast { name, speaker_name, engine_type, pan, distance, volume, params, height, height_offset: 0.0, appearance: None },
        );
        self.pending_cast_name = Some(cast_key);
    }

    fn parse_script_line(&mut self, line: &str, line_no: usize) -> Option<ScriptItem> {
        // 数字のみ → Parallel
        if line.trim().chars().all(|c| c.is_ascii_digit()) && !line.trim().is_empty() {
            let n: usize = line.trim().parse().ok()?;
            return Some(ScriptItem::Command(ScriptCommand::Parallel(n)));
        }

        // # コマンド行
        if line.starts_with('#') {
            let rest = &line[1..];
            // `# ` または `#\t` → コメント
            if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
                return None;
            }
            let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
            let cmd = parts[0];
            let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");
            return match cmd {
                "pause" => arg.parse::<f64>().ok().map(|ms| ScriptItem::Command(ScriptCommand::Pause(ms))),
                "paragraph" => Some(ScriptItem::Command(ScriptCommand::Paragraph)),
                "bgm_start" => Some(ScriptItem::Command(ScriptCommand::BgmStart(arg.to_string()))),
                "bgm_stop" => Some(ScriptItem::Command(ScriptCommand::BgmStop)),
                "se" => Some(ScriptItem::Command(ScriptCommand::Se(arg.to_string()))),
                _ => None,
            };
        }

        // 台詞行: `役名(params):テキスト` or `役名:テキスト`
        let sep = if line.contains(':') {
            ':'
        } else if line.contains('：') {
            '：'
        } else {
            return None;
        };
        let (name_part, raw_text) = line.split_once(sep)?;
        let name_part = name_part.trim();
        let raw_text = raw_text.trim();

        let (role, params_str) = if let Some(idx) = name_part.find('(') {
            let role = name_part[..idx].trim();
            let params_inner = name_part[idx + 1..].trim_end_matches(')');
            (role, params_inner)
        } else {
            (name_part, "")
        };

        if !self.casts.contains_key(role) {
            self.warnings.push(ParseWarning {
                line_no,
                message: format!("キャスト「{role}」が未定義です（この行は無視されます）"),
            });
            return None;
        }

        let (text, display_text) = expand_ruby(raw_text);
        let offset_params = extract_kv_params(params_str);

        Some(ScriptItem::Speech {
            cast_name: role.to_string(),
            text,
            display_text,
            offset_params,
            scene_config: crate::types::SceneConfig::new(String::new()),
        })
    }
}

impl Default for ScriptParser {
    fn default() -> Self {
        Self::new()
    }
}

/// `'word:reading'` → (reading_text, display_text) に展開
fn expand_ruby(text: &str) -> (String, String) {
    let mut synthesis = text.to_string();
    let mut display = text.to_string();

    let re = regex::Regex::new(r"'([^':：]+?)[:：]([^':：]+?)'").unwrap();
    for cap in re.captures_iter(text) {
        let full = &cap[0];
        let word = &cap[1];
        let reading = &cap[2];
        synthesis = synthesis.replace(full, reading);
        display = display.replace(full, word);
    }
    (synthesis, display)
}

/// カンマ区切り `key=value` 文字列から HashMap<String, f64> を抽出
fn extract_kv_params(s: &str) -> HashMap<String, f64> {
    let mut map = HashMap::new();
    for part in s.split(',') {
        if let Some((k, v)) = part.split_once('=') {
            if let Ok(val) = v.trim().parse::<f64>() {
                map.insert(k.trim().to_string(), val);
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ScriptCommand, ScriptItem};

    const SIMPLE_SCRIPT: &str = r#"
@scene 居間 room_size=0.3 reverb_wet=0.5

@pause
sentence 200
cast 500
paragraph 1000

@cast
ずんだもん:ずんだもん:ノーマル,voicevox,pan=-30,distance=1.0

四国めたん:四国めたん:ノーマル,voicevox,pan=30

@script
ずんだもん:こんにちは！
四国めたん:こんにちは！
#pause 500
#paragraph
ずんだもん:またね。
"#;

    #[test]
    fn parses_single_scene() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        assert_eq!(scenes.len(), 1);
        assert_eq!(scenes[0].config.name, "居間");
    }

    #[test]
    fn parses_scene_room_params() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        let sc = &scenes[0].config;
        assert!((sc.room_size.unwrap() - 0.3).abs() < 1e-6);
        assert!((sc.reverb_wet.unwrap() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn speech_items_carry_their_scene_config() {
        // produce() は Speech アイテム埋め込みの scene_config を音響処理に渡すため、
        // パーサがシーン確定時に Scene.config をアイテムへ転写していることを保証する。
        let src = "@scene 一 room_size=0.8 reverb_wet=0.3\n\
                   @cast\nA:話者:ノーマル,voicevox,pan=0\n\
                   @script\nA:こんにちは\n\
                   @scene 二 room_w=20 room_d=30 room_h=10 listener_z=1.5\n\
                   @script\nA:こんばんは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        assert_eq!(scenes.len(), 2);

        let ScriptItem::Speech { scene_config, .. } = &scenes[0].items[0] else {
            panic!("scene0 items[0] が Speech ではない");
        };
        assert_eq!(scene_config.name, "一");
        assert_eq!(scene_config.room_size, Some(0.8));
        assert_eq!(scene_config.reverb_wet, Some(0.3));

        let ScriptItem::Speech { scene_config, .. } = &scenes[1].items[0] else {
            panic!("scene1 items[0] が Speech ではない");
        };
        assert_eq!(scene_config.name, "二");
        assert_eq!(scene_config.room_w, Some(20.0));
        assert_eq!(scene_config.room_d, Some(30.0));
        assert_eq!(scene_config.room_h, Some(10.0));
        assert_eq!(scene_config.listener_z, Some(1.5));
    }

    #[test]
    fn parses_cast_entries() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        let casts = &scenes[0].casts;
        assert!(casts.contains_key("ずんだもん"));
        let c = &casts["ずんだもん"];
        assert_eq!(c.engine_type, "voicevox");
        assert!((c.pan - (-30.0)).abs() < 1e-6);
        assert!((c.distance - 1.0).abs() < 1e-6);
    }

    #[test]
    fn parses_speech_items() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        let speeches: Vec<_> = scenes[0].items.iter().filter(|i| {
            matches!(i, ScriptItem::Speech { .. })
        }).collect();
        assert_eq!(speeches.len(), 3);
        if let ScriptItem::Speech { cast_name, text, .. } = &speeches[0] {
            assert_eq!(cast_name, "ずんだもん");
            assert_eq!(text, "こんにちは！");
        } else {
            panic!("expected speech");
        }
    }

    #[test]
    fn parses_pause_command() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        let pause_item = scenes[0].items.iter().find(|i| {
            matches!(i, ScriptItem::Command(ScriptCommand::Pause(_)))
        });
        assert!(pause_item.is_some());
        if let ScriptItem::Command(ScriptCommand::Pause(ms)) = pause_item.unwrap() {
            assert!((ms - 500.0).abs() < 1e-6);
        }
    }

    #[test]
    fn parses_paragraph_command() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        let found = scenes[0].items.iter().any(|i| {
            matches!(i, ScriptItem::Command(ScriptCommand::Paragraph))
        });
        assert!(found);
    }

    #[test]
    fn parses_multiple_scenes() {
        let script = r#"
@scene 居間 room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:こんにちは

@scene 屋外 room_size=0.8 reverb_wet=0.2

@script
A:さようなら
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        assert_eq!(scenes.len(), 2);
        assert_eq!(scenes[0].config.name, "居間");
        assert_eq!(scenes[1].config.name, "屋外");
        assert!((scenes[1].config.room_size.unwrap() - 0.8).abs() < 1e-6);
    }

    #[test]
    fn comment_lines_are_skipped() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:セリフ
# これはコメント
A:続き
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        let count = scenes[0].items.iter().filter(|i| matches!(i, ScriptItem::Speech { .. })).count();
        assert_eq!(count, 2);
    }

    #[test]
    fn parallel_command_parsed() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox
B:B:スタイル,voicevox

@script
2
A:セリフA
B:セリフB
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        let parallel = scenes[0].items.iter().find(|i| {
            matches!(i, ScriptItem::Command(ScriptCommand::Parallel(_)))
        });
        assert!(parallel.is_some());
        if let ScriptItem::Command(ScriptCommand::Parallel(n)) = parallel.unwrap() {
            assert_eq!(*n, 2);
        }
    }

    #[test]
    fn ruby_notation_separates_text_and_display() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:'東京:とうきょう'に行く
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        if let ScriptItem::Speech { text, display_text, .. } = &scenes[0].items[0] {
            assert_eq!(text, "とうきょうに行く");
            assert_eq!(display_text, "東京に行く");
        } else {
            panic!("expected speech");
        }
    }

    #[test]
    fn bgm_start_stop_parsed() {
        let script = r#"
@scene テスト room_size=0.1

@cast

@script
#bgm_start bgm01.wav
#bgm_stop
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        let bgm_start = scenes[0].items.iter().any(|i| {
            matches!(i, ScriptItem::Command(ScriptCommand::BgmStart(_)))
        });
        let bgm_stop = scenes[0].items.iter().any(|i| {
            matches!(i, ScriptItem::Command(ScriptCommand::BgmStop))
        });
        assert!(bgm_start, "bgm_start not found");
        assert!(bgm_stop, "bgm_stop not found");
    }

    #[test]
    fn pause_config_from_section() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        let pc = &scenes[0].pause_config;
        assert!((pc.sentence_ms - 200.0).abs() < 1e-6);
        assert!((pc.cast_ms - 500.0).abs() < 1e-6);
        assert!((pc.paragraph_ms - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn speech_with_inline_params() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox,pan=0

@script
A(pan=15,distance=2):セリフ
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        if let ScriptItem::Speech { offset_params, .. } = &scenes[0].items[0] {
            assert!((offset_params["pan"] - 15.0).abs() < 1e-6);
            assert!((offset_params["distance"] - 2.0).abs() < 1e-6);
        } else {
            panic!("expected speech");
        }
    }

    #[test]
    fn scene_header_parses_room_dims_and_listener() {
        let p = ScriptParser::new();
        let sc = p.parse_scene_header("ホール room_w=25 room_d=45 room_h=18 listener_dx=0 listener_dy=-15");
        assert_eq!(sc.name, "ホール");
        assert_eq!(sc.room_w, Some(25.0));
        assert_eq!(sc.room_d, Some(45.0));
        assert_eq!(sc.room_h, Some(18.0));
        assert_eq!(sc.listener_dx, Some(0.0));
        assert_eq!(sc.listener_dy, Some(-15.0));
    }

    #[test]
    fn scene_header_room_dims_default_none() {
        let p = ScriptParser::new();
        let sc = p.parse_scene_header("小部屋 room_size=0.1");
        assert_eq!(sc.room_w, None);
        assert_eq!(sc.room_d, None);
        assert_eq!(sc.room_h, None);
        assert_eq!(sc.listener_dx, None);
        assert_eq!(sc.listener_dy, None);
        assert_eq!(sc.room_size, Some(0.1));
    }

    #[test]
    fn scene_header_parses_listener_z() {
        let p = ScriptParser::new();
        let sc = p.parse_scene_header("舞台 room_w=20 room_d=30 room_h=12 listener_z=1.1");
        assert_eq!(sc.listener_z, Some(1.1));
        let sc2 = p.parse_scene_header("小部屋 room_size=0.1");
        assert_eq!(sc2.listener_z, None);
    }

    #[test]
    fn unknown_cast_produces_warning_with_line_number() {
        let mut p = ScriptParser::new();
        let src = "@scene テスト room_size=0.1\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\nA:こんにちは\n誰か:こんばんは\n";
        let scenes = p.parse_str(src).unwrap();
        // 未定義キャスト行は従来どおり無視される
        let n = scenes[0].items.iter().filter(|i| matches!(i, ScriptItem::Speech { .. })).count();
        assert_eq!(n, 1);
        // 警告が行番号付きで記録される
        let w = p.warnings();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].line_no, 6);
        assert!(w[0].message.contains("誰か"));
    }

    #[test]
    fn warnings_are_reset_per_parse() {
        let mut p = ScriptParser::new();
        let src = "@scene テスト room_size=0.1\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\n誰か:こんばんは\n";
        p.parse_str(src).unwrap();
        assert_eq!(p.warnings().len(), 1);
        p.parse_str(src).unwrap();
        assert_eq!(p.warnings().len(), 1, "2回目のparseで累積しない");
    }

    #[test]
    fn cast_line_parses_height_into_field() {
        let scenes = ScriptParser::new()
            .parse_str("@scene テスト room_size=0.1\n@cast\nA:話者:ノーマル,voicevox,pan=0,height=1.7\n@script\nA: こんにちは\n")
            .unwrap();
        let cast = scenes[0].casts.get("A").unwrap();
        assert_eq!(cast.height, Some(1.7));
        assert!((cast.height_offset - 0.0).abs() < 1e-10);
    }

    #[test]
    fn cast_appearance_collected_until_blank_line() {
        let src = "@scene テスト room_size=0.1\n\
                   @cast\n\
                   ずんだもん:ずんだもん:ノーマル,voicevox,pan=0\n\
                   小柄で緑髪の元気なキャラクター。\n\
                   ずんだ餅のイメージカラーの服を着ている。\n\
                   \n\
                   @script\n\
                   ずんだもん:こんにちは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        let cast = scenes[0].casts.get("ずんだもん").unwrap();
        assert_eq!(
            cast.appearance.as_deref(),
            Some("小柄で緑髪の元気なキャラクター。\nずんだ餅のイメージカラーの服を着ている。")
        );
    }

    #[test]
    fn cast_without_free_text_has_no_appearance() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        let cast = scenes[0].casts.get("ずんだもん").unwrap();
        assert_eq!(cast.appearance, None);
    }

    #[test]
    fn cast_entries_without_blank_line_separator_merge_into_appearance() {
        // 空行を挟まないと、次のキャスト定義行が前のキャストの自由記述として飲み込まれる(仕様どおりの制約)
        let src = "@scene テスト room_size=0.1\n\
                   @cast\n\
                   A:話者A:ノーマル,voicevox,pan=0\n\
                   B:話者B:ノーマル,voicevox,pan=10\n\
                   \n\
                   @script\n\
                   A:こんにちは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        assert!(scenes[0].casts.contains_key("A"));
        assert!(!scenes[0].casts.contains_key("B"));
        let cast_a = scenes[0].casts.get("A").unwrap();
        assert_eq!(
            cast_a.appearance.as_deref(),
            Some("B:話者B:ノーマル,voicevox,pan=10")
        );
    }

    #[test]
    fn cast_appearance_flushes_without_trailing_blank_line_before_next_section() {
        let src = "@scene テスト room_size=0.1\n\
                   @cast\n\
                   A:話者A:ノーマル,voicevox,pan=0\n\
                   眼鏡をかけた青年。\n\
                   @script\n\
                   A:こんにちは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        let cast = scenes[0].casts.get("A").unwrap();
        assert_eq!(cast.appearance.as_deref(), Some("眼鏡をかけた青年。"));
    }

    #[test]
    fn scene_description_collected_until_next_section() {
        let src = "@scene 教室 room_size=0.3\n\
                   放課後の静かな教室。窓から夕日が差し込んでいる。\n\
                   黒板には日直の名前が書かれている。\n\
                   @pause\n\
                   sentence 200\n\
                   @cast\n\
                   A:A:ノーマル,voicevox\n\
                   @script\n\
                   A:こんにちは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        assert_eq!(
            scenes[0].config.description.as_deref(),
            Some("放課後の静かな教室。窓から夕日が差し込んでいる。\n黒板には日直の名前が書かれている。")
        );
    }

    #[test]
    fn scene_without_free_text_has_no_description() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        assert_eq!(scenes[0].config.description, None);
    }

    #[test]
    fn scene_description_ignores_blank_lines_within_block() {
        let src = "@scene 教室 room_size=0.3\n\
                   一行目の描写。\n\
                   \n\
                   二行目の描写。\n\
                   @cast\n\
                   A:A:ノーマル,voicevox\n\
                   @script\n\
                   A:こんにちは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        assert_eq!(
            scenes[0].config.description.as_deref(),
            Some("一行目の描写。\n二行目の描写。")
        );
    }
}
