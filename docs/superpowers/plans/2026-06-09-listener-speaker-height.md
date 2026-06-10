# 聴取者・話者の高さ指定 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 台本で聴取者の高さ(scene)・話者の高さ(cast、行内臨時パラメータ加算対応)を床面からの絶対高さ[m]で指定でき、早期反射の床/天井反射の幾何に反映する。

**Architecture:** scene `listener_z` を `RoomGeometry.listener_height` に解決し、cast `height`(基準・絶対、None=聴取者高さ)＋`height_offset`(行内加算)から話者実効高さを求め、`build_early_taps` に `source_height` として渡す。高さは早期反射の幾何のみに作用（Sabine 残響には不影響）。全未指定で従来とbyte一致。

**Tech Stack:** Rust / s2v-core / s2v-audio — 新規依存なし

設計書: `docs/superpowers/specs/2026-06-09-listener-speaker-height-design.md`

**コンパイル順の注意:** `Cast` への2フィールド追加は s2v-core/s2v-audio/s2v-engines の全 `Cast { }` リテラルを同時に壊すため、それらの更新は **Task 2 に集約**する。`build_early_taps` のシグネチャ変更は processor を壊すため **Task 3 に集約**（末尾で `cargo test -p s2v-audio` 緑化）。

---

## Task 1: scene に聴取者高さ listener_z を追加

**Files:**
- Modify: `crates/s2v-core/src/types.rs`
- Modify: `crates/s2v-core/src/parser.rs`

- [ ] **Step 1: パーサ失敗テストを書く** — `crates/s2v-core/src/parser.rs` の `mod tests` 末尾に追加:
```rust
    #[test]
    fn scene_header_parses_listener_z() {
        let p = ScriptParser::new();
        let sc = p.parse_scene_header("舞台 room_w=20 room_d=30 room_h=12 listener_z=1.1");
        assert_eq!(sc.listener_z, Some(1.1));
        let sc2 = p.parse_scene_header("小部屋 room_size=0.1");
        assert_eq!(sc2.listener_z, None);
    }
```

- [ ] **Step 2: 失敗確認** — Run: `cargo test -p s2v-core scene_header_parses_listener_z` → FAIL（`listener_z` 未定義でコンパイルエラー）。

- [ ] **Step 3: SceneConfig に listener_z を追加** — `crates/s2v-core/src/types.rs` の `SceneConfig` struct に追加（`listener_dy` の後）:
```rust
    /// 聴取者(マイクペア中心)の床面からの絶対高さ[m]。省略時は config の ear_height。
    #[serde(default)]
    pub listener_z: Option<f64>,
```
`SceneConfig::new` に `listener_z: None,` を追加（`listener_dy: None,` の後）。
`types.rs` の `scene_config_defaults` テストに `assert_eq!(sc.listener_z, None);` を追加。

- [ ] **Step 4: parse_scene_header に listener_z を追加** — `crates/s2v-core/src/parser.rs` の `parse_scene_header` の戻り値 `SceneConfig { ... }` に追加（`listener_dy:` の行の後）:
```rust
            listener_z: params.get("listener_z").copied(),
```

- [ ] **Step 5: テスト通過確認** — Run: `cargo test -p s2v-core` → PASS（新テスト＋既存全通過）。

- [ ] **Step 6: コミット**
```bash
git add crates/s2v-core/src/types.rs crates/s2v-core/src/parser.rs
git commit -m "feat(core): add scene listener_z (listener height) parameter"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Task 2: cast に話者高さ height / height_offset を追加（全 Cast リテラル更新）

**重要:** `Cast` へのフィールド追加は全 `Cast { }` リテラルを壊す。本タスクで s2v-core/s2v-audio/s2v-engines の全リテラルを更新し、末尾で `cargo test --workspace` を緑にしてからコミットする。

**Files:**
- Modify: `crates/s2v-core/src/cast.rs`（フィールド・with_offsets・base_cast・テスト）
- Modify: `crates/s2v-core/src/parser.rs`（parse_cast_line で height 取り込み）
- Modify: `crates/s2v-core/src/types.rs`（テストの Cast リテラル）
- Modify: `crates/s2v-audio/src/processor.rs`（dummy_cast）
- Modify: `crates/s2v-engines/src/engine.rs`、`crates/s2v-engines/src/xtts_engine.rs`、`crates/s2v-engines/src/http_engine.rs`（dummy_cast）

- [ ] **Step 1: with_offsets/パーサの失敗テストを書く** — `crates/s2v-core/src/cast.rs` の `mod tests` に追加:
```rust
    #[test]
    fn with_offsets_height_accumulates_into_offset() {
        // 行内 height はその行限定の加算として height_offset に積まれる(基準 height は不変)
        let cast = base_cast(); // height=None, height_offset=0.0
        let mut offsets = HashMap::new();
        offsets.insert("height".to_string(), 0.5_f64);
        let eff = cast.with_offsets(&offsets);
        assert!((eff.height_offset - 0.5).abs() < 1e-10);
        assert_eq!(eff.height, cast.height);
    }
```
`crates/s2v-core/src/parser.rs` の `mod tests` に追加:
```rust
    #[test]
    fn cast_line_parses_height_into_field() {
        let scenes = ScriptParser::new()
            .parse_str("@scene テスト room_size=0.1\n@cast\nA:話者:ノーマル,voicevox,pan=0,height=1.7\n@script\nA: こんにちは\n")
            .unwrap();
        let cast = scenes[0].casts.get("A").unwrap();
        assert_eq!(cast.height, Some(1.7));
        assert!((cast.height_offset - 0.0).abs() < 1e-10);
    }
```
（`parse_str(&mut self, &str) -> anyhow::Result<Vec<Scene>>` は確認済み。既存 cast パーサテストと同じく `ScriptParser::new().parse_str(...).unwrap()` で呼ぶ。`@cast` 行の書式は `名前:話者名:スタイル,engine,key=val,...`。）

- [ ] **Step 2: 失敗確認** — Run: `cargo test -p s2v-core with_offsets_height_accumulates_into_offset` → FAIL（`height`/`height_offset` 未定義でコンパイルエラー）。

- [ ] **Step 3: Cast にフィールド追加 + with_offsets** — `crates/s2v-core/src/cast.rs` の `Cast` struct に追加（`params` の後）:
```rust
    /// 話者の床面からの絶対高さ[m]の基準値。None = 聴取者と同じ高さ。
    #[serde(default)]
    pub height: Option<f64>,
    /// 行内臨時パラメータで加算される、その行限定の高さオフセット[m]。
    #[serde(default)]
    pub height_offset: f64,
```
`with_offsets` の match に `"height"` アームを追加（`"volume" =>` の後、`other =>` の前）:
```rust
                "height" => cast.height_offset += v,
```

- [ ] **Step 4: parse_cast_line で height を取り込む** — `crates/s2v-core/src/parser.rs` の `parse_cast_line` で、`volume` を remove している箇所の後に追加:
```rust
        let height = raw.remove("height");
```
同関数の `Cast { ... }` リテラルに `height, height_offset: 0.0,` を追加（`params,` の後）:
```rust
            Cast { name, speaker_name, engine_type, pan, distance, volume, params, height, height_offset: 0.0 },
```

- [ ] **Step 5: 全 Cast リテラルにフィールドを追加** — 以下の各 `Cast { ... }` リテラルに `height: None, height_offset: 0.0,` を追加する:
  - `crates/s2v-core/src/cast.rs` の `base_cast()`（`params: { ... },` の後に追加）。
  - `crates/s2v-core/src/types.rs` の `scene_accepts_casts` テスト内 Cast。
  - `crates/s2v-audio/src/processor.rs` の `dummy_cast()`（`params: HashMap::new(),` の後）。
  - `crates/s2v-engines/src/engine.rs` の `dummy_cast()`。
  - `crates/s2v-engines/src/xtts_engine.rs` の `dummy_cast`（または Cast リテラル）。
  - `crates/s2v-engines/src/http_engine.rs` の `dummy_cast`（または Cast リテラル）。
  各ファイルで `grep -n 'Cast {' <file>` で該当行を特定し、漏れなく更新すること。

- [ ] **Step 6: テスト通過確認** — Run: `cargo test --workspace` → PASS（新2テスト＋全既存。フィールド追加で全クレートがコンパイルできること）。

- [ ] **Step 7: コミット**
```bash
git add crates/s2v-core/src/cast.rs crates/s2v-core/src/parser.rs crates/s2v-core/src/types.rs crates/s2v-audio/src/processor.rs crates/s2v-engines/src/engine.rs crates/s2v-engines/src/xtts_engine.rs crates/s2v-engines/src/http_engine.rs
git commit -m "feat(core): add cast speaker height (base + per-line offset)"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Task 3: 早期反射へ聴取者・話者高さを配線（RoomGeometry + early + processor）

**重要:** `build_early_taps` のシグネチャ変更が processor を壊すため、acoustics・early・processor をまとめて変更し、末尾で `cargo test -p s2v-audio` 緑化後にコミットする。

**Files:**
- Modify: `crates/s2v-audio/src/acoustics.rs`（RoomGeometry.listener_height + resolve）
- Modify: `crates/s2v-audio/src/early.rs`（build_early_taps source_height + lz/sz + geo_for）
- Modify: `crates/s2v-audio/src/processor.rs`（source_height 解決・受け渡し）

- [ ] **Step 1: acoustics に listener_height を追加** — `crates/s2v-audio/src/acoustics.rs`:
`RoomGeometry` struct に追加:
```rust
    pub listener_height: f64,
```
`resolve_room_geometry` の戻り値構築を:
```rust
    RoomGeometry { dims, listener_offset, listener_height: scene.listener_z.unwrap_or(er.ear_height) }
```
acoustics の `mod tests` に追加:
```rust
    #[test]
    fn resolve_listener_height_uses_scene_then_config() {
        let mut er = EarlyConfig::default();
        er.ear_height = 1.2;
        let scene_none = SceneConfig::new("x");
        assert!((resolve_room_geometry(&scene_none, &er, 0.5).listener_height - 1.2).abs() < 1e-10);
        let scene_set = SceneConfig { listener_z: Some(2.0), ..SceneConfig::new("x") };
        assert!((resolve_room_geometry(&scene_set, &er, 0.5).listener_height - 2.0).abs() < 1e-10);
    }
```

- [ ] **Step 2: early.rs に source_height を導入** — `crates/s2v-audio/src/early.rs`:
`build_early_taps` のシグネチャに `source_height: f64` を追加（`geo: &RoomGeometry` の直後）:
```rust
pub fn build_early_taps(
    mono: &[f32],
    distance: f64,
    pan_rad: f64,
    vol_factor: f64,
    audio: &AudioConfig,
    er: &EarlyConfig,
    geo: &RoomGeometry,
    source_height: f64,
    sample_rate: u32,
    min_delay_direct: usize,
) -> Vec<EarlyTap> {
```
本体の高さ取得を変更（現在 `let lz = er.ear_height.clamp(...)` と `let sz = lz;`）:
```rust
    let lz = geo.listener_height.clamp(eps, h - eps);
    let sx = (lx + distance * pan_rad.sin()).clamp(eps, w - eps);
    let sy = (ly + distance * pan_rad.cos()).clamp(eps, d - eps);
    let sz = source_height.clamp(eps, h - eps);
```
（`lx`/`ly` の行はそのまま。`sz = lz;` を `sz = source_height.clamp(eps, h - eps);` に置換。`er` は素材等で引き続き使うため import は不変。）

early.rs テストの更新:
- `geo_for` ヘルパーに `listener_height` を追加:
```rust
    fn geo_for(room_size: f64, er: &EarlyConfig) -> RoomGeometry {
        RoomGeometry {
            dims: crate::geometry::room_dims(room_size, er.room_dims_min, er.room_dims_max),
            listener_offset: er.listener_offset,
            listener_height: er.ear_height,
        }
    }
```
- 各 `build_early_taps(..., &geo_for(0.1, &er), 48000, 0)` 呼び出しに `source_height` 引数を追加。従来挙動を保つため `er.ear_height` を渡す:
  `build_early_taps(..., &geo_for(0.1, &er), er.ear_height, 48000, 0)`。
  対象: `disabled_returns_no_taps`, `only_surfaces_with_positive_coeff_produce_taps`, `floor_tap_delay_matches_analytic_value`, `panned_source_produces_left_right_asymmetric_taps`, `front_wall_reflection_coeff_scales_tap_gain`。
- 新規テスト（話者高さで床反射遅延が変わる）を追加:
```rust
    #[test]
    fn higher_source_increases_floor_reflection_delay() {
        // 床のみ残し、話者を高くすると床反射(像はz=-source_height)の経路が伸びて遅延が増える
        let mut er = EarlyConfig::default();
        er.ceiling.reflection_coeff = 0.0;
        er.front_wall.reflection_coeff = 0.0;
        er.back_wall.reflection_coeff = 0.0;
        er.side_walls.reflection_coeff = 0.0;
        let mono = vec![1.0_f32; 2000];
        let geo = geo_for(0.5, &er);
        let low = build_early_taps(&mono, 2.0, 0.0, 1.0, &audio_cfg(), &er, &geo, 1.2, 48000, 0);
        let high = build_early_taps(&mono, 2.0, 0.0, 1.0, &audio_cfg(), &er, &geo, 2.5, 48000, 0);
        assert_eq!(low.len(), 1);
        assert_eq!(high.len(), 1);
        assert!(high[0].rel_l > low[0].rel_l, "話者が高いほど床反射が遅い: low={}, high={}", low[0].rel_l, high[0].rel_l);
    }
```

- [ ] **Step 3: processor で source_height を解決・受け渡し** — `crates/s2v-audio/src/processor.rs` の `process`:
`build_early_taps` 呼び出しの直前で話者実効高さを算出し、引数に渡す。現在の呼び出し（`&room_geo` を渡している箇所）を:
```rust
        let source_height = cast.height.unwrap_or(room_geo.listener_height) + cast.height_offset;
        let early_taps = build_early_taps(
            &mono, cast.distance, pan_rad, vol_factor,
            &self.config, &self.config.early_reflections,
            &room_geo, source_height, self.config.sample_rate, min_delay,
        );
```
（`room_geo` は既存の解決済み `RoomGeometry`。`cast` は process の引数。）

processor の `mod tests` に統合テストを追加:
```rust
    #[test]
    fn speaker_height_changes_process_output() {
        // 話者高さを変えると早期反射の床反射が変わり、出力(早期反射ON)が変化する
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.wav");
        write_noise_wav(&input, 48000, 0.1);
        let mut cfg = default_audio_config();
        cfg.reverb_wet = 0.0; // 残響を切り早期反射のみ比較
        let proc = AudioProcessor::new(cfg);
        let scene = SceneConfig { room_w: Some(8.0), room_d: Some(8.0), room_h: Some(5.0), reverb_wet: Some(0.0), ..SceneConfig::new("室") };

        let mut low = dummy_cast(0.0, 2.0); low.height = Some(1.0);
        let mut high = dummy_cast(0.0, 2.0); high.height = Some(3.0);
        let out_low = dir.path().join("low.wav");
        let out_high = dir.path().join("high.wav");
        proc.process(&input, &out_low, &low, &scene).unwrap();
        proc.process(&input, &out_high, &high, &scene).unwrap();

        let read = |p: &std::path::Path| -> Vec<i16> {
            let mut r = hound::WavReader::open(p).unwrap();
            r.samples::<i16>().map(|s| s.unwrap()).collect()
        };
        assert_ne!(read(&out_low), read(&out_high), "話者高さで出力が変わること");
    }
```

- [ ] **Step 4: 全体テスト** — Run: `cargo test -p s2v-audio` → PASS（acoustics 新テスト・early 新テスト・processor 新テスト＋全既存。未指定時は従来一致）。さらに `cargo test --workspace` も PASS。

- [ ] **Step 5: コミット**
```bash
git add crates/s2v-audio/src/acoustics.rs crates/s2v-audio/src/early.rs crates/s2v-audio/src/processor.rs
git commit -m "feat(audio): apply scene listener height and cast speaker height to early reflections"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Task 4: ドキュメント更新と最終確認

**Files:**
- Modify: `台本仕様.txt`
- Modify: `docs/early_reflections.html`

- [ ] **Step 1: 台本仕様.txt に高さを追記** — `@scene` の音響場設定に `listener_z=**`（聴取者の床からの絶対高さ[m]、省略時 config ear_height）を追記する。`@cast` の位置パラメータ説明（`pan`/`distance` を記載している箇所）に `height=**`（話者の床からの絶対高さ[m]、省略時=聴取者と同高、行内臨時パラメータでその行限定の加算が可能）を追記する。Edit ツールで UTF-8/日本語を保って編集すること。

- [ ] **Step 2: early_reflections.html に高さを追記** — `docs/early_reflections.html` の「7. 台本(scene)で部屋を指定する」の聴取位置の表に `listener_z`（床からの絶対高さ）の行を追加し、別途「話者の高さ」（`@cast height` と行内 `A(height=**)` の加算）を説明する短い段落または表を追加する。Edit ツールで UTF-8 を保つこと。footer の対応コミット範囲は変更不要（任意）。

- [ ] **Step 3: 全体ビルド・テスト・リリース** — Run: `cargo test --workspace` → 全 PASS（件数記録）。`cargo build --release` → 成功確認。

- [ ] **Step 4: スモーク** — Run: `cargo run -- <listener_z/height を含む台本.txt>`（config.toml を実行ファイル隣に置く前提。`@scene ... listener_z=1.1` と `@cast ... height=1.7` を含む台本でパース・パイプライン起動を確認）。エラーなく起動すれば OK。

- [ ] **Step 5: コミット**
```bash
git add "台本仕様.txt" docs/early_reflections.html
git commit -m "docs: document listener_z and cast height parameters"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Self-Review メモ

- **Spec coverage**: §1 聴取者高さ=Task1(SceneConfig/parser)+Task3(RoomGeometry.listener_height/resolve)。§2 話者高さ=Task2(Cast.height/height_offset/with_offsets/parser)。§3 配線=Task3(early lz/sz + processor source_height)。§4 後方互換=未指定時 ear_height で sz=lz（Task3、source_height に er.ear_height を渡すテストで担保、process 既存テストで回帰確認）。§5 単位/命名=Task1/2 のフィールド名。§6 リテラル追従=Task2(Cast)+Task1(SceneConfig new)+Task3(RoomGeometry/geo_for)。§7 テスト=各Task。ドキュメント=Task4。
- **Placeholder scan**: なし（Task2 Step1 の parse_str は「存在確認し既存書式に合わせる」と明記。実装者は既存 cast パーサテストの入力経路に倣う）。
- **Type consistency**: `SceneConfig.listener_z:Option<f64>`（Task1）、`Cast.height:Option<f64>`/`Cast.height_offset:f64`/with_offsets "height"=>height_offset+=v（Task2）、`RoomGeometry.listener_height:f64`/resolve（Task3）、`build_early_taps(..., geo:&RoomGeometry, source_height:f64, ...)`（Task3）、processor `source_height = cast.height.unwrap_or(room_geo.listener_height) + cast.height_offset`（Task3）は定義と使用で一致。
- **コンパイル順**: Cast フィールド追加は全リテラルを壊すため Task2 に集約し workspace 緑化後コミット。build_early_taps 変更は processor を壊すため Task3 に集約。Task1 は SceneConfig 関数更新構文済みで単独緑。
