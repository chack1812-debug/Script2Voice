use std::path::PathBuf;

use crate::history::History;
use crate::jobs::Jobs;
use crate::presets::{self, Preset};
use crate::scene_line::LabParams;

pub enum LabSource {
    ScriptLine,
    WavFile,
}

pub struct LabTab {
    pub params: LabParams,
    pub presets: Vec<Preset>,
    pub preset_warning: Option<String>,
    pub selected_preset: usize,
    pub source: LabSource,
    pub source_wav: Option<PathBuf>,
    pub history: History,
    pub error: Option<String>,
}

impl LabTab {
    pub fn new() -> Self {
        // presets.toml は exe と同じフォルダ（config.toml と同じ規約）
        let path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("presets.toml")))
            .unwrap_or_else(|| PathBuf::from("presets.toml"));
        let (presets, preset_warning) = presets::load_presets(&path);
        Self {
            params: LabParams::default(),
            presets,
            preset_warning,
            selected_preset: 0,
            source: LabSource::WavFile,
            source_wav: None,
            history: History::new(50),
            error: None,
        }
    }

    /// 現在の音源入力（処理対象の WAV パス）。
    fn input(&self, preview_raw: &Option<(usize, PathBuf)>) -> Option<PathBuf> {
        match self.source {
            LabSource::ScriptLine => preview_raw.as_ref().map(|(_, p)| p.clone()),
            LabSource::WavFile => self.source_wav.clone(),
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        jobs: &Jobs,
        preview_raw: &Option<(usize, PathBuf)>,
        player: &mut Option<crate::audio_play::Player>,
    ) {
        if let Some(w) = &self.preset_warning {
            ui.colored_label(egui::Color32::from_rgb(200, 130, 0), format!("⚠ {w}"));
        }
        if let Some(e) = &self.error {
            ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
        }

        // --- 音源選択 ---
        ui.horizontal(|ui| {
            ui.label("音源:");
            let has_line = preview_raw.is_some();
            if ui
                .add_enabled(
                    has_line,
                    egui::RadioButton::new(matches!(self.source, LabSource::ScriptLine),
                        match preview_raw {
                            Some((no, _)) => format!("台本の行 {no}（試聴済み）"),
                            None => "台本の行（先にタブ1で試聴）".to_string(),
                        }),
                )
                .clicked()
            {
                self.source = LabSource::ScriptLine;
            }
            if ui
                .radio(matches!(self.source, LabSource::WavFile), "WAVファイル")
                .clicked()
            {
                self.source = LabSource::WavFile;
            }
            if matches!(self.source, LabSource::WavFile) {
                if ui.button("選択...").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("WAV", &["wav"]).pick_file() {
                        self.source_wav = Some(p);
                    }
                }
                if let Some(p) = &self.source_wav {
                    ui.label(p.file_name().and_then(|s| s.to_str()).unwrap_or("?"));
                }
            }
        });

        ui.separator();

        // --- プリセット ---
        ui.horizontal(|ui| {
            ui.label("プリセット:");
            egui::ComboBox::from_id_salt("preset")
                .selected_text(&self.presets[self.selected_preset].name)
                .show_ui(ui, |ui| {
                    for (i, p) in self.presets.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_preset, i, &p.name);
                    }
                });
            if ui.button("適用").clicked() {
                let p = self.presets[self.selected_preset].clone();
                self.params.apply_preset(&p);
            }
        });

        // --- パラメータ ---
        egui::Grid::new("lab_params").num_columns(6).show(ui, |ui| {
            ui.label("部屋 幅[m]");
            ui.add(egui::Slider::new(&mut self.params.room_w, 2.0..=60.0));
            ui.label("奥行[m]");
            ui.add(egui::Slider::new(&mut self.params.room_d, 2.0..=80.0));
            ui.label("高さ[m]");
            ui.add(egui::Slider::new(&mut self.params.room_h, 2.0..=30.0));
            ui.end_row();
            ui.label("聴取 dx[m]");
            ui.add(egui::Slider::new(&mut self.params.listener_dx, -20.0..=20.0));
            ui.label("dy[m]");
            ui.add(egui::Slider::new(&mut self.params.listener_dy, -30.0..=30.0));
            ui.label("高さ z[m]");
            ui.add(egui::Slider::new(&mut self.params.listener_z, 0.2..=5.0));
            ui.end_row();
            ui.label("pan[°]");
            ui.add(egui::Slider::new(&mut self.params.pan, -90.0..=90.0));
            ui.label("距離[m]");
            ui.add(egui::Slider::new(&mut self.params.distance, 0.1..=30.0));
            ui.label("話者高さ[m]");
            ui.add(egui::Slider::new(&mut self.params.height, 0.0..=5.0));
            ui.end_row();
            ui.label("残響倍率");
            ui.add(egui::Slider::new(&mut self.params.reverb_wet, 0.0..=3.0));
            ui.end_row();
        });

        // --- 実行 ---
        let busy = jobs.busy_lab.load(std::sync::atomic::Ordering::SeqCst);
        ui.horizontal(|ui| {
            let input = self.input(preview_raw);
            if ui
                .add_enabled(!busy && input.is_some(), egui::Button::new("▶ 処理して試聴"))
                .clicked()
            {
                self.error = None;
                jobs.lab_process(input.unwrap(), self.params.clone());
            }
            if busy {
                ui.spinner();
            }
        });

        // --- @scene 行 ---
        let line = self.params.scene_line("シーン名");
        ui.horizontal(|ui| {
            ui.code(&line);
            if ui.button("📋 コピー").clicked() {
                ui.output_mut(|o| o.copied_text = line.clone());
            }
        });

        ui.separator();

        // --- 試聴履歴 ---
        ui.horizontal(|ui| {
            ui.strong("試聴履歴");
            let (a, b) = (self.history.sel_a, self.history.sel_b);
            if ui.add_enabled(a.is_some(), egui::Button::new("▶ A")).clicked() {
                if let (Some(id), Some(p)) = (a, player.as_mut()) {
                    if let Some(e) = self.history.get(id) {
                        let _ = p.play(&e.wav);
                    }
                }
            }
            if ui.add_enabled(b.is_some(), egui::Button::new("▶ B")).clicked() {
                if let (Some(id), Some(p)) = (b, player.as_mut()) {
                    if let Some(e) = self.history.get(id) {
                        let _ = p.play(&e.wav);
                    }
                }
            }
            if ui.button("クリア").clicked() {
                self.history.clear();
            }
        });

        let mut recall: Option<LabParams> = None;
        let mut play: Option<PathBuf> = None;
        let mut toggle: Option<usize> = None;
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            egui::Grid::new("history").striped(true).num_columns(5).show(ui, |ui| {
                for e in self.history.entries() {
                    let mark = if self.history.sel_a == Some(e.id) {
                        "A"
                    } else if self.history.sel_b == Some(e.id) {
                        "B"
                    } else {
                        ""
                    };
                    if ui.selectable_label(!mark.is_empty(), format!("#{} {}", e.id, mark)).clicked() {
                        toggle = Some(e.id);
                    }
                    ui.label(format!(
                        "部屋{}x{}x{} wet{} pan{} 距離{}",
                        e.params.room_w, e.params.room_d, e.params.room_h,
                        e.params.reverb_wet, e.params.pan, e.params.distance,
                    ));
                    if ui.button("▶").clicked() {
                        play = Some(e.wav.clone());
                    }
                    if ui.button("呼び戻す").clicked() {
                        recall = Some(e.params.clone());
                    }
                    if ui.button("💾 書き出し").clicked() {
                        if let Some(dest) = rfd::FileDialog::new()
                            .add_filter("WAV", &["wav"])
                            .set_file_name(format!("lab_{:04}.wav", e.id))
                            .save_file()
                        {
                            if let Err(err) = std::fs::copy(&e.wav, &dest) {
                                tracing::warn!("書き出し失敗: {err}");
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(id) = toggle {
            self.history.toggle_select(id);
        }
        if let Some(p) = recall {
            self.params = p;
        }
        if let (Some(path), Some(pl)) = (play, player.as_mut()) {
            let _ = pl.play(&path);
        }
    }
}
