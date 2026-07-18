//! Script2Voice の出力(音声 + [PARAGRAPH]入りSRT)とシーン画像から動画を生成する。
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::ffmpeg_cmd::build_command;
use crate::scene_map::{load_scene_map, resolve_asset_paths, resolve_assets, validate_assets_exist, AssetKind};
use crate::srt_timing::{compute_segments, parse_paragraph_markers};

/// compose サブコマンドのオプション。
pub struct ComposeOptions {
    pub project_dir: PathBuf,
    pub scene_map: Option<PathBuf>,
    pub burn_subtitle: bool,
    pub output: Option<PathBuf>,
}

/// project_dir 内の音声ファイルを探す。full_dialogue.wav を優先、無ければ .mp3。
fn find_audio_file(project_dir: &Path) -> anyhow::Result<PathBuf> {
    for name in ["full_dialogue.wav", "full_dialogue.mp3"] {
        let candidate = project_dir.join(name);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("{} に full_dialogue.wav / full_dialogue.mp3 が見つかりません", project_dir.display())
}

/// ffprobe でメディアの総再生時間(秒)を取得する。
fn probe_duration_seconds(media_path: &Path) -> anyhow::Result<f64> {
    let output = std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(media_path)
        .output()
        .with_context(|| format!("ffprobe の起動に失敗しました: {}", media_path.display()))?;
    if !output.status.success() {
        anyhow::bail!("ffprobe が失敗しました: {}", media_path.display());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let dur: f64 = text
        .trim()
        .parse()
        .with_context(|| format!("ffprobe 出力を数値に変換できません: {text:?}"))?;
    Ok(dur)
}

/// 動画を合成する。
pub fn run(opts: &ComposeOptions) -> anyhow::Result<()> {
    let project_dir = &opts.project_dir;
    let scene_map_path = opts
        .scene_map
        .clone()
        .unwrap_or_else(|| project_dir.join("scene_map.json"));
    let output_path = opts
        .output
        .clone()
        .unwrap_or_else(|| project_dir.join("output.mp4"));
    let srt_path = project_dir.join("timeline").join("subtitles.srt");

    let audio_path = find_audio_file(project_dir)?;
    let srt_text = std::fs::read_to_string(&srt_path)
        .with_context(|| format!("字幕を読めません: {}", srt_path.display()))?;
    let markers = parse_paragraph_markers(&srt_text);
    let total_duration = probe_duration_seconds(&audio_path)?;
    let segments = compute_segments(&markers, total_duration)?;

    let scene_map = load_scene_map(&scene_map_path)?;
    let mut assets = resolve_assets(&scene_map, segments.len())?;
    let base_dir = scene_map_path
        .canonicalize()
        .unwrap_or_else(|_| scene_map_path.clone());
    let base_dir = base_dir.parent().unwrap_or(Path::new("."));
    assets = resolve_asset_paths(assets, base_dir);
    validate_assets_exist(&assets)?;
    for asset in assets.iter_mut() {
        if asset.kind == AssetKind::Video {
            asset.source_duration = Some(probe_duration_seconds(Path::new(&asset.path))?);
        }
    }

    let durations: Vec<f64> = segments.iter().map(|(s, e)| e - s).collect();
    let burn = if opts.burn_subtitle { Some(srt_path.as_path()) } else { None };
    let cmd = build_command(&audio_path, &assets, &durations, &output_path, burn)?;

    let status = std::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .status()
        .context("ffmpeg の起動に失敗しました")?;
    if !status.success() {
        anyhow::bail!("ffmpeg が失敗しました (exit={:?})", status.code());
    }
    println!("動画を生成しました: {}", output_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_audio_prefers_wav_over_mp3() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("full_dialogue.wav"), b"").unwrap();
        std::fs::write(dir.path().join("full_dialogue.mp3"), b"").unwrap();
        assert_eq!(find_audio_file(dir.path()).unwrap(), dir.path().join("full_dialogue.wav"));
    }

    #[test]
    fn find_audio_falls_back_to_mp3() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("full_dialogue.mp3"), b"").unwrap();
        assert_eq!(find_audio_file(dir.path()).unwrap(), dir.path().join("full_dialogue.mp3"));
    }

    #[test]
    fn find_audio_errors_when_neither_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_audio_file(dir.path()).is_err());
    }
}
