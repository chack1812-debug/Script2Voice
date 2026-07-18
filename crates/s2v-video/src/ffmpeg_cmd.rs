//! FFmpeg のスライドショー合成コマンドを構築する純粋関数(実行はしない)。
use std::path::Path;

use crate::scene_map::{Asset, AssetKind};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const CRF: u32 = 18;

fn scale_pad_filter(label_in: &str, label_out: &str) -> String {
    format!(
        "[{label_in}]scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=decrease,\
         pad={WIDTH}:{HEIGHT}:(ow-iw)/2:(oh-ih)/2,setsar=1,setpts=PTS-STARTPTS[{label_out}]"
    )
}

/// ffmpeg subtitles フィルタ用にパスをエスケープする(Windowsのドライブレターのコロン対策)。
fn escape_subtitles_path(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\\', "/").replace(':', "\\:");
    format!("'{escaped}'")
}

/// ffmpeg コマンドを引数リストとして構築する。
/// 画像は静止ループ、動画クリップは source_duration が duration 以上ならトリミング、
/// 未満なら不足分を最終フレーム静止(tpad)で埋める。burn_subtitle_path 指定で字幕焼き込み。
pub fn build_command(
    audio_path: &Path,
    assets: &[Asset],
    durations: &[f64],
    output_path: &Path,
    burn_subtitle_path: Option<&Path>,
) -> anyhow::Result<Vec<String>> {
    if assets.len() != durations.len() {
        anyhow::bail!("assets と durations は同じ長さでなければならない");
    }
    if assets.is_empty() {
        anyhow::bail!("assets が空です");
    }
    for (i, &d) in durations.iter().enumerate() {
        if d <= 0.0 {
            anyhow::bail!("durations[{i}] は0より大きくなければなりません: {d}");
        }
    }

    let mut cmd: Vec<String> = vec!["ffmpeg".into(), "-y".into(), "-i".into(), audio_path.to_string_lossy().into_owned()];
    for (asset, &duration) in assets.iter().zip(durations) {
        match asset.kind {
            AssetKind::Video => {
                cmd.push("-t".into());
                cmd.push(format!("{duration:.3}"));
                cmd.push("-i".into());
                cmd.push(asset.path.clone());
            }
            AssetKind::Image => {
                cmd.push("-loop".into());
                cmd.push("1".into());
                cmd.push("-t".into());
                cmd.push(format!("{duration:.3}"));
                cmd.push("-i".into());
                cmd.push(asset.path.clone());
            }
        }
    }

    let mut filter_parts: Vec<String> = Vec::new();
    let mut video_labels: Vec<String> = Vec::new();
    for (i, (asset, &duration)) in assets.iter().zip(durations).enumerate() {
        let in_label = format!("{}:v", i + 1);
        let out_label = format!("v{}", i + 1);
        let source_duration = if asset.kind == AssetKind::Video { asset.source_duration } else { None };
        let deficit = source_duration.map(|s| duration - s).unwrap_or(0.0);
        if deficit > 0.0 {
            let scaled = format!("v{}pre", i + 1);
            filter_parts.push(scale_pad_filter(&in_label, &scaled));
            filter_parts.push(format!(
                "[{scaled}]tpad=stop_mode=clone:stop_duration={deficit:.3}[{out_label}]"
            ));
        } else {
            filter_parts.push(scale_pad_filter(&in_label, &out_label));
        }
        video_labels.push(format!("[{out_label}]"));
    }

    let concat_inputs = video_labels.join("");
    filter_parts.push(format!("{concat_inputs}concat=n={}:v=1:a=0[vout]", assets.len()));

    let mut final_video_label = "[vout]".to_string();
    if let Some(sub) = burn_subtitle_path {
        filter_parts.push(format!("[vout]subtitles={}[vsub]", escape_subtitles_path(sub)));
        final_video_label = "[vsub]".to_string();
    }

    cmd.push("-filter_complex".into());
    cmd.push(filter_parts.join(";"));
    cmd.push("-map".into());
    cmd.push(final_video_label);
    cmd.push("-map".into());
    cmd.push("0:a".into());
    cmd.extend(["-c:v", "libx264", "-crf"].iter().map(|s| s.to_string()));
    cmd.push(CRF.to_string());
    cmd.extend(["-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", "-movflags", "+faststart"].iter().map(|s| s.to_string()));
    cmd.push(output_path.to_string_lossy().into_owned());
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_map::{Asset, AssetKind};
    use std::path::Path;

    fn img(p: &str) -> Asset {
        Asset { kind: AssetKind::Image, path: p.into(), source_duration: None }
    }
    fn vid(p: &str, d: f64) -> Asset {
        Asset { kind: AssetKind::Video, path: p.into(), source_duration: Some(d) }
    }
    fn filter_of(cmd: &[String]) -> &str {
        let i = cmd.iter().position(|a| a == "-filter_complex").unwrap();
        &cmd[i + 1]
    }
    fn input_paths(cmd: &[String]) -> Vec<&str> {
        cmd.iter().enumerate().filter(|(_, a)| *a == "-i").map(|(i, _)| cmd[i + 1].as_str()).collect()
    }

    #[test]
    fn feeds_audio_then_looped_timed_images() {
        let cmd = build_command(
            Path::new("project/full_dialogue.wav"),
            &[img("images/scene01.png"), img("images/scene02.png")],
            &[1.5, 63.75],
            Path::new("project/output.mp4"),
            None,
        )
        .unwrap();
        assert_eq!(cmd[0], "ffmpeg");
        let inputs = input_paths(&cmd);
        assert_eq!(inputs[0], "project/full_dialogue.wav");
        assert_eq!(inputs[1], "images/scene01.png");
        let loop_idx = cmd.iter().position(|a| a == "-loop").unwrap();
        assert_eq!(&cmd[loop_idx..loop_idx + 4], &["-loop", "1", "-t", "1.500"]);
    }

    #[test]
    fn filter_scales_pads_and_concats_all_images() {
        let cmd = build_command(
            Path::new("a.wav"),
            &[img("s1.png"), img("s2.png"), img("s3.png")],
            &[1.0, 2.0, 3.0],
            Path::new("out.mp4"),
            None,
        )
        .unwrap();
        let f = filter_of(&cmd);
        assert!(f.contains("[1:v]scale=1920:1080:force_original_aspect_ratio=decrease"));
        assert!(f.contains("[v1][v2][v3]concat=n=3:v=1:a=0[vout]"));
        let map_idx = cmd.iter().position(|a| a == "-map").unwrap();
        assert_eq!(cmd[map_idx + 1], "[vout]");
        assert!(cmd.iter().any(|a| a == "0:a"));
    }

    #[test]
    fn appends_subtitles_filter_and_remaps_when_burn_given() {
        let cmd = build_command(
            Path::new("a.wav"),
            &[img("s1.png")],
            &[5.0],
            Path::new("out.mp4"),
            Some(Path::new("project/timeline/subtitles.srt")),
        )
        .unwrap();
        let f = filter_of(&cmd);
        assert!(f.contains("[vout]subtitles="));
        assert!(f.contains("[vsub]"));
        let map_idx = cmd.iter().position(|a| a == "-map").unwrap();
        assert_eq!(cmd[map_idx + 1], "[vsub]");
    }

    #[test]
    fn uses_libx264_crf18_shortest() {
        let cmd = build_command(Path::new("a.wav"), &[img("s1.png")], &[5.0], Path::new("out.mp4"), None).unwrap();
        let cv = cmd.iter().position(|a| a == "-c:v").unwrap();
        assert_eq!(cmd[cv + 1], "libx264");
        let crf = cmd.iter().position(|a| a == "-crf").unwrap();
        assert_eq!(cmd[crf + 1], "18");
        assert!(cmd.iter().any(|a| a == "-shortest"));
        assert_eq!(cmd.last().unwrap(), "out.mp4");
    }

    #[test]
    fn forces_yuv420p_and_faststart() {
        let cmd = build_command(Path::new("a.wav"), &[img("s1.png")], &[5.0], Path::new("out.mp4"), None).unwrap();
        let pf = cmd.iter().position(|a| a == "-pix_fmt").unwrap();
        assert_eq!(cmd[pf + 1], "yuv420p");
        let mf = cmd.iter().position(|a| a == "-movflags").unwrap();
        assert_eq!(cmd[mf + 1], "+faststart");
    }

    #[test]
    fn errors_on_length_mismatch() {
        let e = build_command(Path::new("a.wav"), &[img("s1.png"), img("s2.png")], &[1.0], Path::new("out.mp4"), None).unwrap_err();
        assert!(e.to_string().contains("同じ長さ"));
    }

    #[test]
    fn errors_on_zero_or_negative_duration() {
        let e = build_command(Path::new("a.wav"), &[img("s1.png"), img("s2.png")], &[1.0, 0.0], Path::new("out.mp4"), None).unwrap_err();
        assert!(e.to_string().contains("duration"));
        let e = build_command(Path::new("a.wav"), &[img("s1.png")], &[-1.0], Path::new("out.mp4"), None).unwrap_err();
        assert!(e.to_string().contains("duration"));
    }

    #[test]
    fn feeds_video_clip_without_loop_flag() {
        let cmd = build_command(Path::new("a.wav"), &[vid("assets/p01.mp4", 10.0)], &[5.0], Path::new("out.mp4"), None).unwrap();
        assert!(!cmd.iter().any(|a| a == "-loop"));
        let t = cmd.iter().position(|a| a == "-t").unwrap();
        assert_eq!(&cmd[t..t + 4], &["-t", "5.000", "-i", "assets/p01.mp4"]);
    }

    #[test]
    fn trims_video_longer_than_duration_without_tpad() {
        let cmd = build_command(Path::new("a.wav"), &[vid("assets/p01.mp4", 10.0)], &[5.0], Path::new("out.mp4"), None).unwrap();
        assert!(!filter_of(&cmd).contains("tpad"));
    }

    #[test]
    fn freezes_last_frame_for_video_shorter_than_duration() {
        let cmd = build_command(Path::new("a.wav"), &[vid("assets/p01.mp4", 3.0)], &[5.0], Path::new("out.mp4"), None).unwrap();
        let f = filter_of(&cmd);
        assert!(f.contains("tpad=stop_mode=clone:stop_duration=2.000"));
        assert!(f.contains("[v1pre]tpad=stop_mode=clone:stop_duration=2.000[v1]"));
    }

    #[test]
    fn mixes_image_and_video_in_concat() {
        let cmd = build_command(Path::new("a.wav"), &[img("s1.png"), vid("assets/p02.mp4", 8.0)], &[1.0, 2.0], Path::new("out.mp4"), None).unwrap();
        assert!(filter_of(&cmd).contains("[v1][v2]concat=n=2:v=1:a=0[vout]"));
    }
}
