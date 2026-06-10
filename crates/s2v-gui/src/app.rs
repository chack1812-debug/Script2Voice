use crate::audio_play::Player;
use crate::jobs::{JobMsg, Jobs};
use crate::logbuf::LogBuffer;
use crate::tab_script::ScriptTab;

#[derive(PartialEq, Clone, Copy)]
pub enum Tab {
    Script,
    Lab,
}

pub struct App {
    pub tab: Tab,
    pub log: LogBuffer,
    pub jobs: Option<Jobs>,
    pub jobs_error: Option<String>,
    pub player: Option<Player>,
    pub script: ScriptTab,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, log: LogBuffer) -> Self {
        // config.toml は CLI と同じ規約（exe と同じフォルダ）で解決
        let exe = std::env::current_exe().ok();
        let config_path = script2voice::resolve_config_path(None, exe.as_deref());
        let (jobs, jobs_error) = match s2v_core::Config::from_file(&config_path) {
            Ok(cfg) => match Jobs::new(cfg) {
                Ok(j) => (Some(j), None),
                Err(e) => (None, Some(format!("初期化失敗: {e}"))),
            },
            Err(e) => (None, Some(format!("config.toml を読めません ({}): {e}", config_path.display()))),
        };
        let mut script = ScriptTab::default();
        script.restore_last(); // 前回の台本パスを復元
        Self {
            tab: Tab::Script,
            log,
            jobs,
            jobs_error,
            player: Player::new(),
            script,
        }
    }

    fn pump_messages(&mut self) {
        let Some(jobs) = &self.jobs else { return };
        let msgs: Vec<JobMsg> = jobs.rx.try_iter().collect();
        for msg in msgs {
            match msg {
                JobMsg::PreviewReady { line_no, wav, raw } => {
                    self.script.preview_raw = Some((line_no, raw));
                    if let Some(p) = &mut self.player {
                        if let Err(e) = p.play(&wav) {
                            self.script.preview_error = Some(e.to_string());
                        }
                    }
                }
                JobMsg::PreviewFailed { error, .. } => {
                    self.script.preview_error = Some(error);
                }
                JobMsg::RunPhase(p) => self.script.run_phase = p,
                JobMsg::RunProgress { done, total } => {
                    self.script.run_progress = Some((done, total));
                }
                JobMsg::RunFinished { result } => match result {
                    Ok(dir) => self.script.last_project_dir = Some(dir),
                    Err(e) => self.script.run_error = Some(e),
                },
                JobMsg::LabReady { .. } | JobMsg::LabFailed { .. } => {
                    // Task 11 で音響ラボに配線
                }
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_messages();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Script, "📜 台本");
                ui.selectable_value(&mut self.tab, Tab::Lab, "🎛 音響ラボ");
            });
        });
        egui::TopBottomPanel::bottom("log")
            .resizable(true)
            .default_height(120.0)
            .show(ctx, |ui| {
                ui.collapsing("実行ログ", |ui| {
                    egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                        for line in self.log.lines() {
                            ui.monospace(line);
                        }
                    });
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(e) = &self.jobs_error {
                ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
                return;
            }
            match self.tab {
                Tab::Script => {
                    if let Some(jobs) = &self.jobs {
                        self.script.ui(ui, jobs);
                    }
                }
                Tab::Lab => {
                    ui.label("(音響ラボ: Task 11 で実装)");
                }
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(jobs) = &self.jobs {
            jobs.shutdown(); // 自動起動したエンジンを停止
        }
    }
}
