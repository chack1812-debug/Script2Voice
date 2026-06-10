use crate::logbuf::LogBuffer;

#[derive(PartialEq, Clone, Copy)]
pub enum Tab {
    Script,
    Lab,
}

pub struct App {
    pub tab: Tab,
    pub log: LogBuffer,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, log: LogBuffer) -> Self {
        Self { tab: Tab::Script, log }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Script => {
                ui.label("(台本タブ: Task 10 で実装)");
            }
            Tab::Lab => {
                ui.label("(音響ラボ: Task 11 で実装)");
            }
        });
        // バックグラウンド完了の取りこぼし防止に定期再描画
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}
