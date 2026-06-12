use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use s2v_core::{Config, Scene, ScriptParser};
use s2v_engines::EngineManager;
use script2voice::{Producer, build_engine_manager, resolve_config_path};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser)]
#[command(name = "script2voice", version, about = "台本から音声・字幕・タイムラインを生成する")]
struct Cli {
    /// 台本ファイルのパス
    script: PathBuf,

    /// 設定ファイル (config.toml) のパス。省略時は実行ファイルと同じディレクトリの config.toml を使用する
    #[arg(short, long)]
    config: Option<PathBuf>,
}

/// 実行ログを追記するファイルのパス（project_dir/run.log）を返す。
fn log_file_path(project_dir: &std::path::Path) -> PathBuf {
    project_dir.join("run.log")
}

/// コンソール（stdout）と project_dir/run.log の両方へログを出力する subscriber を初期化する。
/// 返した WorkerGuard は main 終了まで保持すること（drop でバッファを flush する）。
fn init_logging(project_dir: &std::path::Path) -> anyhow::Result<WorkerGuard> {
    let path = log_file_path(project_dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("ログファイルを開けません: {}", path.display()))?;
    let (file_writer, guard) = tracing_appender::non_blocking(file);

    let time_format = "%Y-%m-%d %H:%M:%S%.3f".to_string();

    let console_layer = fmt::layer()
        .with_timer(ChronoLocal::new(time_format.clone()))
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(file_writer)
        .with_timer(ChronoLocal::new(time_format))
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    Ok(guard)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let script_path = std::fs::canonicalize(&cli.script)
        .with_context(|| format!("台本ファイルが見つかりません: {}", cli.script.display()))?;

    let project_name = script_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("台本ファイル名が不正です: {}", script_path.display()))?
        .to_string();
    let project_dir = script_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(&project_name);
    std::fs::create_dir_all(&project_dir)?;

    let _guard = init_logging(&project_dir)?;

    tracing::info!("--- Project: {project_name} ---");
    tracing::info!("Output Directory: {}", project_dir.display());

    let exe_path = std::env::current_exe().ok();
    let config_path = resolve_config_path(cli.config, exe_path.as_deref());
    tracing::info!("設定ファイル: {}", config_path.display());
    let config = Config::from_file(&config_path)
        .with_context(|| format!("設定ファイルの読み込みに失敗しました: {}", config_path.display()))?;

    let mut parser = ScriptParser::new();
    let scenes = parser.parse_file(&script_path)?;

    let mut required_engines: HashSet<String> = HashSet::new();
    for scene in &scenes {
        for cast in scene.casts.values() {
            required_engines.insert(cast.engine_type.clone());
        }
    }
    tracing::info!(
        "使用予定のエンジン: {}",
        required_engines.iter().cloned().collect::<Vec<_>>().join(", ")
    );

    let engine_manager = Arc::new(build_engine_manager(&config));

    let result = run_pipeline(&engine_manager, &required_engines, &config, &project_dir, &scenes).await;
    engine_manager.shutdown_all();
    result?;

    tracing::info!("--- 完了: {project_name} ---");
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
fn required_engines(parsed: &[(PathBuf, Vec<Scene>)]) -> HashSet<String> {
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
fn parse_all(scripts: &[PathBuf]) -> (Vec<(PathBuf, Vec<Scene>)>, Vec<(PathBuf, String)>) {
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

async fn run_pipeline(
    engine_manager: &Arc<EngineManager>,
    required_engines: &HashSet<String>,
    config: &Config,
    project_dir: &std::path::Path,
    scenes: &[s2v_core::Scene],
) -> anyhow::Result<()> {
    engine_manager.activate_required(required_engines).await?;
    let producer = Producer::new(Arc::clone(engine_manager), config, project_dir)?;
    producer.produce(scenes).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_script_path_with_no_explicit_config() {
        let cli = Cli::try_parse_from(["script2voice", "script.txt"]).unwrap();
        assert_eq!(cli.script, std::path::PathBuf::from("script.txt"));
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
    fn log_file_path_is_run_log_in_project_dir() {
        let p = log_file_path(std::path::Path::new("/tmp/proj"));
        assert_eq!(p.file_name().unwrap(), "run.log");
        assert_eq!(p.parent().unwrap(), std::path::Path::new("/tmp/proj"));
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
