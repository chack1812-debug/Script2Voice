use std::path::{Path, PathBuf};

use crate::audio_play::Player;

/// 共通再生バーの状態。再生はすべてここを経由させ「いま何が鳴っているか」を一元管理する。
pub struct Transport {
    pub now_playing: Option<String>,
    last_wav: Option<PathBuf>,
}

impl Transport {
    pub fn new() -> Self {
        Self { now_playing: None, last_wav: None }
    }

    /// 再生を開始し、表示名と再再生用パスを記録する。失敗時はエラーメッセージを返す。
    pub fn play(&mut self, player: &mut Option<Player>, path: &Path) -> Result<(), String> {
        let Some(pl) = player.as_mut() else {
            return Err("音声出力デバイスがありません".into());
        };
        pl.play(path).map_err(|e| e.to_string())?;
        self.now_playing = Some(
            path.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string(),
        );
        self.last_wav = Some(path.to_path_buf());
        Ok(())
    }

    pub fn stop(&mut self, player: &mut Option<Player>) {
        if let Some(pl) = player.as_mut() {
            pl.stop();
        }
        self.now_playing = None;
    }

    /// 再生バー本体。a/b は履歴の A/B 選択 WAV(あれば有効化)。
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        player: &mut Option<Player>,
        a: Option<PathBuf>,
        b: Option<PathBuf>,
    ) {
        ui.horizontal(|ui| {
            let can_replay = self.last_wav.is_some() && player.is_some();
            if ui.add_enabled(can_replay, egui::Button::new("▶")).clicked() {
                if let Some(p) = self.last_wav.clone() {
                    let _ = self.play(player, &p);
                }
            }
            if ui.add_enabled(player.is_some(), egui::Button::new("⏹")).clicked() {
                self.stop(player);
            }
            ui.label(match &self.now_playing {
                Some(n) => format!("再生中: {n}"),
                None => "—".to_string(),
            });
            ui.separator();
            if ui.add_enabled(a.is_some(), egui::Button::new("▶ A")).clicked() {
                if let Some(p) = a.clone() {
                    let _ = self.play(player, &p);
                }
            }
            if ui.add_enabled(b.is_some(), egui::Button::new("▶ B")).clicked() {
                if let Some(p) = b.clone() {
                    let _ = self.play(player, &p);
                }
            }
        });
    }
}
