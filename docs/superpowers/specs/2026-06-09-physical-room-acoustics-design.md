# 物理ベースの部屋音響（台本で寸法・聴取位置／素材で残響を導出）設計書

作成日: 2026-06-09

## 背景・目的

直前に実装した早期反射（イメージソース法）は、部屋寸法を `room_size`(0..1) から補間し、
聴取者位置を config の `listener_offset` から取っていた。一方、拡散リバーブは
`room_size`→経験式 `rv_time=0.05+room_size·3.0` と手動 `reverb_wet` という**別系統**で動いており、
壁素材（反射率/吸音）は早期反射にしか効いていない。

本機能は、台本（scene）で**実際の部屋寸法と聴取者位置**を指定できるようにし、
**素材×寸法から拡散リバーブ（残響長・残響量）を物理的に導出**することで、早期反射と残響を
ひとつの物理モデルに統一する。`reverb_wet` は廃止せず「物理基準値へのスケーラ」として残し、
ナレーションの明瞭さ等の演出制御を維持する。屋外などは壁の反射率を 0 にすれば自然に表現できる。

## スコープ

- **やること**
  - scene で部屋寸法 `room_w/room_d/room_h` と聴取者位置 `listener_dx/listener_dy`(中央からのオフセット) を指定可能にする。
  - 寸法×素材から Sabine の式で残響長 RT60 を導出し、拡散リバーブの `rv_time` に用いる。
  - 平均自由行程からプリディレイを導出する。
  - 平均吸音から wet 基準値を導出し、`reverb_wet`(既定1.0) をそのスケーラにする。
  - 早期反射と拡散リバーブを、解決済みの単一の部屋ジオメトリ＋共通の素材から駆動する。
- **やらないこと**
  - 二次以上の反射（早期反射は一次のまま）。
  - 周波数帯域別の残響時間（RT60 は広帯域の単一値）。
  - Python 版への移植（Rust 専用拡張。意図的に乖離）。

## 用語・座標

座標は既存の早期反射と同一: x=幅(左右,+x右)、y=奥行(前後,+y前方)、z=高さ(+z上)。
箱は x∈[0,W], y∈[0,D], z∈[0,H]。聴取者(マイクペア中心) は部屋水平中央＋オフセット、高さ `ear_height`。

## 1. 部屋ジオメトリの解決（scene → 寸法＋聴取位置）

処理時に scene と config から単一の `RoomGeometry` を解決する。

```
struct RoomGeometry { dims: [f64; 3], listener_offset: [f64; 2] }
```

解決規則（優先順）:
- 寸法 `dims`:
  - scene に `room_w`,`room_d`,`room_h` が**すべて**あればそれを使用。
  - そうでなければ scene の `room_size`（無ければ AudioConfig 既定の解決値）から
    `room_dims(room_size, room_dims_min, room_dims_max)` で補間（既存関数）。
- 聴取オフセット `listener_offset`:
  - scene に `listener_dx`/`listener_dy` があればその値（欠けた成分は 0）。
  - なければ config `early_reflections.listener_offset`。

`RoomGeometry` は早期反射（§6）と残響導出（§2,§3）の両方に渡す。Cast の `distance`/`pan` は不変（聴取者相対）。

## 2. 残響長 RT60 の物理導出（Sabine）

素材は既存5素材（floor/ceiling/front_wall/back_wall/side_walls）の `reflection_coeff` を流用。

- 面積: `S_floor = S_ceiling = W·D`、`S_front = S_back = W·H`、`S_side = 2·D·H`（左右2枚）。総面積 `S = 2WD + 2WH + 2DH`。
- 吸音率(エネルギー): `α_i = 1 − coeff_i²`（`reflection_coeff` は振幅反射係数）。
- 総吸音: `A = WD·α_floor + WD·α_ceiling + WH·α_front + WH·α_back + 2DH·α_side`。
- 体積: `V = W·D·H`。
- **RT60**: `rt60 = (0.161 · V / A.max(1e-6)).clamp(0.05, 12.0)`（秒。`A.max(1e-6)` は0除算回避）。
  - 下限0.05s/上限12.0sは定数（全面無吸音 A→0 での発散と、過小値を防ぐ）。
- 拡散リバーブIRの `rv_time = rt60`（現IRの減衰 `exp(−6.91·t/rv_time)` は t=rv_time で−60dB＝RT60の定義に一致）。

プリディレイ:
- 平均自由行程 `mfp = 4V/S`、`pre_delay_s = mfp / sound_speed`。
- `pre_delay_samples = (fs · pre_delay_s) as usize`（現 `fs·(0.01+0.04·room_size)` を置換）。

現 `rv_time = 0.05 + room_size·3.0` および `pre_delay = fs·(0.01+0.04·room_size)` の経験式は廃止。

## 3. reverb_wet の物理基準値スケーラ化

- 平均吸音: `avg_alpha = A / S`。
- **wet基準値**: `wet_base = (1.0 − avg_alpha).clamp(0.0, 1.0)`（屋外/高吸音→0、無吸音→1）。
- 実効wet: `actual_wet = (reverb_wet · wet_base · (1.0 + wet_distance_slope · avg_dist)).min(0.9)`。
- `reverb_wet` の意味を「絶対wet量」から「物理基準値への倍率」へ変更。**既定 1.0**。
  - config.toml の `[audio] reverb_wet` を 0.7 → 1.0 に更新。
  - AudioConfig のフィールド名は `reverb_wet` のまま（互換のため）。

**非互換（明示）**: この変更と §2 の Sabine 化により、既存台本の残響の長さ・量は変わる（要再調整）。

## 4. 台本（scene）構文と SceneConfig 拡張

scene ヘッダは既存の `@scene 名前 key=value ...`（`key=f64`）形式。新キーを追加:

```
@scene ホール room_w=25 room_d=45 room_h=18 listener_dx=0 listener_dy=-15 reverb_wet=1.0
@scene 小部屋 room_size=0.1
@scene 屋外 room_w=50 room_d=50 room_h=30 reverb_wet=0.0
```

`SceneConfig`(s2v-core/types.rs) に `Option<f64>` フィールドを追加:
```
room_w, room_d, room_h, listener_dx, listener_dy   // すべて Option<f64>、既定 None
```
`parse_scene_header`(parser.rs) で各キーを `params.get(...).copied()` で取り込む。既存 `room_size`/`reverb_wet` は維持。
Cast 行（`distance`/`pan` 等）は不変。

## 5. IrCache のキー変更（room_size → (rt60, pre_delay)）

現在 IR は `room_size`(round4) をキーにキャッシュし `build_ir(room_size, fs)` で生成している。
新方式では IR は `(rt60, pre_delay)` に依存する。

- キャッシュキー: `(OrderedFloat(round4(rt60)), pre_delay_samples)`。
- 生成関数: `build_ir(rt60: f64, pre_delay: usize, fs: u32)`（内部の `rv_time=rt60`、`pre_delay` をそのまま使用。乱数シードは `rt60` ベースに変更し決定性維持）。
- `compute_if_needed(rt60, pre_delay)` / `prewarm(&[(rt60, pre_delay)])`。
- `apply(&mut stereo, rt60, pre_delay, reverb_wet, wet_base, avg_dist, wet_distance_slope)`:
  IR取得キーを `(rt60, pre_delay)` にし、`actual_wet` を §3 の式（`wet_base` を含む）で計算。

プリウォーム呼び出し側（main/producer）:
- 現在は全 scene の `room_size` 集合をプリウォーム。
- 新たに、各 scene を §1 で解決し §2 で `(rt60, pre_delay)` を算出した集合をプリウォームする。

## 6. 早期反射を解決済みジオメトリで駆動

`build_early_taps`（early.rs）の内部で `room_dims(room_size, ...)` と `er.listener_offset` を使っていた箇所を、
**引数で渡す `RoomGeometry`（`dims` と `listener_offset`）に置換**する。6面イメージ・遅延・ゲイン・素材ローパスのロジックは不変。
`ear_height`・素材・`early_level` は引き続き EarlyConfig から取得。

## 7. 屋外などの扱い

天井・壁の `reflection_coeff=0` にすると:
- 早期反射: その面のタップが消える（既存の `coeff<=0` スキップ）。床のみ残る。
- §2: それらの面で `α=1` となり A 増大 → RT60 短縮。
- §3: `avg_alpha` 増大 → `wet_base → 0` → ほぼ無残響。

→ 「床反射のみ＋ほぼ無残響」＝屋外。全面 `coeff=1` の極端は RT60 上限(12s)クランプで保護。

## 8. 互換性・優先順位

- scene が `room_w/d/h` を省略 → `room_size`→寸法補間（従来の寸法解決）。`room_size` も省略 → AudioConfig 既定で解決。
- scene が `listener_dx/dy` を省略 → config `listener_offset`。
- 早期反射 `enabled=false`: 早期反射の加算は無効（その部分は回帰不変）。ただし**拡散リバーブの物理化（§2,§3）は早期反射のON/OFFと独立**に適用されるため、残響の音は経験式時代から変わる。
- **明示的な非互換**: `reverb_wet` がスケーラ化、残響長が Sabine 化。既存台本の響きは変化する。config 既定 `reverb_wet=1.0`。

## 9. テスト

- **物理/幾何の純粋関数**（新規 geometry/reverb 関数）:
  - Sabine `rt60`: 既知寸法・素材で解析値一致。全面 `coeff=1`(無吸音) で上限12sにクランプ。高吸音で短く（下限0.05s方向）。
  - `wet_base = (1−avg_alpha)`: 全面吸音(coeff=0)で→~0、無吸音(coeff=1)で→~1。
  - 面積・体積・平均自由行程の計算。
  - 屋外（壁・天井 coeff=0、床 coeff>0）で rt60 が短く wet_base が小さい。
- **パーサ**: `@scene ... room_w/d/h/listener_dx/dy` の取り込み、`room_size` フォールバック、優先順位（room_w/d/h があれば room_size より優先）。
- **IrCache**: `(rt60,pre_delay)` キーでのキャッシュ・プリウォーム、`build_ir` 決定性、`apply` が wet_base を反映。
- **早期反射**: `RoomGeometry` を渡す形へ変更後も既存の挙動（床タップ遅延・左右非対称・disabled空）を維持。
- **統合(process)**: scene の寸法指定が早期反射と残響の両方に効く。屋外設定（壁0）でほぼ無残響＋床反射のみになる。

## 受け入れ条件

- 台本の scene で部屋寸法・聴取者位置を指定でき、早期反射と拡散リバーブの両方に反映される。
- 残響長・残響量が寸法×素材から物理的に決まり、`reverb_wet` がその倍率として働く。
- 壁・天井の反射率を 0 にすると屋外的（床反射のみ・ほぼ無残響）になる。
- `room_w/d/h` 省略時は従来どおり `room_size` で解決できる。
- 全テスト通過。`cargo build && cargo test --workspace` 成功。
