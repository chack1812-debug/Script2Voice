# 行プレビュー「発声しない」報告の修正計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 「試聴を押してもエンジン起動ログ以降なにも起きない（ように見える）」問題を、進行状況・エラーの可視化とエンジン起動タイムアウトの引き上げで解消する。

**Beads:** 計画実行時に発行

## 調査結果（2026-06-11、systematic-debugging）

**再現テスト（証拠）:**
- ヘッドレスプローブ `examples/preview_probe.rs`（GUI と同じ 2 ワーカー tokio ランタイム）で
  プレビュー経路（エンジン自動起動→合成→音響処理）は **voicevox: 5.0秒 / aivis: 15.6秒で正常完走**。
- GUI プロセス内の自動試聴プローブ（`S2V_GUI_PROBE=1`）でも**再生開始まで正常完走**（5.2秒）。
- ⇒ パイプライン自体にバグは無い。

**ユーザー症状と一致する構造的問題（根本原因）:**
1. **無反応に見える待ち時間**: エンジン初回起動（AivisSpeech は10秒超、モデルロードが遅い環境では
   さらに長い）＋合成（長文で数秒〜）の間、UI には「▶ボタンが無効になる」以外の表示が一切ない。
2. **失敗の不可視**: 試聴失敗（起動タイムアウト等）は調査前の版では**ログに記録されず**、
   エラーバナーは**タブ1にしか出ない**。🎛 でタブ2へ移った直後に失敗すると、ユーザーには
   「エンジン起動ログの後、何も起きない」ようにしか見えない（報告と完全に一致）。
3. **起動タイムアウト 30 秒**（`crates/s2v-engines/src/process.rs` の `POLL_RETRIES=30`×1秒）は
   AivisSpeech の初回モデルロードでは不足しうる。タイムアウトすると 2. により無音で終わる。
4. （既知の残課題）🎛 押下時に別の行のプレビューが実行中だと自動プレビューが黙ってスキップされる。

**調査中に実装済み（コミット 4fa1cf2）:** 試聴経路のログ（合成開始/合成完了/準備完了/再生開始/
試聴失敗/再生失敗）、`S2V_GUI_DEBUG`（stderr ミラー）、`S2V_GUI_PROBE`（起動時自動試聴）、
ヘッドレスプローブ example。

---

### Task 1: 再生バーに進行状況と直近エラーを常時表示（タブ非依存）

**Files:**
- Modify: `crates/s2v-gui/src/tab_script.rs`（`preview_pending` フィールド追加）
- Modify: `crates/s2v-gui/src/app.rs`（pending の設定/解除、transport パネルでの表示）

- [ ] **Step 1: ScriptTab に試聴中の行番号を追加**

`ScriptTab` に `pub preview_pending: Option<usize>,` を追加（`Default` に `preview_pending: None,`）。
`ui()` の「▶ この行を試聴」クリック処理に `self.preview_pending = Some(line.no);` を追加:

```rust
                    if ui
                        .add_enabled(!busy, egui::Button::new("▶ この行を試聴"))
                        .clicked()
                    {
                        self.preview_error = None;
                        self.preview_pending = Some(line.no);
                        jobs.preview(line.clone());
                    }
```

- [ ] **Step 2: app.rs — pending の設定・解除と表示**

1. `OpenLab` の自動プレビュー分岐（`jobs.preview(line);` の直前）に
   `self.script.preview_pending = Some(line_no);` を追加。
2. probe 分岐の `jobs.preview(line);` の直前にも `self.script.preview_pending = Some(line.no);` を追加。
3. `pump_messages` の `PreviewReady`/`PreviewFailed` 両アームの先頭に
   `self.script.preview_pending = None;` を追加。
4. transport パネルを拡張（`self.transport.ui(...)` の**前**に1行追加）:

```rust
        egui::TopBottomPanel::bottom("transport").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(no) = self.script.preview_pending {
                    ui.spinner();
                    ui.label(format!("行{no} を合成中…（エンジン初回起動時は30秒以上かかります）"));
                }
                if let Some(e) = &self.script.preview_error {
                    ui.colored_label(egui::Color32::RED, format!("⚠ 試聴失敗: {e}"));
                }
                if let Some(e) = &self.lab.error {
                    ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
                }
            });
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

（タブ1内の既存エラーバナーはそのまま残す。二重表示になるが、可視性優先）

- [ ] **Step 3: ビルド・テスト・確認**

Run: `cargo test -p s2v-gui` → 全 PASS。
Run: `$env:S2V_GUI_PROBE='1'; cargo run -p s2v-gui` → 起動直後の自動試聴中、再生バーに
スピナー＋「行1 を合成中…」が表示され、再生開始で消えることを目視。

- [ ] **Step 4: コミット**

```bash
git add crates/s2v-gui/src/tab_script.rs crates/s2v-gui/src/app.rs
git commit -m "feat(gui): show preview progress and errors in transport bar (tab-independent)"
```

---

### Task 2: エンジン起動タイムアウトを設定可能にし既定を 60 秒へ

**Files:**
- Modify: `crates/s2v-core/src/config.rs`（EngineConfig に `startup_timeout_s`）
- Modify: `crates/s2v-engines/src/process.rs`（タイムアウトのパラメータ化）
- Modify: `crates/s2v-engines/src/http_engine.rs` / `xtts_engine.rs`（受け渡し）
- Modify: `src/lib.rs`（build_engine_manager で config 値を渡す）
- Modify: `config.toml`（コメントで案内追加）

- [ ] **Step 1: 失敗するテストを書く**（s2v-core config）

config.rs の tests に追加:

```rust
    #[test]
    fn engine_startup_timeout_parses_and_defaults() {
        let cfg: Config = toml::from_str(SAMPLE_TOML_WITH_TIMEOUT).unwrap(); // 既存サンプルに startup_timeout_s = 90 を足したもの
        assert_eq!(cfg.voicevox.startup_timeout_s, Some(90));
        let cfg2: Config = toml::from_str(SAMPLE_TOML).unwrap(); // 既存サンプル
        assert_eq!(cfg2.voicevox.startup_timeout_s, None);
    }
```

（SAMPLE_TOML は既存テストの定数名に合わせること。無ければ既存テストで使っている toml 文字列を再利用）

- [ ] **Step 2: 実装**

1. `EngineConfig`（[voicevox]/[aivis]/[xtts] の struct）に
   `#[serde(default)] pub startup_timeout_s: Option<u64>,` を追加。
2. `process.rs`: `const POLL_RETRIES: usize = 30;` を既定値関数に変更し、
   `ensure_running` に `timeout: Duration` 引数を追加（ポーリング回数 = `timeout.as_secs()`、
   1秒間隔は不変）。**既定 60 秒**（`pub const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(60);`）。
   既存テストは明示的に短いタイムアウトを渡す形に更新。
3. `HttpEngine`/`XttsEngine`: `with_exe_path` に `startup_timeout: Duration` 引数を追加
   （または `with_startup_timeout` ビルダー。呼び出し箇所は build_engine_manager のみなので引数追加が簡潔）。
4. `src/lib.rs build_engine_manager`:

```rust
    let vv_timeout = std::time::Duration::from_secs(config.voicevox.startup_timeout_s.unwrap_or(60));
```

を各エンジンで計算して渡す。
5. `config.toml` の先頭コメントに
   `# startup_timeout_s: 自動起動の待機秒数（省略時 60。モデルロードが遅い場合は増やす）` を追記。

- [ ] **Step 3: テスト**

Run: `cargo test --workspace` → 全 PASS（process.rs の既存6テスト・E2E 含む）

- [ ] **Step 4: コミット**

```bash
git add crates/s2v-core/src/config.rs crates/s2v-engines/src crates/s2v-gui src/lib.rs config.toml
git commit -m "feat(engines): configurable startup timeout, default raised 30s -> 60s"
```

---

### Task 3: 🎛 押下時の取りこぼし解消（実行中なら完了後に自動継続）

**Files:**
- Modify: `crates/s2v-gui/src/app.rs`

- [ ] **Step 1: 実装**

`App` に `pending_lab_line: Option<usize>,` を追加（new で None）。`OpenLab` 処理を変更:

```rust
                                crate::tab_script::ScriptAction::OpenLab { line_no } => {
                                    self.tab = Tab::Lab;
                                    self.lab.source = crate::tab_lab::LabSource::ScriptLine;
                                    let has_raw = self
                                        .script
                                        .preview_raw
                                        .as_ref()
                                        .map(|(no, _)| *no == line_no)
                                        .unwrap_or(false);
                                    if !has_raw {
                                        self.pending_lab_line = Some(line_no);
                                        // 空いていれば即時、実行中なら PreviewReady/Failed 後に pump が継続実行する
                                        self.try_dispatch_pending_preview();
                                    }
                                }
```

ヘルパと pump 処理を追加:

```rust
    /// pending_lab_line の行のプレビューを、busy でなければ起動する。
    fn try_dispatch_pending_preview(&mut self) {
        let Some(line_no) = self.pending_lab_line else { return };
        let Some(jobs) = &self.jobs else { return };
        if jobs.busy_preview.load(std::sync::atomic::Ordering::SeqCst) {
            return; // 完了時に pump_messages から再試行される
        }
        if let Some(line) = self
            .script
            .model
            .as_ref()
            .and_then(|m| m.lines.iter().find(|l| l.no == line_no))
            .cloned()
        {
            self.script.preview_error = None;
            self.script.preview_pending = Some(line.no);
            jobs.preview(line);
        } else {
            self.pending_lab_line = None; // 行が消えた（再読込等）
        }
    }
```

`pump_messages` の `PreviewReady { line_no, .. }` アーム末尾に:

```rust
                    if self.pending_lab_line == Some(line_no) {
                        self.pending_lab_line = None;
                    } else {
                        self.try_dispatch_pending_preview();
                    }
```

`PreviewFailed { line_no, .. }` アーム末尾に（失敗した行が pending 自身なら諦める）:

```rust
                    if self.pending_lab_line == Some(line_no) {
                        self.pending_lab_line = None;
                    } else {
                        self.try_dispatch_pending_preview();
                    }
```

注: `PreviewFailed` の `line_no` フィールドをここで初めて使用する（既存の dead_code 警告が1件解消）。
borrow エラーが出る場合（pump 内で `&self.jobs` 保持中の `try_dispatch_pending_preview` 呼び出し等）は、
msgs 処理後にフラグを立てて pump の外で呼ぶ形に**意味を変えず**修正し報告。

- [ ] **Step 2: テスト・確認**

Run: `cargo test -p s2v-gui` → 全 PASS。
Run: `cargo build -p s2v-gui 2>&1` → 警告が `script_model.rs scenes` の1件のみになること。

- [ ] **Step 3: コミット**

```bash
git add crates/s2v-gui/src/app.rs
git commit -m "feat(gui): queue open-in-lab preview while another preview is running"
```

---

### Task 4: タブ2の音源表示に「合成中」を反映＋マニュアル追記

**Files:**
- Modify: `crates/s2v-gui/src/tab_lab.rs`（`ui()` に `preview_pending: Option<usize>` 引数追加）
- Modify: `crates/s2v-gui/src/app.rs`（呼び出し）
- Modify: `docs/manual.html`（トラブルシューティングに1行）

- [ ] **Step 1: tab_lab の音源ラジオ表示**

`ui()` に引数 `preview_pending: Option<usize>` を追加し、音源ラジオのラベル決定を:

```rust
                        match (preview_raw, preview_pending) {
                            (_, Some(no)) => format!("台本の行 {no}（合成中…）"),
                            (Some((no, _)), None) => format!("台本の行 {no}（試聴済み）"),
                            (None, None) => "台本の行（先にタブ1で試聴）".to_string(),
                        },
```

に変更（`has_line` の enable 条件は従来どおり `preview_raw.is_some()`）。
app.rs の呼び出しに `self.script.preview_pending` を渡す。

- [ ] **Step 2: manual.html トラブルシューティング表に行を追加**

「6. トラブルシューティング」の表（出力ロックの行の後）に:

```html
  <tr>
    <td>GUI で「▶試聴」を押しても音が出ない・反応がないように見える</td>
    <td>エンジンの初回起動（特に AivisSpeech のモデルロード）と合成で<strong>数十秒</strong>かかることがあります。
    画面下部の再生バーに「行N を合成中…」と表示されている間はお待ちください。
    起動が遅い環境では config.toml の各エンジンに <code>startup_timeout_s = 120</code> のように待機秒数を指定できます。
    失敗した場合は再生バーとログ（フッター）に理由が表示されます。</td>
  </tr>
```

- [ ] **Step 3: 最終確認・コミット**

Run: `cargo test --workspace` → 全 PASS。`cargo build --release -p s2v-gui` → 成功。
`S2V_GUI_PROBE=1` でリリース exe を起動し、再生バーの進行表示→再生開始の流れを確認。

```bash
git add crates/s2v-gui/src/tab_lab.rs crates/s2v-gui/src/app.rs docs/manual.html
git commit -m "feat(gui): show synthesizing state in lab source label; document preview wait time"
```
