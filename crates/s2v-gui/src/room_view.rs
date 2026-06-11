use crate::scene_line::LabParams;

/// 話者と聴取者の距離の下限[m]（0除算・密着防止）
pub const MIN_DISTANCE: f64 = 0.1;
/// 壁からの最小マージン[m]
pub const WALL_MARGIN: f64 = 0.05;

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
