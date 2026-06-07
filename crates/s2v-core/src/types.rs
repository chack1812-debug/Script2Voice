use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::cast::Cast;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SceneConfig {
    pub name: String,
    pub room_size: f64,
    pub reverb_wet: f64,
}

impl SceneConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            room_size: 0.1,
            reverb_wet: 0.7,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum ScriptItem {
    Speech {
        cast_name: String,
        text: String,
        display_text: String,
        offset_params: HashMap<String, f64>,
        scene_config: SceneConfig,
    },
    Command(ScriptCommand),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum ScriptCommand {
    Pause(f64),
    Paragraph,
    BgmStart(String),
    BgmStop,
    Se(String),
    Parallel(usize),
}

#[derive(Debug, Clone)]
pub struct PauseConfig {
    pub sentence_ms: f64,
    pub cast_ms: f64,
    pub paragraph_ms: f64,
}

impl Default for PauseConfig {
    fn default() -> Self {
        Self {
            sentence_ms: 200.0,
            cast_ms: 500.0,
            paragraph_ms: 1000.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub config: SceneConfig,
    pub casts: HashMap<String, Cast>,
    pub items: Vec<ScriptItem>,
    pub pause_config: PauseConfig,
}

impl Scene {
    pub fn new(config: SceneConfig) -> Self {
        Self {
            config,
            casts: HashMap::new(),
            items: Vec::new(),
            pause_config: PauseConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_config_defaults() {
        let sc = SceneConfig::new("居間");
        assert_eq!(sc.name, "居間");
        assert!((sc.room_size - 0.1).abs() < 1e-10);
        assert!((sc.reverb_wet - 0.7).abs() < 1e-10);
    }

    #[test]
    fn scene_config_custom_values() {
        let sc = SceneConfig {
            name: "広場".to_string(),
            room_size: 0.8,
            reverb_wet: 0.3,
        };
        assert!((sc.room_size - 0.8).abs() < 1e-10);
    }

    #[test]
    fn script_item_speech_round_trip() {
        let item = ScriptItem::Speech {
            cast_name: "キャラA".to_string(),
            text: "こんにちは".to_string(),
            display_text: "こんにちは".to_string(),
            offset_params: HashMap::new(),
            scene_config: SceneConfig::new("室内"),
        };
        if let ScriptItem::Speech { cast_name, text, .. } = &item {
            assert_eq!(cast_name, "キャラA");
            assert_eq!(text, "こんにちは");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn script_command_variants_are_constructible() {
        let cmds = vec![
            ScriptCommand::Pause(300.0),
            ScriptCommand::Paragraph,
            ScriptCommand::BgmStart("bgm01.wav".to_string()),
            ScriptCommand::BgmStop,
            ScriptCommand::Se("se_door.wav".to_string()),
            ScriptCommand::Parallel(3),
        ];
        assert_eq!(cmds.len(), 6);
    }

    #[test]
    fn scene_new_starts_empty() {
        let scene = Scene::new(SceneConfig::new("テスト"));
        assert!(scene.items.is_empty());
        assert!(scene.casts.is_empty());
    }

    #[test]
    fn pause_config_defaults_are_reasonable() {
        let pc = PauseConfig::default();
        assert!(pc.sentence_ms > 0.0);
        assert!(pc.cast_ms >= pc.sentence_ms);
        assert!(pc.paragraph_ms >= pc.cast_ms);
    }

    #[test]
    fn scene_accepts_casts() {
        let mut scene = Scene::new(SceneConfig::new("屋外"));
        let cast = Cast {
            name: "主人公".to_string(),
            speaker_name: "ずんだもん".to_string(),
            engine_type: "voicevox".to_string(),
            pan: -15.0,
            distance: 1.5,
            volume: 1.0,
            params: HashMap::new(),
        };
        scene.casts.insert("主人公".to_string(), cast);
        assert!(scene.casts.contains_key("主人公"));
    }
}
