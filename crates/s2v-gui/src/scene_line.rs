use s2v_core::{Cast, SceneConfig};
use std::collections::HashMap;

/// 音響ラボのスライダー状態。
#[derive(Debug, Clone, PartialEq)]
pub struct LabParams {
    pub room_w: f64,
    pub room_d: f64,
    pub room_h: f64,
    pub listener_dx: f64,
    pub listener_dy: f64,
    pub listener_z: f64,
    pub reverb_wet: f64,
    pub pan: f64,
    pub distance: f64,
    pub height: f64,
}

impl Default for LabParams {
    fn default() -> Self {
        Self {
            room_w: 4.0,
            room_d: 5.0,
            room_h: 3.0,
            listener_dx: 0.0,
            listener_dy: 0.0,
            listener_z: 1.2,
            reverb_wet: 1.0,
            pan: 0.0,
            distance: 1.0,
            height: 1.2,
        }
    }
}

impl LabParams {
    pub fn apply_preset(&mut self, p: &crate::presets::Preset) {
        if let Some(v) = p.room_w {
            self.room_w = v;
        }
        if let Some(v) = p.room_d {
            self.room_d = v;
        }
        if let Some(v) = p.room_h {
            self.room_h = v;
        }
        if let Some(v) = p.listener_dx {
            self.listener_dx = v;
        }
        if let Some(v) = p.listener_dy {
            self.listener_dy = v;
        }
        if let Some(v) = p.listener_z {
            self.listener_z = v;
        }
        if let Some(v) = p.reverb_wet {
            self.reverb_wet = v;
        }
        if let Some(v) = p.pan {
            self.pan = v;
        }
        if let Some(v) = p.distance {
            self.distance = v;
        }
        if let Some(v) = p.height {
            self.height = v;
        }
    }

    pub fn to_scene_config(&self, name: &str) -> SceneConfig {
        let mut sc = SceneConfig::new(name);
        sc.room_w = Some(self.room_w);
        sc.room_d = Some(self.room_d);
        sc.room_h = Some(self.room_h);
        sc.listener_dx = Some(self.listener_dx);
        sc.listener_dy = Some(self.listener_dy);
        sc.listener_z = Some(self.listener_z);
        sc.reverb_wet = Some(self.reverb_wet);
        sc
    }

    /// 音響ラボ用の合成 Cast（エンジン非依存。engine_volume_offsets は未登録キー→1.0）。
    pub fn to_cast(&self) -> Cast {
        Cast {
            name: "ラボ".into(),
            speaker_name: String::new(),
            engine_type: String::new(),
            pan: self.pan,
            distance: self.distance,
            volume: 1.0,
            params: HashMap::new(),
            height: Some(self.height),
            height_offset: 0.0,
            appearance: None,
        }
    }

    /// 台本に貼り付けられる @scene 行を生成する。
    pub fn scene_line(&self, scene_name: &str) -> String {
        format!(
            "@scene {} room_w={} room_d={} room_h={} listener_dx={} listener_dy={} listener_z={} reverb_wet={}",
            scene_name,
            self.room_w,
            self.room_d,
            self.room_h,
            self.listener_dx,
            self.listener_dy,
            self.listener_z,
            self.reverb_wet,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_line_roundtrips_through_parser() {
        let mut p = LabParams::default();
        p.room_w = 25.0;
        p.room_d = 45.0;
        p.room_h = 18.0;
        p.listener_dy = -15.0;
        p.listener_z = 1.1;
        p.reverb_wet = 0.5;
        let line = p.scene_line("ラボ");
        // 生成した @scene 行を実パーサに通して値が一致することを確認
        let src = format!("{line}\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\nA:あ\n");
        let mut parser = s2v_core::ScriptParser::new();
        let scenes = parser.parse_str(&src).unwrap();
        let sc = &scenes[0].config;
        assert_eq!(sc.name, "ラボ");
        assert_eq!(sc.room_w, Some(25.0));
        assert_eq!(sc.room_d, Some(45.0));
        assert_eq!(sc.room_h, Some(18.0));
        assert_eq!(sc.listener_dx, Some(0.0));
        assert_eq!(sc.listener_dy, Some(-15.0));
        assert_eq!(sc.listener_z, Some(1.1));
        assert_eq!(sc.reverb_wet, Some(0.5));
    }

    #[test]
    fn apply_preset_overrides_only_given_fields() {
        let mut p = LabParams::default();
        p.pan = 30.0;
        let preset = crate::presets::builtin_presets()
            .into_iter()
            .find(|p| p.name == "2000席ホール")
            .unwrap();
        p.apply_preset(&preset);
        assert_eq!(p.room_w, 25.0);
        assert_eq!(p.listener_dy, -15.0);
        assert_eq!(p.pan, 30.0, "preset に無い項目は維持");
    }

    #[test]
    fn to_cast_and_scene_config_carry_values() {
        let p = LabParams::default();
        let c = p.to_cast();
        assert_eq!(c.distance, 1.0);
        assert_eq!(c.height, Some(1.2));
        let sc = p.to_scene_config("x");
        assert_eq!(sc.room_size, None, "寸法直接指定なので room_size は使わない");
        assert_eq!(sc.room_w, Some(4.0));
    }
}
