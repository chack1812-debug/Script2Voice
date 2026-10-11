//! Script2Voice の出力(音声 + [PARAGRAPH]入りSRT)とシーン画像から動画を生成する。
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;

use crate::ffmpeg_cmd::build_command;
use crate::scene_map::{
    load_scene_map, resolve_asset_paths, resolve_assets, validate_assets_exist, AssetKind,
};
use crate::srt_timing::{compute_segments, parse_paragraph_markers};

/// ffprobe の待ち時間の上限。長さを調べるだけなので通常は一瞬で終わる。
/// ffmpeg 本体は長い動画で長時間かかり得るため、タイムアウトを付けない。
const FFPROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// compose サブコマンドのオプション。
pub struct ComposeOptions {
    pub project_dir: PathBuf,
    pub scene_map: Option<PathBuf>,
    pub burn_subtitle: bool,
    pub output: Option<PathBuf>,
    /// 出力ロックで連番退避された生成一式（`full_dialogue_N.wav` など）を使う場合の N。
    /// `None` なら接尾辞なしの既定の一式を使う。
    pub generation: Option<u32>,
}

/// 世代番号を、生成時のファイル名接尾辞（`_N`）にする。
fn generation_suffix(generation: Option<u32>) -> String {
    generation.map(|n| format!("_{n}")).unwrap_or_default()
}

fn srt_path_for(project_dir: &Path, suffix: &str) -> PathBuf {
    project_dir
        .join("timeline")
        .join(format!("subtitles{suffix}.srt"))
}

/// 出力先の既定値。世代を指定した場合は既定世代の動画を上書きしないよう `output_N.mp4` にする。
fn default_output_for(project_dir: &Path, suffix: &str) -> PathBuf {
    project_dir.join(format!("output{suffix}.mp4"))
}

/// project_dir 内の音声ファイルを探す。full_dialogue{suffix}.wav を優先、無ければ .mp3。
fn find_audio_file(project_dir: &Path, suffix: &str) -> anyhow::Result<PathBuf> {
    let names = [
        format!("full_dialogue{suffix}.wav"),
        format!("full_dialogue{suffix}.mp3"),
    ];
    for name in &names {
        let candidate = project_dir.join(name);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "{} に {} / {} が見つかりません",
        project_dir.display(),
        names[0],
        names[1]
    )
}

/// 外部コマンドを実行し、`timeout` を過ぎたら kill してエラーにする。
/// 標準出力は短い前提（ffprobe の duration 1行など）で、終了後にまとめて読む。
fn output_with_timeout(mut cmd: Command, timeout: Duration) -> anyhow::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("{} 秒でタイムアウトしました", timeout.as_secs_f64());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdout = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_end(&mut stdout)?;
    }
    Ok(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

/// ffprobe でメディアの総再生時間(秒)を取得する。
fn probe_duration_seconds(media_path: &Path) -> anyhow::Result<f64> {
    let mut cmd = Command::new("ffprobe");
    cmd.args([
        "-v",
        "quiet",
        "-show_entries",
        "format=duration",
        "-of",
        "csv=p=0",
    ])
    .arg(media_path);
    let output = output_with_timeout(cmd, FFPROBE_TIMEOUT)
        .with_context(|| format!("ffprobe の実行に失敗しました: {}", media_path.display()))?;
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
    let suffix = generation_suffix(opts.generation);
    let scene_map_path = opts
        .scene_map
        .clone()
        .unwrap_or_else(|| project_dir.join("scene_map.json"));
    let output_path = opts
        .output
        .clone()
        .unwrap_or_else(|| default_output_for(project_dir, &suffix));
    let srt_path = srt_path_for(project_dir, &suffix);

    let audio_path = find_audio_file(project_dir, &suffix)?;
    let srt_text = std::fs::read_to_string(&srt_path)
        .with_context(|| format!("字幕を読めません: {}", srt_path.display()))?;
    let markers: Vec<f64> = parse_paragraph_markers(&srt_text)
        .iter()
        .map(|m| m.time_s)
        .collect();
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
    let burn = if opts.burn_subtitle {
        Some(srt_path.as_path())
    } else {
        None
    };
    let cmd = build_command(&audio_path, &assets, &durations, &output_path, burn)?;

    if output_path.exists() {
        eprintln!(
            "警告: 既存の出力ファイルを上書きします: {}",
            output_path.display()
        );
    }
    let status = Command::new(&cmd[0])
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
        assert_eq!(
            find_audio_file(dir.path(), "").unwrap(),
            dir.path().join("full_dialogue.wav")
        );
    }

    #[test]
    fn find_audio_falls_back_to_mp3() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("full_dialogue.mp3"), b"").unwrap();
        assert_eq!(
            find_audio_file(dir.path(), "").unwrap(),
            dir.path().join("full_dialogue.mp3")
        );
    }

    #[test]
    fn find_audio_errors_when_neither_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_audio_file(dir.path(), "").is_err());
    }

    // ---- s2v-18b.8: 出力世代(_N)の指定 ----

    #[test]
    fn find_audio_uses_generation_suffix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("full_dialogue.wav"), b"").unwrap();
        std::fs::write(dir.path().join("full_dialogue_2.wav"), b"").unwrap();
        assert_eq!(
            find_audio_file(dir.path(), "_2").unwrap(),
            dir.path().join("full_dialogue_2.wav")
        );
        assert!(find_audio_file(dir.path(), "_3").is_err());
    }

    #[test]
    fn generation_paths_follow_suffix() {
        let opts = ComposeOptions {
            project_dir: PathBuf::from("proj"),
            scene_map: None,
            burn_subtitle: false,
            output: None,
            generation: Some(2),
        };
        assert_eq!(generation_suffix(opts.generation), "_2");
        assert_eq!(
            srt_path_for(&opts.project_dir, "_2"),
            PathBuf::from("proj")
                .join("timeline")
                .join("subtitles_2.srt")
        );
        assert_eq!(
            default_output_for(&opts.project_dir, "_2"),
            PathBuf::from("proj").join("output_2.mp4")
        );
        assert_eq!(generation_suffix(None), "");
        assert_eq!(
            default_output_for(&opts.project_dir, ""),
            PathBuf::from("proj").join("output.mp4")
        );
    }

    // ---- s2v-18b.8: ffprobe のタイムアウト ----

    #[test]
    fn output_with_timeout_kills_hung_process() {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", "ping", "-n", "30", "127.0.0.1"]);
        let start = std::time::Instant::now();
        let err = output_with_timeout(cmd, Duration::from_millis(500)).unwrap_err();
        assert!(err.to_string().contains("タイムアウト"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn output_with_timeout_returns_stdout_of_quick_process() {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", "echo", "12.5"]);
        let out = output_with_timeout(cmd, Duration::from_secs(10)).unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "12.5");
    }
}
