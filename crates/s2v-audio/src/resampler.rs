use rubato::{FftFixedIn, Resampler};

/// モノラル f32 サンプル列を `from_rate` から `to_rate` にリサンプリングする。
/// rubato::FftFixedIn を使用 (SIMD 最適化)。
pub fn resample_mono(samples: &[f32], from_rate: u32, to_rate: u32) -> anyhow::Result<Vec<f32>> {
    if from_rate == to_rate {
        return Ok(samples.to_vec());
    }
    if samples.is_empty() {
        return Ok(Vec::new());
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let chunk_size = 1024_usize;

    let mut resampler = FftFixedIn::<f32>::new(
        from_rate as usize,
        to_rate as usize,
        chunk_size,
        2,
        1,
    )?;

    let mut input_frames = samples.to_vec();
    // 最後のチャンクに必要なパディングを追加
    let needed = chunk_size * ((input_frames.len() + chunk_size - 1) / chunk_size);
    input_frames.resize(needed, 0.0);

    let mut output = Vec::with_capacity((samples.len() as f64 * ratio) as usize + chunk_size);

    for chunk in input_frames.chunks(chunk_size) {
        let wave_in = vec![chunk.to_vec()];
        let wave_out = resampler.process(&wave_in, None)?;
        output.extend_from_slice(&wave_out[0]);
    }

    // 期待出力サンプル数に切り詰め
    let expected = (samples.len() as f64 * ratio).round() as usize;
    output.truncate(expected);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_wave(freq: f32, sample_rate: u32, duration_s: f32) -> Vec<f32> {
        let n = (sample_rate as f32 * duration_s) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    fn same_rate_returns_identical() {
        let samples = sine_wave(440.0, 24000, 0.1);
        let out = resample_mono(&samples, 24000, 24000).unwrap();
        assert_eq!(out.len(), samples.len());
        for (a, b) in samples.iter().zip(out.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn empty_input_returns_empty() {
        let out = resample_mono(&[], 24000, 48000).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn upsample_produces_correct_length() {
        let samples = sine_wave(440.0, 24000, 0.5);
        let out = resample_mono(&samples, 24000, 48000).unwrap();
        let expected = (samples.len() as f64 * 2.0).round() as usize;
        // rubato の出力は ±chunk_size 以内に収まる
        assert!((out.len() as isize - expected as isize).unsigned_abs() <= 1024);
    }

    #[test]
    fn downsample_produces_correct_length() {
        let samples = sine_wave(440.0, 48000, 0.5);
        let out = resample_mono(&samples, 48000, 24000).unwrap();
        let expected = (samples.len() as f64 * 0.5).round() as usize;
        assert!((out.len() as isize - expected as isize).unsigned_abs() <= 1024);
    }

    #[test]
    fn output_samples_are_finite() {
        let samples = sine_wave(1000.0, 22050, 0.2);
        let out = resample_mono(&samples, 22050, 48000).unwrap();
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
