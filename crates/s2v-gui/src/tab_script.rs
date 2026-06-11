use std::path::PathBuf;

use crate::jobs::Jobs;
use crate::script_model::{self, ScriptModel, WatchedFile};

pub struct ScriptTab {
    pub model: Option<ScriptModel>,
    pub load_error: Option<String>,
    watcher: Option<WatchedFile>,
    pub auto_reload: bool,
    pub selected: Option<usize>,
    /// 直近プレビューの raw（音響ラボの「台本の行」音源）
    pub preview_raw: Option<(usize, PathBuf)>,
    pub preview_error: Option<String>,
    pub run_phase: String,
    pub run_progress: Option<(usize, usize)>,
    pub run_error: Option<String>,
    pub last_project_dir: Option<PathBuf>,
}

impl Default for ScriptTab {
    fn default() -> Self {
        Self {
            model: None,
            load_error: None,
            watcher: None,
            auto_reload: true,
            selected: None,
            preview_raw: None,
            preview_error: None,
            run_phase: String::new(),
            run_progress: None,
            run_error: None,
            last_project_dir: None,
        }
    }
}

/// タブ1から App へ依頼するアクション。
pub enum ScriptAction {
    /// 選択行を音響ラボで調整する（タブ切替＋音源設定。raw 未取得なら自動プレビュー）
    OpenLab { line_no: usize },
}

impl ScriptTab {
    /// 前回開いた台本パスの保存先（exe と同じフォルダの s2v-gui.last）。
    fn last_path_file() -> Option<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("s2v-gui.last")))
    }

    /// 起動時に前回の台本を自動で開く（無ければ何もしない）。
    pub fn restore_last(&mut self) {
        let Some(f) = Self::last_path_file() else { return };
        let Ok(s) = std::fs::read_to_string(&f) else { return };
        let p = PathBuf::from(s.trim());
        if p.exists() {
            self.open(p);
        } else {
            tracing::debug!("前回の台本が見つかりません: {}", p.display());
        }
    }

    pub fn open(&mut self, path: PathBuf) {
        match script_model::load(&path) {
            Ok(m) => {
                if let Some(f) = Self::last_path_file() {
                    let _ = std::fs::write(&f, path.to_string_lossy().as_bytes());
                }
                self.watcher = Some(WatchedFile::new(path));
                self.model = Some(m);
                self.load_error = None;
                self.selected = None;
            }
            Err(e) => self.load_error = Some(e), // 前回モデルは保持
        }
    }

    fn reload_if_changed(&mut self) {
        if !self.auto_reload {
            return;
        }
        let Some(w) = self.watcher.as_mut() else { return };
        if !w.poll() {
            return;
        }
        if let Some(m) = &self.model {
            let path = m.path.clone();
            match script_model::load(&path) {
                Ok(new_model) => {
                    self.model = Some(new_model);
                    self.load_error = None;
                    // 行の増減で番号がずれるため、選択とプレビュー音源は無効化する
                    self.selected = None;
                    self.preview_raw = None;
                    tracing::info!("台本を再読込しました: {}", path.display());
                }
                Err(e) => self.load_error = Some(e),
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, jobs: &Jobs) -> Option<ScriptAction> {
        let mut action = None;
        self.reload_if_changed();

        ui.horizontal(|ui| {
            if ui.button("📂 台本を開く...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("台本テキスト", &["txt"])
                    .pick_file()
                {
                    self.open(path);
                }
            }
            if let Some(m) = &self.model {
                ui.label(m.path.display().to_string());
            }
            ui.checkbox(&mut self.auto_reload, "保存時に自動再読込");
        });

        if let Some(e) = &self.load_error {
            ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
        }
        if let Some(e) = &self.preview_error {
            ui.colored_label(egui::Color32::RED, format!("⚠ 試聴失敗: {e}"));
        }

        let Some(model) = &self.model else {
            ui.label("台本ファイルを開いてください。");
            return None;
        };

        for w in &model.warnings {
            ui.colored_label(
                egui::Color32::from_rgb(200, 130, 0),
                format!("⚠ {}行目: {}", w.line_no, w.message),
            );
        }

        ui.separator();

        let busy = jobs.busy_preview.load(std::sync::atomic::Ordering::SeqCst);
        let panes_h = (ui.available_height() - 84.0).max(160.0); // 下部の実行ストリップ分を確保
        let left_w = ui.available_width() * 0.38;

        ui.horizontal_top(|ui| {
            // ── 左: 行リスト ──
            ui.vertical(|ui| {
                ui.set_width(left_w);
                ui.strong("行リスト");
                egui::ScrollArea::vertical()
                    .id_salt("line_list")
                    .max_height(panes_h)
                    .show(ui, |ui| {
                        for line in &model.lines {
                            let sel = self.selected == Some(line.no);
                            let head: String = line.display_text.chars().take(22).collect();
                            if ui
                                .selectable_label(sel, format!("{:>3} {} {}", line.no, line.cast_name, head))
                                .clicked()
                            {
                                self.selected = Some(line.no);
                            }
                        }
                    });
            });

            ui.separator();

            // ── 右: 選択行の詳細 ──
            ui.vertical(|ui| {
                let Some(line) = self
                    .selected
                    .and_then(|no| model.lines.iter().find(|l| l.no == no))
                    .cloned()
                else {
                    ui.label("← 行を選択してください");
                    return;
                };
                ui.strong(format!("行 {} ／ {}（{}）", line.no, line.cast_name, line.scene_name));
                egui::ScrollArea::vertical()
                    .id_salt("line_detail")
                    .max_height(panes_h * 0.45)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(&line.display_text));
                    });
                let sc = &line.scene_config;
                let dims = match (sc.room_w, sc.room_d, sc.room_h) {
                    (Some(w), Some(d), Some(h)) => format!("{w}×{d}×{h}m"),
                    _ => format!(
                        "room_size={}",
                        sc.room_size.map_or("既定".to_string(), |v| v.to_string())
                    ),
                };
                ui.label(format!(
                    "シーン: {dims} ／ listener z={} reverb_wet={}",
                    sc.listener_z.map_or("既定".to_string(), |v| v.to_string()),
                    sc.reverb_wet.map_or("既定".to_string(), |v| v.to_string()),
                ));
                ui.label(format!(
                    "cast: pan {:+.1}° ／ 距離 {:.2}m ／ 高さ {} ／ 音量 {:.2}",
                    line.cast.pan,
                    line.cast.distance,
                    line.cast.height.map_or("聴取者と同じ".to_string(), |h| format!("{h}m")),
                    line.cast.volume,
                ));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!busy, egui::Button::new("▶ この行を試聴"))
                        .clicked()
                    {
                        self.preview_error = None;
                        jobs.preview(line.clone());
                    }
                    if ui.button("🎛 ラボでこの行を調整 →").clicked() {
                        action = Some(ScriptAction::OpenLab { line_no: line.no });
                    }
                });
            });
        });

        ui.separator();

        // ── 下部: 一括実行ストリップ（全幅）──
        let running = jobs.busy_run.load(std::sync::atomic::Ordering::SeqCst);
        ui.horizontal(|ui| {
            if ui.add_enabled(!running, egui::Button::new("▶ 一括実行")).clicked() {
                self.run_error = None;
                self.run_progress = None;
                self.last_project_dir = None;
                jobs.run_all(model.path.clone());
            }
            if ui.add_enabled(running, egui::Button::new("⏹ キャンセル")).clicked() {
                jobs.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if running {
                ui.label(format!("実行中: {}", self.run_phase));
            }
            if let Some(dir) = self.last_project_dir.clone() {
                ui.label("✅ 完了");
                if ui.button("📁 出力フォルダを開く").clicked() {
                    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                }
            }
        });
        if let Some((done, total)) = self.run_progress {
            ui.add(
                egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                    .text(format!("{done}/{total} 行")),
            );
        }
        if let Some(e) = &self.run_error {
            ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
        }
        action
    }
}
