# Script2Voice-Rust版 — Claude Code project-local rules

作業前に必ず `D:\UDS\AGENT_WORKFLOW.md` を読む。このファイルは共通規程を緩和せず、Script2Voice-Rust版固有の差分だけを定める。

## Project type and priority

Script2Voice-Rust版は Development Project である。規則の優先順位は次のとおりとする。

1. Explicit Human instruction
2. Safety and irreversible-operation constraints
3. `D:\UDS\AGENT_WORKFLOW.md`
4. このproject-local rulesの、共通規程より厳しい差分
5. Current Beads issue
6. Git / Obsidianの各Source of Truth
7. Memory / conversation history

## Script2Voice-Rust版 Source of Truth

- **Git**: Rust sourceとtests（`src/`、`crates/`）、Cargo workspace（`Cargo.toml` / `Cargo.lock`）、`config.toml`、`台本仕様.txt`、CLI / GUI / export仕様、コード同期型の`docs/superpowers/specs/`・`docs/superpowers/plans/`・`docs/manual.html`。
- **Obsidian**: `D:\ObsidianVault\ClaudeMemory\Projects\Script2Voice-Rust版\`。`失敗・教訓ログ.md`は長期的なLessons Learned、`音声合成エンジン候補_ライセンス調査.md`はResearch、`進捗.md`はProject OverviewとLegacy work-stateが混在する参照先である。新規task状態をObsidianへ複製しない。
- **Beads**: `.beads`を持つため、new workのcurrent work state、dependency、assignment、review、verificationはBeadsを正本とする。既存Issue・label・memoryはLegacyとして扱い、共通規程に従って新規情報から運用する。
- **Memory**: Claude Memoryやconversation historyは補助情報であり、現在の仕様・判断・task状態の根拠にはしない。

## Build and test

通常の検証コマンドは次のとおり。

```bash
cargo fmt --check
cargo test --workspace
cargo build --workspace
```

GUIに影響する変更では、追加で次を実行する。

```bash
cargo build -p s2v-gui
```

## Claude Code tool and hook safety

Claude Codeのpermission allowlistは操作権限であり、判断権限・close権限・Human approvalを意味しない。`.claude/settings.json`のSessionStart hookは`bd prime --hook-json`を呼ぶが、hookはcontext refreshの補助であり、権限・承認・Source of Truthを置き換えない。hookの手動実行・設定変更は明示的な依頼がある場合だけ行う。

## Project-specific safety and risk rules

- 音声DSP pipelineの順序または既定音響パラメータ、`台本仕様.txt` / parser syntax、`config.toml`形式、public CLI、VOICEVOX / AivisSpeech / XTTS engine interface、SRT / timeline JSON / FCPXML /音声出力形式を実質的に変える変更は **High Risk** とする。
- ffmpeg / ffprobe、外部音声エンジンの起動・HTTP通信、出力ファイル・lock file、bulk processingを扱う前に、実行対象・入出力path・上書き/削除の有無を確認する。既存成果物の上書き・削除、大量処理、外部操作は共通規程のHuman Decision Boundaryに従う。
- release、Program Filesへの配布・更新、外部公開/upload、破壊的な台本・project data migrationはHuman approvalなしに行わない。
- `unsafe`、Windows Job Object/FFI、engine process cleanupを変更する場合はHigh Riskとして扱う。
- Git commit、git push、BeadsのDolt remote syncは、明示的なHuman instructionなしに行わない。

共通Role、Risk、Beads一般規則、Obsidian一般規則、Human approval一般規則の全文は、ここに再掲しない。共通規程を参照すること。
