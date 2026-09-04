---
title: Script2Voice — VOICEVOX/AivisSpeechエンジン複数起動対応 引き継ぎ書
date: 2026-09-04
対象: Claude Code（Script2Voice-Rust版 実装担当）
プロジェクト: D:\UDS\Script2Voice-Rust版
---

# 背景・目的

Script2Voice（Rust製TTS自動化パイプライン。VOICEVOX / AivisSpeech / XTTS をカセット式で切り替えるモジュラー設計）を複数プロセス同時に起動すると、後から起動したプロセスがエラーを起こす。

現状は「先行プロセスが起因で後発がエラーになるだけ」で実用上致命的ではないが、不要なエラーを出さずに複数起動できるようにしたい。加えて、**Script2Voice終了時にはエンジン（engine.exe）も確実に終了させたい**という要件がある。

対象エンジンは VOICEVOX Engine および AivisSpeech Engine（AivisSpeech EngineはVOICEVOX Engineをベースにした派生）。いずれもローカルHTTPサーバーとして動作し、`/version` 等のエンドポイントを持つ。XTTSカセットは今回の調査対象外であり、同じ挙動が当てはまるかは未検証（別途確認要）。

# 判明している事実（調査済み）

## 1. エンジンは並列合成に対応していない

VOICEVOX ENGINEは単一プロセスに対して複数の同時HTTPリクエストを受け付けはするが、内部処理は事実上シーケンシャル（直列）。10並列でリクエストしても10個を順番に投げた場合とほぼ同じ総所要時間になることがベンチマークで確認されている。

- 参考: [VOICEVOX ENGINE（API）のスループット検証 - Qiita](https://qiita.com/uezo/items/7e476147ec6312ad8a2c)

→ **複数のエンジンプロセスを並行起動しても速度面のメリットはほぼない**。むしろCPU/GPU(VRAM)を余分に消費するだけ。設計方針としては「エンジンは1プロセスだけ常駐させ、全Script2Voiceプロセスで共有する」方向が良い。

## 2. 複数エンジンプロセスの同時起動には既知の不具合がある

Windows環境でVOICEVOXエンジンを複数プロセス同時起動すると、OpenJTalkのユーザー辞書コンパイル処理（`pyopenjtalk.set_user_dict`）がファイルハンドルを掴んだままになり、2つ目以降のプロセスがエラーになる不具合が報告されている。

- 参考: [windowsでエンジンの多重起動を可能にする（openjtalkによるエラーを出なくする）· Issue #1347 · VOICEVOX/voicevox_engine](https://github.com/VOICEVOX/voicevox_engine/issues/1347)
- PR #1514で対応されたとみられるが、詳細な実装内容とAivisSpeech Engineへの反映状況は未確認。**使用中のエンジンバージョンが対応済みか要確認**。

また、単純にデフォルトポート固定（VOICEVOX: 50021 / AivisSpeech: 10101）で複数プロセスが同時にbindしようとすると、2つ目以降が起動失敗する（ポート競合）。これが現状Script2Voiceで起きているエラーの主因と推測される（Script2Voiceが起動のたびに毎回engine.exeをspawnしている場合）。

- AivisSpeech Engineの`--port`オプションでポート変更可能。個別ポートにすれば競合は避けられるが、上記1の理由から推奨しない。
- 参考: [AivisSpeech-Engine README](https://github.com/Aivis-Project/AivisSpeech-Engine/blob/master/README.md)

# 推奨アーキテクチャ

方針: **エンジンは1プロセスだけを共有し、Script2Voiceの各プロセスはそれを検知して使い回す。エンジンの起動と終了はOSレベルの機構で確実に制御する。**

## A. 起動時: 既存エンジンの検知＋排他制御

1. Script2Voiceがエンジンを使う直前に、対象ポート（VOICEVOX: 50021 / AivisSpeech: 10101、カセットにより異なる）へ `GET /version` を軽くリクエストする。
2. 200が返れば「既に起動済み」と判断し、自分ではspawnせずそのエンジンをそのまま使う。
3. レスポンスがなければ自分でengine.exeをspawnし、`/version`が応答するまでポーリングして起動完了を待つ。

**注意（レースコンディション）**: 「チェック→起動」の間に複数プロセスがほぼ同時に走ると、両方とも「未起動」と誤判定して二重起動してしまう可能性がある。この一連の処理をnamed mutex（もしくはロックファイル＋`fs2`の`try_lock_exclusive`）で排他制御し、ロックを取れたプロセスだけがチェック・起動判断を行うようにする。

## B. 終了時: Job Objectによる確実なエンジン終了（Windows）

ユーザー要件により「Script2Voice終了時にはエンジンも確実に終了させたい」を満たす必要がある。ユーザーランドの参照カウント（PIDレジストリ等）方式は、全プロセスがクラッシュ/強制終了した場合にクリーンアップコードが一切走らず、エンジンが孤児化するリスクが残るため不採用とする。

代わりに **Windows Job Object（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`）** を使う。カーネルのハンドル参照カウントに基づく仕組みで、アプリ側の終了処理コードが実行されなくても（クラッシュ・強制終了含むあらゆる終了パターンで）OSが自動的にengine.exeを道連れ終了させる。

### 仕組み

1. 名前付きJob Object（例: `Local\Script2Voice_EngineJob`）を作成し、`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`フラグを設定する。
2. engine.exeを実際にspawnしたプロセスが、そのengine.exeプロセスを`AssignProcessToJobObject`でJobに所属させる。
3. **Script2Voiceの全プロセス**（engineを起動した本人かどうかに関わらず、共有エンジンを使う全プロセス）が起動時に同じ名前のJob Objectをオープンし、そのハンドルをプロセス終了までずっと保持する（明示的にクローズしない）。
4. Script2Voiceプロセスが終了すると（正常終了・クラッシュ・強制終了いずれでも）、OSがそのプロセスの全ハンドルを自動的にクローズする→Job Objectへの参照が1つ減る。
5. 全Script2Voiceプロセス分のハンドルがすべて閉じられた瞬間（＝最後の1プロセスが終了した瞬間）、`KILL_ON_JOB_CLOSE`によりengine.exeがカーネルにより即座に強制終了される。

### 実装イメージ（`windows` crate）

```rust
use windows::Win32::Foundation::*;
use windows::Win32::System::JobObjects::*;
use windows::core::PCWSTR;

/// プロセス起動時に一度呼び、返ったHANDLEはプロセス終了までstatic等で保持し続ける。
/// 明示的にCloseHandleしないこと（呼ぶとその時点で参照が減ってしまう）。
fn open_or_create_engine_job() -> HANDLE {
    let name: Vec<u16> = "Local\\Script2Voice_EngineJob\0".encode_utf16().collect();
    unsafe {
        let h = CreateJobObjectW(None, PCWSTR(name.as_ptr()))
            .expect("CreateJobObjectW failed");

        // 新規作成時だけ KILL_ON_JOB_CLOSE を設定（既存Jobなら設定済みのはず）
        if GetLastError() != ERROR_ALREADY_EXISTS {
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ).expect("SetInformationJobObject failed");
        }
        h
    }
}

// engine.exeを自分が起動した場合のみ呼ぶ
fn assign_engine_to_job(job: HANDLE, engine_child: &std::process::Child) -> windows::core::Result<()> {
    use std::os::windows::io::AsRawHandle;
    let h_engine = HANDLE(engine_child.as_raw_handle() as isize);
    unsafe { AssignProcessToJobObject(job, h_engine) }
}
```

A（起動時mutex）とB（終了時Job Object）は役割が異なるため併用する。mutexは「重複起動の防止」、Job Objectは「最後の1プロセス終了時にengineも確実に終了させる」ための機構。

# 未確定事項・要確認事項（Claude Codeでの実装時に検討）

- 使用中のVOICEVOX/AivisSpeech Engineのバージョンが、OpenJTalk辞書ロック問題（issue #1347 / PR #1514）に対応済みかどうかの確認。未対応バージョンの場合、辞書コンパイルパスの競合が別途起きうる。
- named mutex / Job Objectの名前空間（`Local\` vs `Global\`）は、同一ユーザーセッション内でのみ複数起動する想定なら`Local\`で問題ないはず。マルチユーザー/サービス跨ぎで動かす可能性があるなら`Global\`への変更を検討。
- カセット（VOICEVOX / AivisSpeech / XTTS）ごとにポート番号・プロセス名が異なるため、Job Object名・mutex名・ヘルスチェック先ポートはカセット種別ごとに分離する必要がある（例: `Local\Script2Voice_EngineJob_VOICEVOX`、`Local\Script2Voice_EngineJob_AivisSpeech`）。
- XTTSカセットについては、そもそもエンジンが同種のHTTPサーバー方式か、プロセスモデルが異なるかを別途確認し、同じ設計を適用できるか判断する。
- エンジン起動完了待ち（`/version`ポーリング）のタイムアウト値、リトライ回数などの具体的なパラメータ設計。
- 既存のScript2Voiceのエンジン起動処理（spawn箇所）のコード確認と、上記A/Bの組み込み方針のすり合わせ。

# 参考資料

- [VOICEVOX ENGINE（API）のスループット検証 - Qiita](https://qiita.com/uezo/items/7e476147ec6312ad8a2c)
- [windowsでエンジンの多重起動を可能にする（openjtalkによるエラーを出なくする）· Issue #1347 · VOICEVOX/voicevox_engine](https://github.com/VOICEVOX/voicevox_engine/issues/1347)
- [AivisSpeech-Engine README](https://github.com/Aivis-Project/AivisSpeech-Engine/blob/master/README.md)
- [音声合成の並列化は可能ですか？ · Issue #513 · VOICEVOX/voicevox_core](https://github.com/VOICEVOX/voicevox_core/issues/513)
