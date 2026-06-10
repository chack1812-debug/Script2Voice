/// Windows 標準の日本語フォントを egui に登録する（無ければ豆腐になるだけで続行）。
pub fn install_japanese_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\meiryo.ttc",
        r"C:\Windows\Fonts\YuGothM.ttc",
        r"C:\Windows\Fonts\msgothic.ttc",
    ];
    let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
        tracing::warn!("日本語フォントが見つかりません（表示が崩れる可能性）");
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("jp".into(), egui::FontData::from_owned(bytes));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("jp".into());
    }
    ctx.set_fonts(fonts);
}
