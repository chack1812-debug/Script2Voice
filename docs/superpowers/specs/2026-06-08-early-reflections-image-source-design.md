# room_size連動の一次反射（イメージソース法）＋距離依存D/R 設計書

作成日: 2026-06-08

## 背景・目的

現在のステレオ音響処理は **直接音（空気吸収＋ITD＋ILD）→ 拡散リバーブ（room_sizeキーのノイズIR）** という構成で、
直接音と拡散テールの間を埋める**早期反射が一切ない**（拡散リバーブのプリディレイは 10〜50ms）。
このため音源は左右方向は定位するが距離・上下が曖昧で、ユーザー報告のとおり「**宙に浮いている**」ように聞こえる。

人間は直接音直後（数ms〜20ms）の**床・壁・天井の早期反射**を直接音と融合（先行音効果）して、
音源を実空間に「接地・外在化」させる。本機能はこの早期反射を**イメージソース法**で生成して接地感を与える。
あわせて、距離に応じた直接/残響比（D/R）の制御を強化して距離感を安定させる。

## スコープ

- **やること**
  - 部屋を箱としてモデル化し、寸法を既存 `room_size` に連動させる。
  - 床・天井・前後左右の**6面の一次反射**（イメージソース法）を生成し、直接音の直後に加算する。
  - 面ごとの**反射率**と**周波数依存吸音（ローパス）**を、5素材グループ（床／天井／前壁／後壁／側壁）で指定可能にする。
  - 聴取者位置を部屋中央＋configオフセット、耳高 `ear_height` で配置可能にする。
  - 距離依存 D/R 制御（早期反射レベル＋拡散wetの距離スロープ）を強化・config化する。
- **やらないこと**
  - 二次以上の反射（一次のみ）。
  - 仰角（上下）の知覚レンダリング（HRTF等）。水平面に畳む。
  - Python版への同等移植（**Rust専用の機能追加＝意図的にPythonと乖離**。pan指向性チューニングと同方針）。

## 1. 座標系・部屋モデル

- 箱座標：`x∈[0,W]`（+x=右）、`y∈[0,D]`（+y=前方／聴取者の正面）、`z∈[0,H]`（+z=上）。
- `room_size` r∈[0,1] から寸法を線形補間：`W = Wmin + (Wmax−Wmin)·r`（D,H も同様）。座標は x=幅(左右)、y=奥行(前後／聴取者正面)、z=高さ。
  - config（単位m、[幅W, 奥行D, 高さH]）：
    - `room_dims_min = [4.0, 5.0, 3.0]` … 想定下限＝**ラジオ雑談番組の収録スタジオ**程度の小ブース。
    - `room_dims_max = [25.0, 45.0, 18.0]` … 想定上限＝**約2000席のコンサートホール**（体積≒2万m³級、奥行きが最長）。
- 聴取者 `L = (W/2 + off_x, D/2 + off_y, ear_height)`。**`L` はステレオマイクペアの中心点**であり、
  実際の収音は L を中心に x軸方向へ `±microphone_spacing/2`（既定 ±0.1m）離れた左右2マイク（高さは共に `ear_height`、正面=+y、外開きORTF）で行う。
  各反射の左右差（ITD/ILD）はこのマイク間隔を使う既存 `calc_geometry` を再利用して算出する（直接音と同一のマイクペア）。
  - config：`listener_offset = [ox, oy]`（既定 `[0,0]`）、`ear_height`（既定 1.2、単位m）。マイク間隔は既存 `microphone_spacing` を流用（新規paramなし）。
- 音源 `S = (Lx + distance·sin(pan), Ly + distance·cos(pan), ear_height)`（聴取者と同高）。
  - `pan` は度→rad済み（既存）。`distance` は水平距離。直接距離 `d0 = distance`。
  - S が箱外に出る場合は各軸を `[ε, dim−ε]`（ε=0.05m）にクランプして退化を防ぐ。

## 2. 一次反射（6イメージソース）

各面について音源 S を鏡像化してイメージ位置を得る（Sz=ear_height）：

| 面 | 平面 | イメージ位置 |
|---|---|---|
| 床 | z=0 | (Sx, Sy, −Sz) |
| 天井 | z=H | (Sx, Sy, 2H−Sz) |
| 左壁 | x=0 | (−Sx, Sy, Sz) |
| 右壁 | x=W | (2W−Sx, Sy, Sz) |
| 後壁 | y=0 | (Sx, −Sy, Sz) |
| 前壁 | y=D | (Sx, 2D−Sy, Sz) |

各イメージ I=(Ix,Iy,Iz) について、聴取者 L 基準で：
- ベクトル `v = I − L = (dx, dy, dz)`、3D経路 `path = |v|`。
- **遅延**：直接音に対する追加遅延 `Δt = (path − d0)/c`（秒）→サンプル。各chの相対ITDは下記の水平幾何から加算する。
- **方位・ITD/ILD**：水平投影 `azimuth = atan2(dx, dy)`、水平距離 `hdist = sqrt(dx²+dy²)` を
  **既存 `calc_geometry(hdist, azimuth)` に渡して再利用**し、`dist_l/dist_r`（相対ITD用）・`angle_l/angle_r`（pat用）を得る。
  上下成分 dz は左右ITDに寄与しないため、共通の遅延 Δt に含めて扱う（水平面へ畳む近似）。
- **レベル**：各ch `gain = vol_factor · (ref_dist / max(path, 0.1)) · pat_ch · reflection_coeff(面)`。
  距離減衰に 3D 経路 `path` を用いることで、高さ迂回（床/天井反射の遠回り）も反映する。
  `pat_ch` は方位 azimuth に対する既存の指向性パターン式（外開き ORTF 配置）を流用。

直接音と各イメージは**同一のステレオ化ヘルパー**（§5）で配置し、ステレオバッファへ加算する。

## 3. 素材（反射率＋周波数依存吸音）

面を **5素材グループ**に分類し、各グループに反射率と吸音カットオフを持たせる：

| 素材グループ | 対象面 | 既定 reflection_coeff | 既定 absorption_cutoff_hz |
|---|---|---|---|
| floor | 床 | 0.5 | 3500 |
| ceiling | 天井 | 0.6 | 6000 |
| front_wall | 前壁(y=D) | 0.85 | 10000 |
| back_wall | 後壁(y=0) | 0.40 | 4000 |
| side_walls | 左壁(x=0)・右壁(x=W) | 0.70 | 8000 |

- 反射ごとに、その素材の **2次Butterworthローパス**（`absorption_cutoff_hz`、既存 `butterworth_lowpass_sos`/`sosfilt` を流用）を適用後、`reflection_coeff` を乗じてからステレオ配置する。
- `absorption_cutoff_hz` を Nyquist 付近（例 24000）にすれば周波数依存吸音を実質無効化できる。
- 前壁の高反射（0.85）でコンサートホール的な前方からの早期反射の強さを表現。後壁は吸音寄り（0.40）。
- 左右側壁は素材を共通とする（聴取オフセットにより距離は左右非対称になるが、素材は対称）。

## 4. 距離依存 D/R 制御（feature 3）

- 直接音：従来どおり逆二乗（変更なし）。
- 早期反射：§2 の物理レベルに全体係数 `early_level`（既定 1.0）を乗じる。
- 拡散テール：`IrCache::apply` の `actual_wet = reverb_wet·(1 + wet_distance_slope·avg_dist)`（上限0.9）の
  **`wet_distance_slope` を config 化**（現状はリテラル 0.1）。既定 0.1 で後方互換。
- 結果：遠い音源ほど（早期＋拡散）/直接 比が増え、距離感が安定する。

## 5. モジュール構成・処理順

- 新規 `crates/s2v-audio/src/early.rs`
  - 部屋寸法算出（room_size→W,D,H）、聴取者/音源配置、6イメージ生成、各イメージの遅延・素材ローパス・ステレオ配置・加算。
  - 公開関数（例）：`add_early_reflections(stereo: &mut Vec<[f32;2]>, mono: &[f32], geom: &SourceGeom, cfg: &EarlyConfig, audio: &AudioConfig, room_size: f64, sample_rate: u32, base_offset: usize)`。
- `processor.rs`
  - 直接音のステレオ化（幾何→gain_l/gain_r→相対遅延→data_l/data_r 配置）を**再利用ヘルパー** `spatialize_into(stereo, mono, distance, pan, level_scale, base_offset, ...)` に切り出し、直接音と各イメージで共用する。
  - 処理順：**直接音 → 早期反射(early.rs) → 拡散リバーブ(IrCache) → リミッター**。
  - 出力バッファ長は、最遠イメージの遅延と拡散テール長を見込んで拡張する。
- `AudioConfig`（s2v-core）を拡張
  - `[audio.early_reflections]` セクションを追加し、`enabled`（既定 true）、`ear_height`、`listener_offset`、
    `room_dims_min`/`room_dims_max`、5素材の `reflection_coeff`/`absorption_cutoff_hz`、`early_level`、
    `wet_distance_slope` を持つ。**全フィールド serde default** で既存 config.toml と後方互換（セクション欠落時は既定値）。

## 6. テスト

- **幾何（純粋関数）**：
  - room_size→寸法の線形補間が端点・中点で期待値。
  - 既知の部屋寸法・`ear_height`・`distance` での**床反射遅延**が解析値 `(sqrt(d0²+(2·ear_height)²)−d0)/c` と一致。
  - 6イメージ位置・path・追加遅延が手計算値と一致。
  - 音源クランプが箱外指定で内側に収まる。
- **統合（process）**：
  - early 有効時、期待遅延位置に反射エネルギーが乗る（無音区間に非ゼロが出る）。
  - early `enabled=false` で従来出力と一致（回帰防止）。
  - 前壁 reflection_coeff を上げると前壁イメージ由来の早期反射レベルが増える。
  - `absorption_cutoff_hz` を下げると当該反射の高域が減衰する。
- **距離D/R**：`distance` を大きくすると（早期＋拡散）/直接 のエネルギー比が増大する。
- 既存テスト（reverb.rs / processor.rs）がすべて通る。`cargo build && cargo test` が成功する。

## 受け入れ条件

- early_reflections 有効時、直接音の直後（数ms〜数十ms）に床・壁・天井由来の一次反射が付加され、接地感が出る。
- 反射の遅延・レベルが音源距離・room_size・聴取オフセットに追従する。
- 5素材の反射率・吸音カットオフが個別に効く（前壁高反射・後壁吸音などが表現できる）。
- 距離が増すと D/R が下がる（より残響的になる）。
- `enabled=false` または該当セクション欠落時は従来挙動と一致。全テスト通過。
