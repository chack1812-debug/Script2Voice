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
    /// 試聴中の行番号（タブ非依存で進行状況を表示するため）
    pub preview_pending: Option<usize>,
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
            preview_pending: None,
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

    /// 下部の一括実行ストリップ（区切り線＋ボタン行＋進捗バー＋エラー行）に必要な高さ。
    /// この分を 2ペインの高さから差し引いて取り置く。
    fn run_strip_height(&self, ui: &egui::Ui) -> f32 {
        let spacing = ui.spacing().item_spacing.y;
        // ボタン・進捗バーの行高（既定の interact_size と実フォントの大きい方）
        let row_h = ui.spacing().interact_size.y.max(
            ui.text_style_height(&egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.y,
        );
        const SEPARATOR_H: f32 = 6.0; // egui::Separator の既定 spacing

        let mut h = spacing + SEPARATOR_H + spacing + row_h; // 区切り線 + ボタン行
        if self.run_progress.is_some() {
            h += spacing + row_h; // 進捗バー
        }
        if self.run_error.is_some() {
            h += spacing + row_h * 2.0; // エラー行（折返し 2 行ぶんを目安に確保）
        }
        h
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
        let strip_h = self.run_strip_height(ui);
        let panes_h = (ui.available_height() - strip_h).max(160.0); // 下部の実行ストリップ分を確保
        let left_w = ui.available_width() * 0.38;

        // 2ペインの高さは allocate_ui で明示的に切る。切らないと:
        //   * `horizontal_top` は残りの高さ全部を子 Ui の max_rect として要求し、
        //   * その中の `ui.separator()` は横レイアウトでは縦線となり、
        //     長さに「その Ui の利用可能な高さ全部」を使う（egui の Separator 仕様）
        // ため、2ペイン行だけで CentralPanel を使い切り、後続の一括実行ストリップが
        // パネルの下端より外へ押し出されて set_clip_rect により不可視になる。
        ui.allocate_ui(egui::vec2(ui.available_width(), panes_h), |ui| {
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
                            self.preview_pending = Some(line.no);
                            jobs.preview(line.clone());
                        }
                        if ui.button("🎛 ラボでこの行を調整 →").clicked() {
                            action = Some(ScriptAction::OpenLab { line_no: line.no });
                        }
                    });
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 行リストがスクロールを要する程度の行数を持つ台本。
    fn sample_script() -> String {
        let mut s = String::from("@scene 一\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\n");
        for i in 1..=40 {
            s.push_str(&format!("A:これは{i}行目の台詞です。長さの目安として少し長めに書いておきます。\n"));
        }
        s
    }

    fn test_jobs() -> Jobs {
        let cfg: s2v_core::Config = toml::from_str(include_str!("../../../config.toml")).unwrap();
        Jobs::new(cfg).unwrap()
    }

    /// 台本タブを数フレーム描画し、(可視領域=クリップ矩形, 実際に使われた矩形) を返す。
    ///
    /// `max_rect` はコンテンツがはみ出すと一緒に広がってしまうため判定に使えない。
    /// CentralPanel は `set_clip_rect` で描画をパネル内に切り詰めるので、
    /// 「見えているか」の基準はクリップ矩形になる。
    fn layout_once(size: egui::Vec2, selected: Option<usize>, running: bool) -> (egui::Rect, egui::Rect) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("台本.txt");
        std::fs::write(&path, sample_script()).unwrap();

        let jobs = test_jobs();
        let mut tab = ScriptTab {
            model: Some(crate::script_model::load(&path).unwrap()),
            selected,
            // 進捗バー・エラー行が増えるぶんもストリップ用に取り置けているか
            run_progress: running.then_some((3, 40)),
            run_error: running.then(|| "合成失敗: エンジンに接続できません".to_string()),
            ..Default::default()
        };

        let ctx = egui::Context::default();
        let mut rects = None;
        // ScrollArea 等はフレームをまたいで状態を持つため、数フレーム回してから測る。
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    tab.ui(ui, &jobs);
                    rects = Some((ui.clip_rect(), ui.min_rect()));
                });
            });
        }
        rects.unwrap()
    }

    /// 一括実行ストリップ（▶ 一括実行／⏹ キャンセル／進捗バー）が CentralPanel の
    /// 下端からはみ出していないこと。
    ///
    /// 回帰の内容: 2ペインを `ui.horizontal_top` で描き、その中で `ui.separator()` を
    /// 呼ぶと、egui の Separator は縦線として「そのUiの利用可能な高さ全部」を占有する。
    /// 高さを制限せずに描くと 2ペイン行だけで CentralPanel を使い切り、後続の
    /// ストリップがパネル外へ押し出されて `set_clip_rect` により不可視になる。
    #[test]
    fn bulk_run_strip_fits_inside_central_panel() {
        for size in [egui::vec2(1100.0, 760.0), egui::vec2(1600.0, 1000.0)] {
            for (selected, running) in
                [(None, false), (Some(1usize), false), (Some(1usize), true)]
            {
                let (clip_rect, min_rect) = layout_once(size, selected, running);
                assert!(
                    min_rect.bottom() <= clip_rect.bottom(),
                    "一括実行ストリップが可視領域からはみ出している \
                     (size={size:?}, selected={selected:?}, running={running}): \
                     使用領域 bottom={} > 可視領域 bottom={}",
                    min_rect.bottom(),
                    clip_rect.bottom(),
                );
            }
        }
    }
}
