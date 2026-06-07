use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use reqwest::Client;
use s2v_core::{Config, ScriptParser};
use s2v_engines::{EngineManager, HttpEngine, XttsEngine};
use script2voice::Producer;
use tracing_subscriber::EnvFilter;

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

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
}
