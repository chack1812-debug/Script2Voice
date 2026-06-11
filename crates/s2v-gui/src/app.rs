use crate::audio_play::Player;
use crate::jobs::{JobMsg, Jobs};
use crate::logbuf::LogBuffer;
use crate::tab_lab::LabTab;
use crate::tab_script::ScriptTab;
use crate::transport::Transport;

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
    pub lab: LabTab,
    pub transport: Transport,
    pub audio_cfg: Option<s2v_core::AudioConfig>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, log: LogBuffer) -> Self {
        // config.toml は CLI と同じ規約（exe と同じフォルダ）で解決
        let exe = std::env::current_exe().ok();
        let config_path = script2voice::resolve_config_path(None, exe.as_deref());
        let mut audio_cfg: Option<s2v_core::AudioConfig> = None;
        let (jobs, jobs_error) = match s2v_core::Config::from_file(&config_path) {
            Ok(cfg) => {
                audio_cfg = Some(cfg.audio.clone());
                match Jobs::new(cfg) {
                    Ok(j) => (Some(j), None),
                    Err(e) => (None, Some(format!("初期化失敗: {e}"))),
                }
            }
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
            lab: LabTab::new(),
            transport: Transport::new(),
            audio_cfg,
        }
    }

    fn pump_messages(&mut self) {
        let Some(jobs) = &self.jobs else { return };
        let msgs: Vec<JobMsg> = jobs.rx.try_iter().collect();
        for msg in msgs {
            match msg {
                JobMsg::PreviewReady { line_no, wav, raw } => {
                    self.script.preview_raw = Some((line_no, raw));
                    if let Err(e) = self.transport.play(&mut self.player, &wav) {
                        self.script.preview_error = Some(e);
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
                JobMsg::LabReady { wav, params } => {
                    self.lab.error = None;
                    self.lab.history.push(params, wav.clone());
                    let _ = self.transport.play(&mut self.player, &wav);
                }
                JobMsg::LabFailed { error } => self.lab.error = Some(error),
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
        egui::TopBottomPanel::bottom("transport").show(ctx, |ui| {
            let (a, b) = {
                let h = &self.lab.history;
                (
                    h.sel_a.and_then(|id| h.get(id)).map(|e| e.wav.clone()),
                    h.sel_b.and_then(|id| h.get(id)).map(|e| e.wav.clone()),
                )
            };
            self.transport.ui(ui, &mut self.player, a, b);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(e) = &self.jobs_error {
                ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
                return;
            }
            match self.tab {
                Tab::Script => {
                    if let Some(jobs) = &self.jobs {
                        if let Some(action) = self.script.ui(ui, jobs) {
                            match action {
                                crate::tab_script::ScriptAction::OpenLab { line_no } => {
                                    self.tab = Tab::Lab;
                                    self.lab.source = crate::tab_lab::LabSource::ScriptLine;
                                    // raw 未取得（または別の行）なら自動でプレビュー合成
                                    let has_raw = self
                                        .script
                                        .preview_raw
                                        .as_ref()
                                        .map(|(no, _)| *no == line_no)
                                        .unwrap_or(false);
                                    if !has_raw
                                        && !jobs.busy_preview.load(std::sync::atomic::Ordering::SeqCst)
                                    {
                                        if let Some(line) = self
                                            .script
                                            .model
                                            .as_ref()
                                            .and_then(|m| m.lines.iter().find(|l| l.no == line_no))
                                            .cloned()
                                        {
                                            self.script.preview_error = None;
                                            jobs.preview(line);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Tab::Lab => {
                    if let Some(jobs) = &self.jobs {
                        let preview_raw = self.script.preview_raw.clone();
                        let cfg = self.audio_cfg.clone();
                        self.lab.ui(
                            ui,
                            jobs,
                            &preview_raw,
                            &mut self.transport,
                            &mut self.player,
                            cfg.as_ref(),
                        );
                    }
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
