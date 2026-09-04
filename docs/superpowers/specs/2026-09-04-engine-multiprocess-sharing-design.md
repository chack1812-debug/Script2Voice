# エンジンプロセスのマルチプロセス共有 設計書

- 日付: 2026-09-04
- 対象クレート: `crates/s2v-engines`
- 発端: `script2voice_engine_multiprocess_handoff.md`（2026-09-04 引き継ぎ書）

## 1. 背景と根本原因

Script2Voice を複数プロセス同時に実行すると、実行途中で音声合成が連続的に失敗する。

### 引き継ぎ書の推測と実態の差

引き継ぎ書は「Script2Voice が起動のたびに毎回 engine.exe を spawn しており、ポート競合で
後発プロセスが起動に失敗している」と推測していた。しかし調査の結果、引き継ぎ書が提案していた
対策の大部分は既に実装済みだった。

| 引き継ぎ書の提案 | 実装状況 |
| --- | --- |
| `/version` ヘルスチェック → 起動済みなら使い回し、無ければ spawn ＋ ポーリング | 実装済み（`process.rs: ensure_running`、s2v-9vr） |
| 起動時の排他制御（named mutex / ロックファイル） | **未実装** |
| Job Object `KILL_ON_JOB_CLOSE` | 実装済み（`job.rs: EngineJob`、s2v-48o）。ただし**無名 Job をプロセスごとに新規作成** |
| 全プロセスが同じ名前付き Job のハンドルを保持し、最後の1つの終了で殺す | **未実装** |

### 実ログによる根本原因の特定

`run.log` に決定的な証拠がある。

```
小説/…/台本_第10話          18:10:51 開始
  18:12:24 まで正常 → 18:12:25〜18:12:33 の12行が
    ERROR 合成失敗 …: error sending request for url (http://127.0.0.1:50021/audio_query?…)
  → 18:12:44 から復旧（別プロセスがエンジンを再起動したため）

週刊AIデスク/20260905      18:20:16 開始
  18:23:27 以降が総崩れ → ユーザーが 18:24:25 に再実行
```

`error sending request` は接続そのものの切断であり、症状は「起動直後の失敗」ではなく
**実行途中でのエンジン消失**である。すなわち:

> 先に終了した Script2Voice プロセスの `terminate_process` が `job.terminate()`
> （`crates/s2v-engines/src/process.rs:96`）を呼び、**他プロセスが使用中の共有エンジンを
> 問答無用で終了させている**。

ポート競合は主因ではない。後発プロセスは `is_alive()` で既存エンジンを検知して spawn しないため、
競合は原理的に起きにくい。

### 重大度

失敗した行の音声ファイルはそのまま欠落し、処理は継続する（上記の例で12行・24行）。
欠落に気づかないまま動画合成へ進むリスクがあるため、「不要なエラーが出るだけ」ではなく
成果物の欠損を伴う不具合である。

## 2. 目的と非目的

### 目的

1. Script2Voice を複数プロセス同時実行しても、エンジンが他プロセスに殺されないようにする。
2. 「Script2Voice の終了時にエンジンも確実に終了させる」という既存要件を維持する。
   ここでの「終了時」は**そのエンジンを使っている最後のプロセスが終了したとき**を指す。
3. 起動時のレースによる二重 spawn を防ぐ。

### 非目的（今回のスコープ外）

- 合成失敗行のリトライ・欠落検知（既存 bead `s2v-cgj` のまま別件とする）。
- OpenJTalk ユーザー辞書ロック問題（voicevox_engine issue #1347）への対応。
  二重 spawn が排他制御で消えれば辞書コンパイルの競合も起きなくなるため、エンジンの
  バージョン確認は不要と判断する。もし残存すれば別途 bead 化する。
- 複数ユーザーセッション／サービス跨ぎでのエンジン共有（`Global\` 名前空間）。

## 3. アプローチの選択

| | A. 名前付き Job Object | B. ロックファイル＋PID 参照カウント | C. spawn 元だけが殺す（現状踏襲） |
| --- | --- | --- | --- |
| 仕組み | 全プロセスが同名 Job のハンドルを保持、最後の1つの終了で OS が殺す | PID レジストリファイルを増減させ 0 になったら殺す | 「自分が spawn した」場合のみ殺す |
| クラッシュ耐性 | OS のハンドル参照カウント。強制終了でも確実 | クリーンアップが走らず孤児化する | spawn 元がクラッシュすれば残留する |
| 今回のバグ | 直る | 直る | **直らない**（これが現状） |
| 追加コード | 小（無名→名前付き、明示 terminate の廃止） | 大（ファイルロック＋PID 生存確認） | ゼロ |

**採用は A**。既に `job.rs` が動作しており、変更が最小でリスクが低い。B は引き継ぎ書が既に却下している。

## 4. 設計

### 4.1 エンジン生存期間の共有（名前付き Job Object）

Job はエンジンごとに1つ。名前は `Local\Script2Voice_Engine_<エンジン名>_<ポート>` とする。
`voicevox` / `aivis` / `xtts` の区別に加え、同名エンジンをポート違いで使う構成でも衝突しない。
ポートは各エンジンが保持する `url`（例: `http://127.0.0.1:50021`）から抽出する。
ポートを抽出できない場合は `url` 全体を英数字以外を `_` に置換した文字列を用いる。

**ハンドルを開くタイミングは「activate 時」とする。** 引き継ぎ書は「全 Script2Voice プロセスが
起動時に開く」としていたが、それだと*そのエンジンを使わないプロセス*が生きている間もエンジンが
残ってしまう。`ensure_running` の中で、既存エンジンを検知した場合も自分で spawn した場合も
必ず同名 Job を開いて保持すれば、「そのエンジンを実際に使っている最後のプロセスが終わった瞬間に
落ちる」という正しい意味になる。

`ensure_running` の変更:

1. 名前付き Job をオープンする。新規作成時（`GetLastError() != ERROR_ALREADY_EXISTS`）のみ
   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` を設定する。既存 Job なら設定済みのためそのまま使う。
2. `is_alive()` が true なら、**Job ハンドルだけ保持して return する**
   （現在は何も保持せず return している）。
3. false なら spawn し、`AssignProcessToJobObject` で Job に割り当て、`/version` をポーリングする。

`terminate_process` の変更:

- **`job.terminate()` と `child.kill()` を削除する。** `EngineProcess` を drop して
  ハンドルを閉じるだけにする。自分が最後のハンドル保持者なら、その瞬間に OS がエンジンツリーごと
  終了させる。他プロセスが使用中なら残る。これが根本原因の直接の修正である。

副作用として、単独実行時にエンジンが実際に落ちるのは「Script2Voice プロセスが完全に終了した
瞬間」になり、現状の `shutdown_all` 時点から数ミリ秒ずれる。`shutdown_all` の呼び出しは
`src/main.rs:211`（Ctrl+C 中断時）、`src/main.rs:216`（バッチ完了時）、
`crates/s2v-gui/src/jobs.rs:88`（GUI の `on_exit`）のいずれもプロセス終了直前であり、
「停止した後に同じプロセスでエンジンを再利用する」経路は存在しないため実害はない。

ユーザーが手動で起動した VOICEVOX / AivisSpeech は Job に属さないため、従来どおり終了対象外である。

### 4.2 起動レースの排他制御（ロックファイル）

引き継ぎ書は named mutex を提案しているが、**Windows の Mutex はスレッドアフィニティを持つ**
（`ReleaseMutex` は取得したスレッドから呼ぶ必要がある）。一方 `ensure_running` の臨界区間は
`is_alive().await` と最大60秒のポーリング `.await` をまたぐ。tokio のマルチスレッドランタイムでは
`.await` 前後でワーカースレッドが移動しうるため、named mutex は誤用になる。

代わりに**ロックファイルの排他オープン**を使う（引き継ぎ書が併記していたロックファイル案の、
依存を増やさない実装）。

```rust
use std::os::windows::fs::OpenOptionsExt;
// share_mode(0) = FILE_SHARE_NONE。
// 他プロセスが開いている間は ERROR_SHARING_VIOLATION で失敗する。
OpenOptions::new().create(true).write(true).share_mode(0).open(&lock_path)
```

- **スレッドアフィニティなし** — `.await` をまたいで保持しても安全。
- **クラッシュ耐性あり** — プロセスが落ちれば OS がハンドルを閉じ、ロックは自動解放される。
  `fs2` / `fs4` の追加も、`CREATE_NEW` 方式のような残骸ファイル問題も不要。
- **依存追加ゼロ** — `std::os::windows::fs::OpenOptionsExt` のみ。

取得は `tokio::time::sleep(100ms)` を挟む非ブロッキングのリトライループとする。
ロックファイルは `%TEMP%\script2voice_engine_<エンジン名>_<ポート>.lock`、Job 名と同じ命名規則で揃える。

**臨界区間は `ensure_running` の本体全体**（`is_alive` チェック → spawn → 起動完了ポーリング）とする。
これを覆わないと「P1 が spawn した直後、まだ `/version` が応答しない隙に P2 が来て二重 spawn する」
のを防げない。

ロックを取れないまま `startup_timeout` を超えた場合は、**警告ログを出して従来どおりの経路に
フォールスルーする**。最悪でも現状の挙動に戻るだけで、待ち続けてハングするより安全である。

名前付き Job のハンドルとロックの寿命は異なる点に注意する。**ロックは起動処理の間だけ、
Job ハンドルはプロセス終了まで**保持する。

### 4.3 名前空間

同一ユーザーセッション内での複数起動を想定し、`Local\` を採用する。

## 5. モジュール構成

| ファイル | 変更内容 |
| --- | --- |
| `crates/s2v-engines/src/job.rs` | `EngineJob::new(name)` を名前付き化。`terminate()` を廃止 |
| `crates/s2v-engines/src/lock.rs` | 新規。ロックファイルの取得・解放（RAII ガード） |
| `crates/s2v-engines/src/process.rs` | `ensure_running` / `terminate_process` の変更 |
| `crates/s2v-engines/src/http_engine.rs` | Job 名・ロック名の材料（エンジン名＋ポート）を渡す |
| `crates/s2v-engines/src/xtts_engine.rs` | 同上。同じ `ensure_running` 経由のため自動的に対象 |
| `src/main.rs`, `crates/s2v-gui` | 変更なし |

## 6. テスト戦略

TDD で進める。名前付き Job は「同一プロセス内で同名 Job を2回開けば参照カウントが2になる」ため、
クロスプロセスの意味論をそのままインプロセスのテストで検証できる。

### 新規テスト

1. **回帰テストの本命 — 共有中のエンジンは殺されない**
   同名 Job のハンドルを2つ開き、ダミープロセスを Job に割り当てる。1つ目を drop しても
   プロセスが生きていること、2つ目を drop した瞬間に死ぬことを検証する。
   これが `process.rs:96` のバグに直接対応する回帰テストである。
2. **二重 spawn 防止** — 同じロック名で `ensure_running` を2つ並行実行し、spawn が1回しか
   起きないことを検証する（既存の `write_marker_script` パターンを流用）。
3. **ロック競合時のフォールスルー** — ロックファイルを外部から掴んだ状態で、`is_alive` が
   true なら警告を出しつつ正常完了すること。フォールスルーは `startup_timeout` 経過後に
   起きるため、テストでは短い `timeout`（例: 1秒）を渡してテスト時間を抑える。
4. **ロックのクラッシュ自動解放** — ロックを保持した `File` を drop すれば即座に再取得できること。

### 既存テストへの影響

- `terminate_process_kills_grandchild_processes_via_job_object` はそのまま通る見込み。
  `terminate_process` は `guard.take()` した `EngineProcess` をスコープ末尾で drop し、
  テストプロセスが唯一のハンドル保持者なので `KILL_ON_JOB_CLOSE` が発火する。
  ただし検証している機構が「`TerminateJobObject` で殺す」から「ハンドルクローズで殺す」へ
  変わるため、テスト名とコメントを実態に合わせて更新する。
- **Job 名はテストごとに一意にする。** Rust のテストは同一プロセス内のスレッドで並列実行される
  ため、名前が固定だと別テスト同士で Job を共有してしまう。
  テストは `format!("Local\\s2v_test_{}", 一意ID)` を渡す。

## 7. 検証

- `cargo test --workspace --all-targets` がグリーンであること。
- 実機で2本の台本を同時に実行し、`run.log` に `error sending request` が出ないこと。
- 全プロセス終了後に engine.exe が残っていないこと（タスクマネージャで確認）。

## 参考資料

- [VOICEVOX ENGINE（API）のスループット検証 - Qiita](https://qiita.com/uezo/items/7e476147ec6312ad8a2c)
- [windowsでエンジンの多重起動を可能にする · Issue #1347 · VOICEVOX/voicevox_engine](https://github.com/VOICEVOX/voicevox_engine/issues/1347)
- [AivisSpeech-Engine README](https://github.com/Aivis-Project/AivisSpeech-Engine/blob/master/README.md)
- 既存設計: `docs/superpowers/specs/2026-06-07-engine-process-cleanup-design.md`
