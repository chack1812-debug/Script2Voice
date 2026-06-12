use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use clap::Parser;
use s2v_core::{Config, Scene, ScriptParser};
use s2v_engines::EngineManager;
use script2voice::{build_engine_manager, resolve_config_path, Producer};
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

/// パース済み台本 1 件: (台本パス, シーン列)。
type ParsedScript = (PathBuf, Vec<Scene>);
/// 失敗した台本 1 件: (台本パス, 理由)。
type ScriptFailure = (PathBuf, String);

#[derive(Parser)]
#[command(name = "script2voice", version, about = "台本から音声・字幕・タイムラインを生成する")]
struct Cli {
    /// 台本ファイルまたはフォルダ（複数指定可。フォルダは直下の .txt を名前順に処理）
    #[arg(required = true, num_args = 1..)]
    scripts: Vec<PathBuf>,

    /// 設定ファイル (config.toml) のパス。省略時は実行ファイルと同じディレクトリの config.toml を使用する
    #[arg(short, long)]
    config: Option<PathBuf>,
}

/// 現在処理中の台本の run.log を指す差し替え可能なファイルハンドル。
/// バッチでは subscriber を1つだけ使い回し、台本の境界でこのハンドルを差し替える。
#[derive(Clone, Default)]
struct SharedLogFile(Arc<Mutex<Option<std::fs::File>>>);

impl SharedLogFile {
    /// ファイル出力先を設定する（None でファイル出力を止める）。
    fn set(&self, file: Option<std::fs::File>) {
        *self.0.lock().unwrap() = file;
    }
}

/// `SharedLogFile` が現在指すファイルへ書き込む Writer。未設定時は破棄する。
struct SharedLogWriter(Arc<Mutex<Option<std::fs::File>>>);

impl std::io::Write for SharedLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut guard = self.0.lock().unwrap();
        match guard.as_mut() {
            Some(f) => f.write(buf),
            None => Ok(buf.len()), // 出力先未設定時は破棄
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let mut guard = self.0.lock().unwrap();
        match guard.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogFile {
    type Writer = SharedLogWriter;
    fn make_writer(&'a self) -> Self::Writer {
        SharedLogWriter(self.0.clone())
    }
}

/// `process_one` の間だけ `SharedLogFile` に出力先を束縛し、スコープを抜けるとき
/// （正常・エラー・パニックいずれでも）出力先を外す RAII ガード。
struct LogScope<'a>(&'a SharedLogFile);

impl Drop for LogScope<'_> {
    fn drop(&mut self) {
        self.0.set(None);
    }
}

/// コンソール + 差し替え可能ファイルの subscriber をプロセスで1回だけ初期化する。
/// 返した `SharedLogFile` を台本の境界で `set` してファイル出力先を切り替える。
fn init_logging() -> SharedLogFile {
    let shared = SharedLogFile::default();
    let time_format = "%Y-%m-%d %H:%M:%S%.3f".to_string();

    let console_layer = fmt::layer()
        .with_timer(ChronoLocal::new(time_format.clone()))
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(shared.clone())
        .with_timer(ChronoLocal::new(time_format))
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    shared
}

/// 台本の出力フォルダに run.log を追記オープンする。
fn open_run_log(project_dir: &Path) -> anyhow::Result<std::fs::File> {
    let path = project_dir.join("run.log");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("ログファイルを開けません: {}", path.display()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let log_file = init_logging();

    let scripts = expand_script_args(&cli.scripts)?;
    tracing::info!("処理対象: {} 台本", scripts.len());

    let exe_path = std::env::current_exe().ok();
    let config_path = resolve_config_path(cli.config.clone(), exe_path.as_deref());
    tracing::info!("設定ファイル: {}", config_path.display());
    let config = Config::from_file(&config_path)
        .with_context(|| format!("設定ファイルの読み込みに失敗しました: {}", config_path.display()))?;

    // 事前パース（失敗は継続）
    let (parsed, parse_failures) = parse_all(&scripts);

    // 必要エンジンを1回だけ起動（失敗は継続）
    let required = required_engines(&parsed);
    tracing::info!(
        "使用予定のエンジン: {}",
        required.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    let engine_manager = Arc::new(build_engine_manager(&config));
    activate_each(&engine_manager, &required).await;

    // 台本ごとに処理（暖まったエンジンを使い回し、失敗は継続）
    let config_ref = &config;
    let em_ref = &engine_manager;
    let log_ref = &log_file;
    let summary = run_each(parsed, parse_failures, |path, scenes| async move {
        process_one(&path, &scenes, config_ref, em_ref, log_ref).await
    })
    .await;

    // 全台本終了後にエンジンを停止
    engine_manager.shutdown_all();

    // サマリ
    tracing::info!(
        "=== バッチ完了: 成功 {} / 失敗 {} (合計 {}) ===",
        summary.succeeded,
        summary.failures.len(),
        summary.total()
    );
    for (path, reason) in &summary.failures {
        tracing::warn!("失敗: {} — {}", path.display(), reason);
    }
    if summary.has_failure() {
        anyhow::bail!("{} 台本が失敗しました", summary.failures.len());
    }
    Ok(())
}

/// 台本引数を展開する。
/// - ファイル: そのまま採用。
/// - ディレクトリ: 直下の拡張子 `.txt`（大文字小文字無視）を名前順に採用（再帰しない）。
/// - 存在しないパス: エラー。
///
/// 採用後に canonicalize した実体パスで重複を除去（出現順は維持）。0 件ならエラー。
fn expand_script_args(args: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    fn push_unique(p: &Path, out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) -> anyhow::Result<()> {
        let canon = std::fs::canonicalize(p)
            .with_context(|| format!("パスを解決できません: {}", p.display()))?;
        if seen.insert(canon.clone()) {
            out.push(canon);
        }
        Ok(())
    }

    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for arg in args {
        if arg.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(arg)
                .with_context(|| format!("フォルダを読めません: {}", arg.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("txt"))
                            .unwrap_or(false)
                })
                .collect();
            entries.sort();
            for p in entries {
                push_unique(&p, &mut out, &mut seen)?;
            }
        } else if arg.is_file() {
            push_unique(arg, &mut out, &mut seen)?;
        } else {
            anyhow::bail!("台本パスが見つかりません: {}", arg.display());
        }
    }

    if out.is_empty() {
        anyhow::bail!("処理対象の台本が見つかりません（.txt が 0 件）");
    }
    Ok(out)
}

/// パース済み全台本から、使用される engine_type の和集合を作る。
fn required_engines(parsed: &[ParsedScript]) -> HashSet<String> {
    let mut set = HashSet::new();
    for (_, scenes) in parsed {
        for scene in scenes {
            for cast in scene.casts.values() {
                set.insert(cast.engine_type.clone());
            }
        }
    }
    set
}

/// 各台本をパースする。失敗しても止めず、成功分と失敗分(パス, 理由)を分けて返す。
/// （`parse_file` はファイル読み込み/UTF-8 エラー時のみ Err。書式の崩れは警告扱いで Ok。）
fn parse_all(scripts: &[PathBuf]) -> (Vec<ParsedScript>, Vec<ScriptFailure>) {
    let mut parsed = Vec::new();
    let mut failures = Vec::new();
    for path in scripts {
        let mut parser = ScriptParser::new();
        match parser.parse_file(path) {
            Ok(scenes) => parsed.push((path.clone(), scenes)),
            Err(e) => {
                tracing::error!("パース失敗 {}: {e:#}", path.display());
                failures.push((path.clone(), format!("パース失敗: {e:#}")));
            }
        }
    }
    (parsed, failures)
}

/// バッチ処理の結果サマリ。
struct BatchSummary {
    succeeded: usize,
    failures: Vec<ScriptFailure>,
}

impl BatchSummary {
    fn total(&self) -> usize {
        self.succeeded + self.failures.len()
    }
    fn has_failure(&self) -> bool {
        !self.failures.is_empty()
    }
}

/// パース済み台本を順に処理する。各台本の失敗で止めず、`prior_failures`（パース失敗等）に
/// 積み増してサマリを返す。`process` は1台本ぶんの処理（成功で Ok、失敗で Err）。
/// テスト容易化のためクロージャ注入にしている。
async fn run_each<F, Fut>(
    parsed: Vec<ParsedScript>,
    mut prior_failures: Vec<ScriptFailure>,
    mut process: F,
) -> BatchSummary
where
    F: FnMut(PathBuf, Vec<Scene>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let total = parsed.len();
    let mut succeeded = 0usize;
    for (i, (path, scenes)) in parsed.into_iter().enumerate() {
        tracing::info!("[{}/{}] 処理開始: {}", i + 1, total, path.display());
        match process(path.clone(), scenes).await {
            Ok(()) => {
                succeeded += 1;
                tracing::info!("[{}/{}] 完了: {}", i + 1, total, path.display());
            }
            Err(e) => {
                tracing::error!("処理失敗 {}: {e:#}", path.display());
                prior_failures.push((path, format!("{e:#}")));
            }
        }
    }
    BatchSummary { succeeded, failures: prior_failures }
}

/// 必要エンジンを個別に起動する。1つの失敗で全体を止めず、警告して継続する
/// （起動に失敗したエンジンを使う台本は後段の合成で失敗扱いになる）。
async fn activate_each(engine_manager: &Arc<EngineManager>, required: &HashSet<String>) {
    for name in required {
        let Some(engine) = engine_manager.get(name) else {
            tracing::warn!("[{name}] 未登録のエンジンが要求されました。スキップします。");
            continue;
        };
        match engine.activate().await {
            Ok(()) => tracing::info!("[{name}] エンジン起動完了。"),
            Err(e) => tracing::warn!(
                "[{name}] エンジン起動に失敗しました（このエンジンを使う台本は失敗します）: {e:#}"
            ),
        }
    }
}

/// 1台本を処理する。出力フォルダ（台本名）を決め、その run.log にログを向けてから
/// 既存 `Producer` を実行する。ログ出力先は処理後に必ず外す。
async fn process_one(
    script_path: &Path,
    scenes: &[Scene],
    config: &Config,
    engine_manager: &Arc<EngineManager>,
    log_file: &SharedLogFile,
) -> anyhow::Result<()> {
    let project_name = script_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("台本ファイル名が不正です: {}", script_path.display()))?
        .to_string();
    let project_dir = script_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&project_name);
    std::fs::create_dir_all(&project_dir)?;

    log_file.set(Some(open_run_log(&project_dir)?));
    let _scope = LogScope(log_file);
    async {
        tracing::info!("--- Project: {project_name} ---");
        tracing::info!("Output Directory: {}", project_dir.display());
        let producer = Producer::new(Arc::clone(engine_manager), config, &project_dir)?;
        producer.produce(scenes).await?;
        tracing::info!("--- 完了: {project_name} ---");
        anyhow::Ok(())
    }
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_each_continues_after_failure_and_counts() {
        let mut parser = s2v_core::ScriptParser::new();
        let scenes = parser.parse_str("@scene S\n@script\n").unwrap();
        let parsed = vec![
            (PathBuf::from("a.txt"), scenes.clone()),
            (PathBuf::from("b.txt"), scenes.clone()),
            (PathBuf::from("c.txt"), scenes.clone()),
        ];

        let summary = run_each(parsed, Vec::new(), |path, _scenes| async move {
            if path == PathBuf::from("b.txt") {
                anyhow::bail!("わざと失敗")
            } else {
                Ok(())
            }
        })
        .await;

        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.failures.len(), 1);
        assert_eq!(summary.failures[0].0, PathBuf::from("b.txt"));
        assert!(summary.has_failure());
        assert_eq!(summary.total(), 3);
    }

    #[tokio::test]
    async fn run_each_keeps_prior_failures() {
        let summary = run_each(
            Vec::new(),
            vec![(PathBuf::from("parsefail.txt"), "パース失敗".to_string())],
            |_p, _s| async { Ok(()) },
        )
        .await;
        assert_eq!(summary.succeeded, 0);
        assert_eq!(summary.failures.len(), 1);
        assert!(summary.has_failure());
    }

    #[test]
    fn parses_multiple_script_paths() {
        let cli = Cli::try_parse_from(["script2voice", "a.txt", "b.txt"]).unwrap();
        assert_eq!(cli.scripts, vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]);
        assert_eq!(cli.config, None);
    }

    #[test]
    fn parses_custom_config_path() {
        let cli = Cli::try_parse_from(["script2voice", "script.txt", "--config", "custom.toml"]).unwrap();
        assert_eq!(cli.config, Some(std::path::PathBuf::from("custom.toml")));
    }

    #[test]
    fn resolve_config_path_prefers_explicit_path() {
        let resolved = resolve_config_path(
            Some(std::path::PathBuf::from("custom.toml")),
            Some(std::path::Path::new("/opt/script2voice/script2voice")),
        );
        assert_eq!(resolved, std::path::PathBuf::from("custom.toml"));
    }

    #[test]
    fn resolve_config_path_defaults_to_executable_directory() {
        let resolved = resolve_config_path(
            None,
            Some(std::path::Path::new("/opt/script2voice/bin/script2voice.exe")),
        );
        assert_eq!(resolved, std::path::PathBuf::from("/opt/script2voice/bin/config.toml"));
    }

    #[test]
    fn resolve_config_path_falls_back_to_relative_when_exe_path_unknown() {
        let resolved = resolve_config_path(None, None);
        assert_eq!(resolved, std::path::PathBuf::from("config.toml"));
    }

    #[test]
    fn fails_without_script_argument() {
        let result = Cli::try_parse_from(["script2voice"]);
        assert!(result.is_err());
    }

    #[test]
    fn shared_log_writer_discards_when_unset_and_writes_when_set() {
        use std::io::Write;
        use tracing_subscriber::fmt::MakeWriter;

        let shared = SharedLogFile::default();

        // 未設定: 書いても捨てられ、エラーにならない
        {
            let mut w = shared.make_writer();
            assert!(w.write(b"discarded\n").is_ok());
        }

        // 設定: ファイルに書かれる
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        shared.set(Some(file));
        {
            let mut w = shared.make_writer();
            w.write_all(b"hello-log\n").unwrap();
            w.flush().unwrap();
        }
        shared.set(None);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("hello-log"));
    }

    #[test]
    fn expand_collects_txt_in_directory_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.txt"), "x").unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::write(dir.path().join("note.md"), "x").unwrap();
        let out = expand_script_args(&[dir.path().to_path_buf()]).unwrap();
        let names: Vec<String> = out
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn expand_accepts_files_and_dedups() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("s.txt");
        std::fs::write(&f, "x").unwrap();
        let out = expand_script_args(&[f.clone(), f.clone()]).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn expand_errors_on_missing_path() {
        let res = expand_script_args(&[PathBuf::from("definitely_not_here_12345.txt")]);
        assert!(res.is_err());
    }

    #[test]
    fn expand_errors_when_directory_has_no_txt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), "x").unwrap();
        let res = expand_script_args(&[dir.path().to_path_buf()]);
        assert!(res.is_err());
    }

    #[test]
    fn parse_all_continues_past_unreadable_file() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.txt");
        std::fs::write(
            &good,
            "@scene S\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\nA:こんにちは\n",
        )
        .unwrap();
        let bad = dir.path().join("bad.txt");
        std::fs::write(&bad, [0xff, 0xfe, 0x00, 0x01]).unwrap(); // 不正UTF-8 → read_to_string失敗

        let (parsed, failures) = parse_all(&[good.clone(), bad.clone()]);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, good);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, bad);
    }

    #[test]
    fn required_engines_unions_across_scripts() {
        let mut p1 = s2v_core::ScriptParser::new();
        let s1 = p1
            .parse_str("@scene S\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\nA:あ\n")
            .unwrap();
        let mut p2 = s2v_core::ScriptParser::new();
        let s2 = p2
            .parse_str("@scene S\n@cast\nB:話者:ノーマル,aivis,pan=0\n@script\nB:い\n")
            .unwrap();
        let parsed = vec![(PathBuf::from("1.txt"), s1), (PathBuf::from("2.txt"), s2)];
        let req = required_engines(&parsed);
        assert!(req.contains("voicevox"));
        assert!(req.contains("aivis"));
        assert_eq!(req.len(), 2);
    }
}
