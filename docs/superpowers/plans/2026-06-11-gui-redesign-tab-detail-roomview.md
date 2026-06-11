# GUI リデザイン（案A: 2ペイン＋部屋俯瞰図＋W/D/H個別＋再生バー）実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** タブ1をマスター/ディテール2ペイン化、タブ2に聴取者・話者をドラッグできる部屋2D俯瞰図を追加、部屋寸法 W/D/H を個別の Slider＋数値入力にし、⏹停止付き再生バーを両タブ共通化する。

**Architecture:** UI 層（crates/s2v-gui）のみの再構成。座標変換・クランプは純関数として `room_view.rs` に分離し TDD。再生状態は新 `transport.rs` に集約。DSP・合成・ライブラリ（s2v-core/audio/engines/script2voice lib）・CLI は不変。

**Tech Stack:** 既存どおり egui 0.29（painter 描画＋ drag interact を追加使用）。新規依存なし。

**Spec:** `docs/superpowers/specs/2026-06-11-gui-redesign-tab-detail-roomview-design.md`（Beads: s2v-iu2）

**前提（現状コード）:** 初版 GUI 実装済み（`9cce27c..4bfd3d4`）。
- `LabParams { room_w, room_d, room_h, listener_dx, listener_dy, listener_z, reverb_wet, pan, distance, height }`（すべて f64、`scene_line.rs`）
- `ScriptTab.preview_raw: Option<(usize, PathBuf)>`、`PreviewLine { no, scene_name, cast_name, display_text, text, cast, scene_config }`
- `LabTab { params, presets, preset_warning, selected_preset, source: LabSource, source_wav, history, error }`
- `Jobs::{preview, run_all, lab_process}`、`JobMsg::*`、`Player::{new, play, stop}`
- `s2v_audio::acoustics::compute_reverb_params(dims, &EarlyConfig, sound_speed, sample_rate) -> ReverbParams{ rt60, .. }`（pub。dims の実型は acoustics.rs を確認すること — RoomGeometry.dims と同型、[f64;3] 想定）

---

### Task 1: room_view.rs — 座標変換・クランプの純ロジック（TDD）

**Files:**
- Create: `crates/s2v-gui/src/room_view.rs`
- Modify: `crates/s2v-gui/src/main.rs`（`mod room_view;` 追加）

- [ ] **Step 1: 失敗するテストを書く**（room_view.rs に型・関数シグネチャとテストのみ）

```rust
use crate::scene_line::LabParams;

/// 話者と聴取者の距離の下限[m]（0除算・密着防止）
pub const MIN_DISTANCE: f64 = 0.1;
/// 壁からの最小マージン[m]
pub const WALL_MARGIN: f64 = 0.05;

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> LabParams {
        let mut p = LabParams::default(); // 部屋 4x5x3, listener 中央, pan0, dist1
        p.room_w = 10.0;
        p.room_d = 20.0;
        p
    }

    #[test]
    fn listener_and_speaker_positions_follow_params() {
        let mut prm = p();
        prm.listener_dx = 1.0;
        prm.listener_dy = -2.0;
        prm.pan = 90.0; // 真右
        prm.distance = 3.0;
        let (lx, ly) = listener_pos(&prm);
        assert!((lx - 6.0).abs() < 1e-9 && (ly - 8.0).abs() < 1e-9);
        let (sx, sy) = speaker_pos(&prm);
        assert!((sx - 9.0).abs() < 1e-9, "pan+90 は +x 方向: {sx}");
        assert!((sy - 8.0).abs() < 1e-6);
    }

    #[test]
    fn drag_speaker_recomputes_pan_distance_including_behind() {
        let mut prm = p(); // listener (5,10)
        drag_speaker_to(&mut prm, 5.0, 7.0); // 真後ろ 3m
        assert!((prm.distance - 3.0).abs() < 1e-9);
        assert!((prm.pan.abs() - 180.0).abs() < 1e-6, "後方は ±180°: {}", prm.pan);
        drag_speaker_to(&mut prm, 2.0, 10.0); // 真左 3m
        assert!((prm.pan + 90.0).abs() < 1e-6, "左は -90°: {}", prm.pan);
    }

    #[test]
    fn drag_clamps_into_room_and_enforces_min_distance() {
        let mut prm = p();
        drag_speaker_to(&mut prm, 99.0, -99.0); // 部屋外
        let (sx, sy) = speaker_pos(&prm);
        assert!(sx <= prm.room_w - WALL_MARGIN + 1e-9 && sy >= WALL_MARGIN - 1e-9);
        drag_speaker_to(&mut prm, 5.0, 10.0); // 聴取者と同座標
        assert!(prm.distance >= MIN_DISTANCE);
    }

    #[test]
    fn drag_listener_updates_offsets_and_keeps_speaker_inside() {
        let mut prm = p();
        prm.pan = 0.0;
        prm.distance = 5.0; // 話者は前方 5m
        drag_listener_to(&mut prm, 5.0, 18.0); // 前壁近くへ → 話者がはみ出すはず
        assert!((prm.listener_dy - 8.0).abs() < 1e-9);
        let (sx, sy) = speaker_pos(&prm);
        assert!(sy <= prm.room_d - WALL_MARGIN + 1e-9, "話者は再クランプ: {sy}");
        assert!(prm.distance < 5.0, "距離が縮む");
        let _ = sx;
    }

    #[test]
    fn normalize_reclamps_after_room_shrink() {
        let mut prm = p();
        prm.listener_dx = 4.0; // (9,10)
        prm.room_w = 6.0;      // 幅縮小 → x=9 は外
        normalize(&mut prm);
        let (lx, _) = listener_pos(&prm);
        assert!(lx <= 6.0 - WALL_MARGIN + 1e-9);
    }

    #[test]
    fn view_map_roundtrips_room_coords() {
        let avail = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 300.0));
        let vm = ViewMap::new(avail, 10.0, 20.0);
        let s = vm.to_screen(2.5, 15.0);
        let (x, y) = vm.to_room(s);
        assert!((x - 2.5).abs() < 1e-3 && (y - 15.0).abs() < 1e-3);
        // 前方(+y)が画面上方向（screen y は小さく）
        let front = vm.to_screen(5.0, 19.0);
        let back = vm.to_screen(5.0, 1.0);
        assert!(front.y < back.y);
    }
}
```

- [ ] **Step 2: 失敗確認**

`main.rs` に `mod room_view;` 追加後:
Run: `cargo test -p s2v-gui room_view`
Expected: コンパイルエラー（関数未定義）

- [ ] **Step 3: 実装**（テストの上に追加）

```rust
/// 聴取者の部屋座標（x∈[0,W], y∈[0,D]。中央 + オフセット）。
pub fn listener_pos(p: &LabParams) -> (f64, f64) {
    (p.room_w / 2.0 + p.listener_dx, p.room_d / 2.0 + p.listener_dy)
}

/// 話者の部屋座標（聴取者基準の pan/distance から。pan 0°=正面(+y)、+が右(+x)）。
pub fn speaker_pos(p: &LabParams) -> (f64, f64) {
    let (lx, ly) = listener_pos(p);
    let r = p.pan.to_radians();
    (lx + p.distance * r.sin(), ly + p.distance * r.cos())
}

fn clamp_to_room(x: f64, y: f64, w: f64, d: f64) -> (f64, f64) {
    (
        x.clamp(WALL_MARGIN, (w - WALL_MARGIN).max(WALL_MARGIN)),
        y.clamp(WALL_MARGIN, (d - WALL_MARGIN).max(WALL_MARGIN)),
    )
}

fn set_pan_distance_from(p: &mut LabParams, sx: f64, sy: f64) {
    let (lx, ly) = listener_pos(p);
    let (vx, vy) = (sx - lx, sy - ly);
    p.distance = (vx * vx + vy * vy).sqrt().max(MIN_DISTANCE);
    p.pan = vx.atan2(vy).to_degrees();
}

/// 話者を図上の部屋座標 (x,y) へドラッグ: 部屋内にクランプし pan/distance を逆算。
pub fn drag_speaker_to(p: &mut LabParams, x: f64, y: f64) {
    let (x, y) = clamp_to_room(x, y, p.room_w, p.room_d);
    set_pan_distance_from(p, x, y);
}

/// 聴取者を図上の部屋座標 (x,y) へドラッグ: listener_dx/dy を更新し、話者を整合させる。
pub fn drag_listener_to(p: &mut LabParams, x: f64, y: f64) {
    let (x, y) = clamp_to_room(x, y, p.room_w, p.room_d);
    p.listener_dx = x - p.room_w / 2.0;
    p.listener_dy = y - p.room_d / 2.0;
    normalize(p);
}

/// W/D 変更・プリセット適用・聴取者移動の後に呼ぶ:
/// 聴取者を部屋内へ、話者がはみ出すなら部屋内へクランプして pan/distance を再計算する。
pub fn normalize(p: &mut LabParams) {
    let (lx, ly) = listener_pos(p);
    let (clx, cly) = clamp_to_room(lx, ly, p.room_w, p.room_d);
    p.listener_dx = clx - p.room_w / 2.0;
    p.listener_dy = cly - p.room_d / 2.0;
    let (sx, sy) = speaker_pos(p);
    let (csx, csy) = clamp_to_room(sx, sy, p.room_w, p.room_d);
    if (csx - sx).abs() > 1e-9 || (csy - sy).abs() > 1e-9 {
        set_pan_distance_from(p, csx, csy);
    }
}

/// 部屋座標 ⇔ 画面座標の等比マッピング（部屋の縦横比保持・領域中央配置・+y が画面上）。
pub struct ViewMap {
    room_rect: egui::Rect,
    scale: f32,
}

impl ViewMap {
    pub fn new(avail: egui::Rect, room_w: f64, room_d: f64) -> Self {
        let scale = (avail.width() / room_w as f32).min(avail.height() / room_d as f32);
        let size = egui::vec2(room_w as f32 * scale, room_d as f32 * scale);
        let room_rect = egui::Rect::from_center_size(avail.center(), size);
        Self { room_rect, scale }
    }

    /// 描画する部屋矩形（画面座標）。
    pub fn rect(&self) -> egui::Rect {
        self.room_rect
    }

    pub fn to_screen(&self, x: f64, y: f64) -> egui::Pos2 {
        egui::pos2(
            self.room_rect.left() + x as f32 * self.scale,
            self.room_rect.bottom() - y as f32 * self.scale,
        )
    }

    pub fn to_room(&self, p: egui::Pos2) -> (f64, f64) {
        (
            ((p.x - self.room_rect.left()) / self.scale) as f64,
            ((self.room_rect.bottom() - p.y) / self.scale) as f64,
        )
    }
}
```

- [ ] **Step 4: テスト確認** `cargo test -p s2v-gui room_view` → PASS（6件）。dead_code 警告は Task 4 で解消。

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-gui/src/room_view.rs crates/s2v-gui/src/main.rs
git commit -m "feat(gui): room-view coordinate/clamp logic with TDD (drag mapping, normalize)"
```

---

### Task 2: 部屋俯瞰図の描画＋ドラッグ UI（room_view.rs に追加）

**Files:**
- Modify: `crates/s2v-gui/src/room_view.rs`

- [ ] **Step 1: 描画関数を追加**（ファイル末尾の tests の前に）

```rust
/// 部屋の俯瞰図を描き、聴取者👤・話者🔊のドラッグを処理する。
/// 値が変化したら true（呼び出し側で RT60 等を再計算）。
pub fn room_view_ui(ui: &mut egui::Ui, p: &mut LabParams, front_wall_coeff: f64) -> bool {
    let mut changed = false;
    let avail = ui.available_size();
    let size = egui::vec2(avail.x.max(220.0), (avail.y - 4.0).clamp(220.0, 420.0));
    let (area, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let vm = ViewMap::new(area.shrink(14.0), p.room_w, p.room_d);
    let painter = ui.painter_at(area);

    // 部屋
    painter.rect(
        vm.rect(),
        3.0,
        egui::Color32::from_rgb(253, 246, 236),
        egui::Stroke::new(1.5, egui::Color32::from_rgb(201, 168, 106)),
    );
    // 前壁（上辺）: 反射率が高いほど濃く太く
    let a = (60.0 + 180.0 * front_wall_coeff.clamp(0.0, 1.0)) as u8;
    painter.line_segment(
        [vm.rect().left_top(), vm.rect().right_top()],
        egui::Stroke::new(5.0, egui::Color32::from_rgba_unmultiplied(138, 109, 59, a)),
    );
    painter.text(
        vm.rect().center_top() + egui::vec2(0.0, 8.0),
        egui::Align2::CENTER_CENTER,
        "前壁",
        egui::FontId::proportional(10.0),
        egui::Color32::from_rgb(138, 109, 59),
    );

    let lpos = vm.to_screen(listener_pos(p).0, listener_pos(p).1);
    let spos = vm.to_screen(speaker_pos(p).0, speaker_pos(p).1);

    // 経路（破線）
    painter.add(egui::Shape::dashed_line(
        &[lpos, spos],
        egui::Stroke::new(1.5, egui::Color32::from_rgb(224, 138, 43)),
        6.0,
        4.0,
    ));

    // ドラッグ可能な2点（👤聴取者 / 🔊話者）
    let drag_point = |ui: &mut egui::Ui, pos: egui::Pos2, id: &str, icon: &str| -> Option<egui::Pos2> {
        let hit = egui::Rect::from_center_size(pos, egui::vec2(26.0, 26.0));
        let resp = ui.interact(hit, egui::Id::new(id), egui::Sense::drag());
        ui.painter_at(area).text(
            pos,
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(if resp.hovered() || resp.dragged() { 22.0 } else { 18.0 }),
            egui::Color32::BLACK,
        );
        if resp.dragged() {
            resp.interact_pointer_pos()
        } else {
            None
        }
    };

    if let Some(np) = drag_point(ui, spos, "room_speaker", "🔊") {
        let (x, y) = vm.to_room(np);
        drag_speaker_to(p, x, y);
        changed = true;
    }
    if let Some(np) = drag_point(ui, lpos, "room_listener", "👤") {
        let (x, y) = vm.to_room(np);
        drag_listener_to(p, x, y);
        changed = true;
    }
    changed
}
```

- [ ] **Step 2: ビルド確認**

Run: `cargo build -p s2v-gui` → 成功（room_view_ui は未配線のため dead_code 警告のみ。egui 0.29 で `painter.rect` の引数が合わない場合は `rect_filled`＋`rect_stroke` の2呼び出しに分けるなど**意味を変えない最小修正**で通し、報告すること）
Run: `cargo test -p s2v-gui` → 全 PASS

- [ ] **Step 3: コミット**

```bash
git add crates/s2v-gui/src/room_view.rs
git commit -m "feat(gui): room top-view painter with draggable listener/speaker"
```

---

### Task 3: transport.rs（共通再生バー）＋ App 配線

**Files:**
- Create: `crates/s2v-gui/src/transport.rs`
- Modify: `crates/s2v-gui/src/app.rs`
- Modify: `crates/s2v-gui/src/main.rs`（`mod transport;` 追加）

- [ ] **Step 1: transport.rs**

```rust
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

    /// 再生バー本体。a/b は履歴の A/B 選択 WAV（あれば有効化）。
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
```

- [ ] **Step 2: app.rs 配線**

1. `use crate::transport::Transport;` を追加し、`App` にフィールド `pub transport: Transport,`（`new()` で `Transport::new()`）。
2. `App::new` の config 読込部で **AudioConfig のクローンを保持**（Task 5 のラボで RT60 表示に使う）:

```rust
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
```

`App` にフィールド `pub audio_cfg: Option<s2v_core::AudioConfig>,` を追加して格納。
3. `pump_messages` の再生を Transport 経由に置換:
   - `PreviewReady`: `if let Err(e) = self.transport.play(&mut self.player, &wav) { self.script.preview_error = Some(e); }`
   - `LabReady`: `let _ = self.transport.play(&mut self.player, &wav);`
4. `update()` のパネル構成: 既存の `TopBottomPanel::bottom("log")` の**前に**もう1つ bottom パネルを追加（egui は後に追加した bottom パネルほど上に積まれるため、**先に log、次に transport** の順で宣言すると transport が log の上になる。逆なら入れ替えること — 実機で目視確認）:

```rust
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
```

5. この時点では tab_lab 内の直接再生（履歴の▶等）はそのまま（Task 5 で Transport 経由に置換）。

- [ ] **Step 3: ビルド・テスト・起動確認**

Run: `cargo test -p s2v-gui` → 全 PASS。`cargo run -p s2v-gui` で再生バーが両タブ下部に出る（▶/⏹/—/▶A/▶B）ことを目視（数秒で kill 可）。

- [ ] **Step 4: コミット**

```bash
git add crates/s2v-gui/src/transport.rs crates/s2v-gui/src/app.rs crates/s2v-gui/src/main.rs
git commit -m "feat(gui): shared transport bar with stop button and now-playing state"
```

---

### Task 4: タブ1の2ペイン化（行リスト｜詳細）＋🎛導線

**Files:**
- Modify: `crates/s2v-gui/src/tab_script.rs`
- Modify: `crates/s2v-gui/src/app.rs`

- [ ] **Step 1: tab_script.rs の `ui()` を全面置換し、アクション enum を追加**

ファイル先頭の use はそのまま。`ScriptTab` 構造体・`Default`・`last_path_file`/`restore_last`/`open`/`reload_if_changed` は変更しない。以下を追加:

```rust
/// タブ1から App へ依頼するアクション。
pub enum ScriptAction {
    /// 選択行を音響ラボで調整する（タブ切替＋音源設定。raw 未取得なら自動プレビュー）
    OpenLab { line_no: usize },
}
```

`ui()` を以下に置換（シグネチャ変更: 戻り値 `Option<ScriptAction>`）:

```rust
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
                let Some(line) = self.selected.and_then(|no| model.lines.iter().find(|l| l.no == no))
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
                    _ => format!("room_size={}", sc.room_size.map_or("既定".into(), |v| v.to_string())),
                };
                ui.label(format!(
                    "シーン: {dims} ／ listener z={} reverb_wet={}",
                    sc.listener_z.map_or("既定".into(), |v| v.to_string()),
                    sc.reverb_wet.map_or("既定".into(), |v| v.to_string()),
                ));
                ui.label(format!(
                    "cast: pan {:+.1}° ／ 距離 {:.2}m ／ 高さ {} ／ 音量 {:.2}",
                    line.cast.pan,
                    line.cast.distance,
                    line.cast.height.map_or("聴取者と同じ".into(), |h| format!("{h}m")),
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
```

注: 借用の都合で `action` への代入がクロージャ越しになる。コンパイルエラーになる場合は
`let mut action = None;` をクロージャ外に置いたまま、行検索を `model.lines.iter().find(...).cloned()` にする等
**意味を変えない最小修正**で通し、全て報告すること。

- [ ] **Step 2: app.rs で ScriptAction を処理**

`Tab::Script` アームを置換:

```rust
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
                                        {
                                            jobs.preview(line.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
```

- [ ] **Step 3: ビルド・テスト・起動確認**

Run: `cargo test -p s2v-gui` → 全 PASS。`cargo run -p s2v-gui` で2ペイン表示・行選択→詳細表示・🎛でタブ2へ切替わることを目視（エンジン起動を伴う▶試聴は押さない）。

- [ ] **Step 4: コミット**

```bash
git add crates/s2v-gui/src/tab_script.rs crates/s2v-gui/src/app.rs
git commit -m "feat(gui): master-detail script tab with full-text pane and open-in-lab action"
```

---

### Task 5: タブ2の再構成（俯瞰図＋W/D/H個別＋DragValue＋Transport経由再生）

**Files:**
- Modify: `crates/s2v-gui/src/tab_lab.rs`
- Modify: `crates/s2v-gui/src/app.rs`（lab.ui の呼び出しシグネチャ変更）

- [ ] **Step 1: tab_lab.rs の `ui()` を全面置換**

構造体・`new()`・`input()` は変更しない。use に `use crate::room_view;` と `use crate::transport::Transport;` を追加。`ui()` のシグネチャを変更:

```rust
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        jobs: &Jobs,
        preview_raw: &Option<(usize, std::path::PathBuf)>,
        transport: &mut Transport,
        player: &mut Option<crate::audio_play::Player>,
        audio_cfg: Option<&s2v_core::AudioConfig>,
    ) {
```

本体（置換後の全体構成）:

```rust
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
                    let mut row = |ui: &mut egui::Ui,
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
```

- [ ] **Step 2: app.rs の呼び出しを更新**

`Tab::Lab` アームを置換:

```rust
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
```

借用衝突（`self.transport` と `self.lab` 等）が起きる場合は、`let mut transport = std::mem::replace(&mut self.transport, Transport::new()); ... self.transport = transport;` の一時取り出しで**意味を変えずに**回避し、報告すること。

- [ ] **Step 3: compute_reverb_params の実シグネチャ確認**

`crates/s2v-audio/src/acoustics.rs` を読み、dims 引数の型（`[f64; 3]` 想定）・戻り値 `ReverbParams.rt60` を確認。違う場合は呼び出し側を実物に合わせ、変更点を報告。

- [ ] **Step 4: ビルド・全テスト・起動確認**

Run: `cargo test --workspace` → 全 PASS（room_view 含む）
Run: `cargo build -p s2v-gui 2>&1` → dead_code 警告ゼロ（room_view_ui 配線済み）。残る場合は報告。
Run: `cargo run -p s2v-gui` → タブ2で俯瞰図が表示され、👤🔊のドラッグで pan/距離表示が変わり、W/D/H の各行が独立に動き、DragValue で数値直接入力できることを目視。

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-gui/src/tab_lab.rs crates/s2v-gui/src/app.rs
git commit -m "feat(gui): lab tab with draggable room top-view, per-axis WDH controls, transport playback"
```

---

### Task 6: 仕上げ（マニュアル更新・最終確認）

**Files:**
- Modify: `docs/manual.html`（3.4 節の画面説明を改訂）

- [ ] **Step 1: manual.html の 3.4 節を改訂**

`<h3 id="gui">3.4 GUI（ランチャー＋音響ラボ）</h3>` の `<ul>` を以下に置換:

```html
<ul>
  <li><strong>台本タブ</strong>: 左の行リストで行を選ぶと、右に台詞全文・シーン・キャスト設定が表示されます。
    「▶この行を試聴」で1行だけ合成・音響処理して確認、「🎛ラボでこの行を調整」でその行を音源にして音響ラボへ移動します。
    台本の編集はお使いのエディタで行い、保存すると自動再読込されます（UTF-8 保存）。一括実行は下部のボタンから（CLI と同一出力）。</li>
  <li><strong>音響ラボタブ</strong>: 部屋を上から見た図の中で<strong>聴取者👤と話者🔊をドラッグ</strong>すると、
    定位（pan）・距離・聴取者位置が連動して変わります。部屋の<strong>幅W・奥行D・高さH はそれぞれ独立</strong>に
    スライダーまたは数値入力で設定でき、残響時間（RT60）の目安が図の下に表示されます。
    結果はパラメータとペアで履歴に残り、A/B を指定して下部の再生バーで交互に聴き比べできます。
    決まった値は <code>@scene</code> 行としてコピーできます。</li>
  <li><strong>再生バー</strong>: 画面下部に共通の再生バーがあり、<strong>⏹停止</strong>・直前の音の再再生・
    履歴 A/B の交互再生がどのタブからでも行えます。</li>
</ul>
```

- [ ] **Step 2: 最終確認**

Run: `cargo test --workspace` → 全 PASS
Run: `cargo build --release -p s2v-gui` → 成功
Run: `Copy-Item config.toml target\release\ -Force; Copy-Item presets.toml target\release\ -Force` → `.\target\release\s2v-gui.exe` を起動し、以下の手動スモークチェックリストを目視確認（エンジン起動を伴う項目はコントローラ/ユーザーに委ねる）:
1. タブ1: 2ペイン表示・行選択→全文表示
2. タブ2: 俯瞰図ドラッグ→pan/距離が連動、W/D/H 個別変更→図と RT60 が追随
3. 再生バーが両タブに表示・⏹が押せる
4. ウィンドウ縮小でレイアウトが破綻しない（最低 1100×760）

- [ ] **Step 3: コミット**（bd close はコントローラが行う）

```bash
git add docs/manual.html
git commit -m "docs: update GUI manual section for master-detail layout and room view"
```
