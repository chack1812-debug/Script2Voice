# 聴取者・話者の高さ指定（床面からの絶対高さ）設計書

作成日: 2026-06-09

## 背景・目的

現状、高さは config `[audio.early_reflections] ear_height`（既定1.2）の1つだけで、
聴取者（マイク）と全話者（音源）に共通適用され、`early.rs` で `sz = lz`（音源高さ＝聴取者高さ）に固定されている。
台本（scene/cast）レベルの高さ指定や、話者ごとの高さは指定できない。

本機能で、**聴取者の高さを scene 単位**、**話者の高さを cast 単位**で、いずれも**床面からの絶対高さ[m]**で
指定できるようにする。これにより床・天井反射の遅延・距離が高さ差を正しく反映し、ステージ上の演者や
見上げ/見下ろしの位置関係を表現できる。行内臨時パラメータでも話者高さを調整できる（他パラメータと同じ加算）。

## スコープ

- **やること**
  - scene に `listener_z`（聴取者の床からの絶対高さ[m]）を追加。
  - cast に話者の高さ（床からの絶対高さ[m]）を追加。`@cast` 行と行内臨時パラメータ（加算）に対応。
  - 早期反射の音源高さ・聴取者高さを上記から解決する（現状の `sz = lz` 固定を置換）。
- **やらないこと**
  - Sabine 残響への高さ反映（残響は体積・素材から決まる。高さは早期反射の幾何にのみ効く）。
  - 仰角（上下）の知覚レンダリング（従来どおり水平面に畳む）。
  - 高さ方向の指向性（マイク・音源とも無指向と仮定）。

## 1. 聴取者の高さ（scene）

- `SceneConfig`(s2v-core/types.rs) に `#[serde(default)] pub listener_z: Option<f64>` を追加（床からの絶対高さ[m]。
  `listener_dx/dy` の「中央オフセット」とは異なり**絶対値**）。
- `parse_scene_header`(parser.rs) で `listener_z` キーを取り込む。
- `RoomGeometry`(acoustics.rs) に `pub listener_height: f64` を追加。
- `resolve_room_geometry` で `listener_height = scene.listener_z.unwrap_or(er.ear_height)`。

## 2. 話者の高さ（cast）

`Cast`(s2v-core/cast.rs) に2つのフィールドを追加（いずれも `#[serde(default)]`）:
- `pub height: Option<f64>` … 床からの絶対高さ[m]の**基準値**。`None` = 聴取者高さと同じ（後方互換）。`@cast` 行で設定。
- `pub height_offset: f64` … 行内臨時パラメータで加算される**その行限定のオフセット**[m]（既定 0.0）。

**パース**:
- `parse_cast_line` で `height` キーを `pan`/`distance` と同様に専用フィールド `Cast.height`（Option）へ取り込む
  （`raw.remove("height")` し、あれば `Some`）。`height_offset` の初期値は 0.0。
- 行内臨時パラメータ（例 `A(height=0.3):セリフ`）は既存の汎用ルートで `offset_params` に入る（パーサ変更不要）。

**with_offsets**（cast.rs）に `"height"` を追加:
```rust
"height" => cast.height_offset += v,
```
これにより行内 `height` は**基準への加算**（その行限定）として働く。基準が `None`（@cast 未指定）でも、
オフセットは聴取者高さに対して加算される（§3 で合算）。

## 3. 処理への配線

`processor.rs` の `process` で**話者の実効高さ**を解決する:
```
source_height = cast.height.unwrap_or(geo.listener_height) + cast.height_offset
```
これを `build_early_taps` に新引数 `source_height: f64` で渡す。

`early.rs` の `build_early_taps`:
- 聴取者高さ: `let lz = geo.listener_height.clamp(eps, h - eps);`（現状の `er.ear_height` を置換）。
- 音源高さ: `let sz = source_height.clamp(eps, h - eps);`（現状の `sz = lz` を置換）。
- 以降の6面イメージ・`dz`・遅延・ゲインのロジックは不変（`dz = img高さ − lz` が高さ差を反映）。

## 4. 後方互換

- scene `listener_z` 未指定 → `listener_height = ear_height`。
- cast `height` 未指定（None）かつ行内 height なし（offset 0）→ `source_height = ear_height`。
- ⇒ `sz = lz = ear_height` となり**現状と完全一致（byte一致）**。早期反射 `enabled=false` の回帰不変も維持。

## 5. 単位・命名・注意

- すべて**床面からの絶対高さ[m]**。scene=`listener_z`、cast(@cast行・行内)=`height`。
- 高さは Sabine 残響（残響長・wet基準値）には影響しない。早期反射の床/天井/壁反射の幾何にのみ効く。
- 高さは `[eps, room_h − eps]`（eps=0.05m）にクランプし、箱外・退化を防ぐ。

## 6. データ構造変更に伴う影響

- `Cast` への2フィールド追加で、`Cast { ... }` のリテラル構築箇所がすべてコンパイルエラーになる
  （parser.rs、cast.rs テスト、types.rs テスト、processor.rs/engine.rs の `dummy_cast` 等）。
  各リテラルに `height: None, height_offset: 0.0` を追加して追従する。
- `SceneConfig` への `listener_z` 追加は、既存が関数更新構文 `..SceneConfig::new(name)` 化済みのため
  追加実害なし（`new` に `listener_z: None` を足す）。
- `RoomGeometry` への `listener_height` 追加で、その構築箇所（acoustics.rs / early.rs の `geo_for` テストヘルパー）を追従。

## 7. テスト

- `resolve_room_geometry`: `listener_z=2.0` 指定で `listener_height==2.0`、未指定で `ear_height`。
- `Cast::with_offsets`: 行内 `height` が `height_offset` に加算される（`height_offset` の加算、基準 `height` は不変）。
- パーサ: `@cast ... height=1.5` が `Cast.height==Some(1.5)`、行内 `A(height=0.3)` が `offset_params["height"]==0.3`。
- early/process:
  - 話者を高く（または低く）すると床反射の遅延が変わる（基準聴取者高さとの差 `|dz|` が増えると床反射経路が伸びる）。
  - `listener_z`・`height` 未指定で従来出力と一致（回帰不変）。
- 既存テスト全通過。`cargo build && cargo test --workspace` 成功。

## 受け入れ条件

- 台本の scene で聴取者高さ、cast（行・行内）で話者高さを床からの絶対値[m]で指定でき、早期反射に反映される。
- 行内 `height` はその行限定の加算として働く。
- いずれも未指定なら従来挙動と完全一致。全テスト通過。
