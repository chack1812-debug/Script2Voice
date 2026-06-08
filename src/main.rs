use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use reqwest::Client;
use s2v_core::{Config, ScriptParser};
use s2v_engines::{EngineManager, HttpEngine, XttsEngine};
use script2voice::Producer;
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

/// `--config` 省略時に使用する設定ファイルパスを決定する。
/// 明示指定があればそれを優先し、なければ実行ファイルと同じディレクトリの `config.toml` を返す。
fn resolve_config_path(explicit: Option<PathBuf>, exe_path: Option<&std::path::Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    exe_path
        .and_then(|p| p.parent())
        .map(|dir| dir.join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("config.toml"))
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

    let client = Arc::new(Client::new());
    let mut engine_manager = EngineManager::new();
    engine_manager.register(
        "voicevox",
        Arc::new(HttpEngine::with_exe_path(
            "voicevox",
            &config.voicevox.url,
            Arc::clone(&client),
            config.voicevox.exe_path.clone(),
        )),
    );
    engine_manager.register(
        "aivis",
        Arc::new(HttpEngine::with_exe_path(
            "aivis",
            &config.aivis.url,
            Arc::clone(&client),
            config.aivis.exe_path.clone(),
        )),
    );
    engine_manager.register(
        "xtts",
        Arc::new(XttsEngine::with_exe_path(
            "xtts",
            &config.xtts.url,
            Arc::clone(&client),
            config.xtts.exe_path.clone(),
        )),
    );

    let engine_manager = Arc::new(engine_manager);

    let result = run_pipeline(&engine_manager, &required_engines, &config, &project_dir, &scenes).await;
    engine_manager.shutdown_all();
    result?;

    tracing::info!("--- 完了: {project_name} ---");
    Ok(())
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
}
