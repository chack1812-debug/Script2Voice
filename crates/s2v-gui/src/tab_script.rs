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
                    tracing::info!("台本を再読込しました: {}", path.display());
                }
                Err(e) => self.load_error = Some(e),
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, jobs: &Jobs) {
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
            return;
        };

        for w in &model.warnings {
            ui.colored_label(
                egui::Color32::from_rgb(200, 130, 0),
                format!("⚠ {}行目: {}", w.line_no, w.message),
            );
        }

        ui.separator();

        let busy = jobs.busy_preview.load(std::sync::atomic::Ordering::SeqCst);
        let mut to_preview: Option<usize> = None;
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            egui::Grid::new("lines").striped(true).num_columns(5).show(ui, |ui| {
                ui.strong("No");
                ui.strong("シーン");
                ui.strong("キャスト");
                ui.strong("台詞");
                ui.strong("");
                ui.end_row();
                for line in &model.lines {
                    let sel = self.selected == Some(line.no);
                    ui.label(line.no.to_string());
                    ui.label(&line.scene_name);
                    ui.label(&line.cast_name);
                    let text: String = line.display_text.chars().take(30).collect();
                    if ui.selectable_label(sel, text).clicked() {
                        self.selected = Some(line.no);
                    }
                    if ui.add_enabled(!busy, egui::Button::new("▶ 試聴")).clicked() {
                        self.selected = Some(line.no);
                        to_preview = Some(line.no);
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(no) = to_preview {
            if let Some(line) = model.lines.iter().find(|l| l.no == no) {
                self.preview_error = None;
                jobs.preview(line.clone());
            }
        }

        ui.separator();

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
        if let Some(dir) = self.last_project_dir.clone() {
            ui.horizontal(|ui| {
                ui.label("✅ 完了");
                if ui.button("📁 出力フォルダを開く").clicked() {
                    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                }
            });
        }
    }
}
