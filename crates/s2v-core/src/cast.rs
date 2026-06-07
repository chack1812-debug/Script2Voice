use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Cast {
    pub name: String,
    pub speaker_name: String,
    pub engine_type: String,
    pub pan: f64,
    pub distance: f64,
    pub volume: f64,
    pub params: HashMap<String, Value>,
}

impl Cast {
    /// 臨時パラメータを適用した新 Cast を返す (Python版 create_effective_cast 相当)
    pub fn with_offsets(&self, offsets: &HashMap<String, f64>) -> Self {
        let mut cast = self.clone();
        for (k, &v) in offsets {
            match k.as_str() {
                "pan" => cast.pan = v,
                "distance" => cast.distance = v,
                "volume" => cast.volume = v,
                other => {
                    cast.params.insert(other.to_string(), Value::from(v));
                }
            }
        }
        cast
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_cast() -> Cast {
        Cast {
            name: "ずんだもん".to_string(),
            speaker_name: "ずんだもん".to_string(),
            engine_type: "voicevox".to_string(),
            pan: 0.0,
            distance: 1.0,
            volume: 1.0,
            params: {
                let mut m = HashMap::new();
                m.insert("style".to_string(), Value::String("ノーマル".to_string()));
                m.insert("speedScale".to_string(), serde_json::json!(1.0));
                m
            },
        }
    }

    #[test]
    fn with_offsets_overrides_pan() {
        let cast = base_cast();
        let mut offsets = HashMap::new();
        offsets.insert("pan".to_string(), 30.0_f64);
        let effective = cast.with_offsets(&offsets);
        assert!((effective.pan - 30.0).abs() < 1e-10);
        assert!((effective.distance - 1.0).abs() < 1e-10);
    }

    #[test]
    fn with_offsets_overrides_distance() {
        let cast = base_cast();
        let mut offsets = HashMap::new();
        offsets.insert("distance".to_string(), 2.5_f64);
        let effective = cast.with_offsets(&offsets);
        assert!((effective.distance - 2.5).abs() < 1e-10);
    }

    #[test]
    fn with_offsets_overrides_volume() {
        let cast = base_cast();
        let mut offsets = HashMap::new();
        offsets.insert("volume".to_string(), 0.8_f64);
        let effective = cast.with_offsets(&offsets);
        assert!((effective.volume - 0.8).abs() < 1e-10);
    }

    #[test]
    fn with_offsets_does_not_mutate_original() {
        let cast = base_cast();
        let mut offsets = HashMap::new();
        offsets.insert("pan".to_string(), 45.0_f64);
        let _effective = cast.with_offsets(&offsets);
        assert!((cast.pan - 0.0).abs() < 1e-10);
    }

    #[test]
    fn with_offsets_empty_leaves_cast_unchanged() {
        let cast = base_cast();
        let offsets = HashMap::new();
        let effective = cast.with_offsets(&offsets);
        assert!((effective.pan - cast.pan).abs() < 1e-10);
        assert!((effective.distance - cast.distance).abs() < 1e-10);
        assert!((effective.volume - cast.volume).abs() < 1e-10);
    }

    #[test]
    fn with_offsets_unknown_key_goes_to_params() {
        let cast = base_cast();
        let mut offsets = HashMap::new();
        offsets.insert("speedScale".to_string(), 1.5_f64);
        let effective = cast.with_offsets(&offsets);
        let speed = effective.params["speedScale"].as_f64().unwrap();
        assert!((speed - 1.5).abs() < 1e-10);
    }
}
