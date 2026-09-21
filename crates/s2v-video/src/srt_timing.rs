//! SRT字幕から [PARAGRAPH] マーカー時刻を抽出し、各表示セグメント(開始秒,終了秒)を計算する。
use std::sync::OnceLock;

use regex::Regex;

/// SRT の [PARAGRAPH] マーカー1件。
#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphMarker {
    pub time_s: f64,
    pub name: Option<String>,
}

fn paragraph_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\d+\r?\n(\d{2}):(\d{2}):(\d{2}),(\d{3}) --> \d{2}:\d{2}:\d{2},\d{3}\r?\n\[PARAGRAPH(?: ([^\]\r\n]*))?\]",
        )
        .expect("PARAGRAPH 正規表現が不正")
    })
}

/// SRTテキストから [PARAGRAPH] エントリを出現順に返す。
pub fn parse_paragraph_markers(srt_text: &str) -> Vec<ParagraphMarker> {
    let mut markers = Vec::new();
    for cap in paragraph_re().captures_iter(srt_text) {
        let h: f64 = cap[1].parse().unwrap();
        let m: f64 = cap[2].parse().unwrap();
        let s: f64 = cap[3].parse().unwrap();
        let ms: f64 = cap[4].parse().unwrap();
        let name = cap
            .get(5)
            .map(|g| g.as_str().trim().to_string())
            .filter(|s| !s.is_empty());
        markers.push(ParagraphMarker {
            time_s: h * 3600.0 + m * 60.0 + s + ms / 1000.0,
            name,
        });
    }
    markers
}

/// マーカー時刻と音声総時間から各表示セグメントの (開始秒, 終了秒) を計算する。
/// マーカーは「0より大きい・単調増加・総時間以下」を満たさなければエラー。
pub fn compute_segments(
    marker_times: &[f64],
    total_duration_s: f64,
) -> anyhow::Result<Vec<(f64, f64)>> {
    let mut prev = 0.0_f64;
    for (i, &t) in marker_times.iter().enumerate() {
        let n = i + 1;
        if t < 0.0 {
            anyhow::bail!("{n}番目の[PARAGRAPH]マーカー時刻が負です: {t}");
        }
        if t <= prev {
            anyhow::bail!(
                "{n}番目の[PARAGRAPH]マーカー時刻が単調増加していません: {t} は直前の境界 {prev} 以下です"
            );
        }
        if t > total_duration_s {
            anyhow::bail!(
                "{n}番目の[PARAGRAPH]マーカー時刻が音声総時間を超えています: {t} > {total_duration_s}"
            );
        }
        prev = t;
    }
    let mut boundaries = Vec::with_capacity(marker_times.len() + 2);
    boundaries.push(0.0);
    boundaries.extend_from_slice(marker_times);
    boundaries.push(total_duration_s);
    Ok(boundaries.windows(2).map(|w| (w[0], w[1])).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paragraph_marker_times() {
        let srt = "1\n00:00:00,000 --> 00:00:01,500\nこんにちは\n\n\
                   2\n00:00:01,500 --> 00:00:01,500\n[PARAGRAPH]\n\n\
                   3\n00:00:03,000 --> 00:00:03,800\nさようなら\n\n\
                   4\n00:01:05,250 --> 00:01:05,250\n[PARAGRAPH]\n\n";
        let times: Vec<f64> = parse_paragraph_markers(srt)
            .iter()
            .map(|m| m.time_s)
            .collect();
        assert_eq!(times, vec![1.5, 65.25]);
    }

    #[test]
    fn parses_paragraph_marker_names() {
        let srt = "1\n00:00:01,500 --> 00:00:01,500\n[PARAGRAPH オープニング]\n\n\
                   2\n00:01:05,250 --> 00:01:05,250\n[PARAGRAPH]\n\n";
        let markers = parse_paragraph_markers(srt);
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].time_s, 1.5);
        assert_eq!(markers[0].name.as_deref(), Some("オープニング"));
        assert_eq!(markers[1].time_s, 65.25);
        assert_eq!(markers[1].name, None);
    }

    #[test]
    fn parse_returns_empty_when_no_markers() {
        let srt = "1\n00:00:00,000 --> 00:00:01,500\nこんにちは\n\n";
        assert!(parse_paragraph_markers(srt).is_empty());
    }

    #[test]
    fn segments_split_into_marker_count_plus_one() {
        let seg = compute_segments(&[1.5, 65.25], 120.0).unwrap();
        assert_eq!(seg, vec![(0.0, 1.5), (1.5, 65.25), (65.25, 120.0)]);
    }

    #[test]
    fn segments_single_when_no_markers() {
        assert_eq!(compute_segments(&[], 10.0).unwrap(), vec![(0.0, 10.0)]);
    }

    #[test]
    fn segments_reject_non_monotonic() {
        let e = compute_segments(&[5.0, 3.0], 10.0).unwrap_err();
        assert!(e.to_string().contains("単調増加"));
    }

    #[test]
    fn segments_reject_duplicate() {
        let e = compute_segments(&[3.0, 3.0], 10.0).unwrap_err();
        assert!(e.to_string().contains("単調増加"));
    }

    #[test]
    fn segments_reject_negative() {
        let e = compute_segments(&[-1.0], 10.0).unwrap_err();
        assert!(e.to_string().contains("負"));
    }

    #[test]
    fn segments_reject_beyond_total() {
        let e = compute_segments(&[15.0], 10.0).unwrap_err();
        assert!(e.to_string().contains("総時間"));
    }

    #[test]
    fn segments_reject_marker_at_zero() {
        let e = compute_segments(&[0.0, 5.0], 10.0).unwrap_err();
        assert!(e.to_string().contains("単調増加"));
    }

    #[test]
    fn parse_handles_crlf_line_endings() {
        // 本プロジェクトは Windows/CRLF 環境。正規表現の \r?\n 分岐を検証する。
        let srt = "1\r\n00:00:00,000 --> 00:00:01,500\r\nこんにちは\r\n\r\n\
                   2\r\n00:00:01,500 --> 00:00:01,500\r\n[PARAGRAPH]\r\n\r\n\
                   3\r\n00:01:05,250 --> 00:01:05,250\r\n[PARAGRAPH]\r\n\r\n";
        let times: Vec<f64> = parse_paragraph_markers(srt)
            .iter()
            .map(|m| m.time_s)
            .collect();
        assert_eq!(times, vec![1.5, 65.25]);
    }
}
