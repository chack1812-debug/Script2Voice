use std::path::PathBuf;

use crate::history::History;
use crate::jobs::Jobs;
use crate::presets::{self, Preset};
use crate::room_view;
use crate::scene_line::LabParams;
use crate::transport::Transport;

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
        transport: &mut Transport,
        player: &mut Option<crate::audio_play::Player>,
        audio_cfg: Option<&s2v_core::AudioConfig>,
    ) {
        if let Some(w) = &self.preset_warning {
            ui.colored_label(egui::Color32::from_rgb(200, 130, 0), format!("⚠ {w}"));
        }
        if let Some(e) = &self.error {
            ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
        }

        // ── 音源選択（上部・全幅）── 既存コードのまま
        ui.horizontal(|ui| {
            ui.label("音源:");
            let has_line = preview_raw.is_some();
            if ui
                .add_enabled(
                    has_line,
                    egui::RadioButton::new(
                        matches!(self.source, LabSource::ScriptLine),
                        match preview_raw {
                            Some((no, _)) => format!("台本の行 {no}（試聴済み）"),
                            None => "台本の行（先にタブ1で試聴）".to_string(),
                        },
                    ),
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

        ui.horizontal_top(|ui| {
            // ── 左: 部屋俯瞰図 ──
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() * 0.46);
                let front = audio_cfg
                    .map(|c| c.early_reflections.front_wall.reflection_coeff)
                    .unwrap_or(0.85);
                room_view::room_view_ui(ui, &mut self.params, front);
                let rt60 = audio_cfg.map(|c| {
                    s2v_audio::acoustics::compute_reverb_params(
                        [self.params.room_w, self.params.room_d, self.params.room_h],
                        &c.early_reflections,
                        c.sound_speed,
                        c.sample_rate,
                    )
                    .rt60
                });
                ui.label(format!(
                    "{}×{}×{} m ／ RT60 {} ／ 聴取者 dx{:+.1} dy{:+.1}（図をドラッグ）",
                    self.params.room_w, self.params.room_d, self.params.room_h,
                    rt60.map_or("—".into(), |v| format!("{v:.2}s")),
                    self.params.listener_dx, self.params.listener_dy,
                ));
            });

            ui.separator();

            // ── 右: パラメータ・実行・履歴 ──
            ui.vertical(|ui| {
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
                        room_view::normalize(&mut self.params);
                    }
                });

                // W/D/H をそれぞれ独立の行で（Slider＋数値直接入力）
                let mut dims_changed = false;
                egui::Grid::new("lab_params").num_columns(3).show(ui, |ui| {
                    let row = |ui: &mut egui::Ui,
                                   label: &str,
                                   v: &mut f64,
                                   range: std::ops::RangeInclusive<f64>,
                                   suffix: &str|
                     -> bool {
                        ui.label(label);
                        let s = ui.add(egui::Slider::new(v, range.clone()).show_value(false));
                        let d = ui.add(
                            egui::DragValue::new(v).range(range).speed(0.1).suffix(suffix),
                        );
                        ui.end_row();
                        s.changed() || d.changed()
                    };
                    dims_changed |= row(ui, "部屋 幅 W", &mut self.params.room_w, 2.0..=60.0, " m");
                    dims_changed |= row(ui, "部屋 奥行 D", &mut self.params.room_d, 2.0..=80.0, " m");
                    dims_changed |= row(ui, "部屋 高さ H", &mut self.params.room_h, 2.0..=30.0, " m");
                    row(ui, "聴取 高さ z", &mut self.params.listener_z, 0.2..=5.0, " m");
                    row(ui, "話者 高さ", &mut self.params.height, 0.0..=5.0, " m");
                    row(ui, "残響倍率", &mut self.params.reverb_wet, 0.0..=3.0, "");
                });
                ui.horizontal(|ui| {
                    ui.label("pan");
                    let p1 = ui.add(
                        egui::DragValue::new(&mut self.params.pan)
                            .range(-180.0..=180.0)
                            .speed(0.5)
                            .suffix(" °"),
                    );
                    ui.label("距離");
                    let p2 = ui.add(
                        egui::DragValue::new(&mut self.params.distance)
                            .range(0.1..=30.0)
                            .speed(0.05)
                            .suffix(" m"),
                    );
                    if p1.changed() || p2.changed() {
                        dims_changed = true;
                    }
                });
                if dims_changed {
                    room_view::normalize(&mut self.params);
                }

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

                let line = self.params.scene_line("シーン名");
                ui.horizontal(|ui| {
                    ui.code(&line);
                    if ui.button("📋 コピー").clicked() {
                        ui.output_mut(|o| o.copied_text = line.clone());
                    }
                });

                ui.separator();

                // ── 試聴履歴（再生は Transport 経由に変更。▶A/▶B は再生バーへ移設済みのため削除）──
                ui.horizontal(|ui| {
                    ui.strong("試聴履歴（クリックで A/B 指定 → 下の再生バーで交互再生）");
                    if ui.button("クリア").clicked() {
                        self.history.clear();
                    }
                });
                let mut recall: Option<crate::scene_line::LabParams> = None;
                let mut play: Option<std::path::PathBuf> = None;
                let mut toggle: Option<usize> = None;
                let mut export_error: Option<String> = None;
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    egui::Grid::new("history").striped(true).num_columns(5).show(ui, |ui| {
                        for e in self.history.entries() {
                            let mark = if self.history.sel_a == Some(e.id) {
                                "A"
                            } else if self.history.sel_b == Some(e.id) {
                                "B"
                            } else {
                                ""
                            };
                            if ui
                                .selectable_label(!mark.is_empty(), format!("#{} {}", e.id, mark))
                                .clicked()
                            {
                                toggle = Some(e.id);
                            }
                            ui.label(format!(
                                "部屋{}x{}x{} wet{} pan{:.0} 距離{:.1}",
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
                                        export_error = Some(format!("書き出し失敗: {err}"));
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
                    room_view::normalize(&mut self.params);
                }
                if let Some(path) = play {
                    let _ = transport.play(player, &path);
                }
                if let Some(e) = export_error {
                    self.error = Some(e);
                }
            });
        });
    }
}
