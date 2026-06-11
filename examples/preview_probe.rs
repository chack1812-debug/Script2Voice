//! GUI の行プレビュー経路（エンジン起動→合成→音響処理）を GUI 抜きで再現する診断プローブ。
//! 実行: cargo run --example preview_probe
//! GUI(jobs.rs) と同じ 2 ワーカーの tokio ランタイム上で各段階の所要時間を表示する。

use std::collections::HashSet;
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    rt.block_on(async {
        let t0 = std::time::Instant::now();
        let config = s2v_core::Config::from_file(std::path::Path::new("config.toml"))?;
        let audio = config.audio.clone();
        let engines = Arc::new(script2voice::build_engine_manager(&config));
        println!("[{:>6.2}s] config/engines 構築", t0.elapsed().as_secs_f64());

        // 引数: [台本パス] [エンジン名フィルタ]（省略時: 音響テスト.txt / 最初の行）
        let args: Vec<String> = std::env::args().collect();
        let script = args.get(1).map(String::as_str).unwrap_or("scripts/音響テスト.txt");
        let engine_filter = args.get(2).cloned();

        let mut parser = s2v_core::ScriptParser::new();
        let scenes = parser.parse_file(std::path::Path::new(script))?;
        let (cast, text, scene_config) = scenes
            .iter()
            .flat_map(|scene| {
                scene.items.iter().filter_map(move |i| match i {
                    s2v_core::ScriptItem::Speech { cast_name, text, scene_config, .. } => scene
                        .casts
                        .get(cast_name)
                        .map(|c| (c.clone(), text.clone(), scene_config.clone())),
                    _ => None,
                })
            })
            .find(|(c, _, _)| {
                engine_filter.as_deref().map_or(true, |f| c.engine_type == f)
            })
            .expect("条件に合う speech 行が見つかりません");
        let cast_name = cast.name.clone();
        println!(
            "[{:>6.2}s] 台本パース: {}（engine={}）「{}」",
            t0.elapsed().as_secs_f64(),
            cast_name,
            cast.engine_type,
            text
        );

        let mut req = HashSet::new();
        req.insert(cast.engine_type.clone());
        engines.activate_required(&req).await?;
        println!("[{:>6.2}s] エンジン起動完了", t0.elapsed().as_secs_f64());

        let tmp = tempfile::tempdir()?;
        let raw = tmp.path().join("probe_raw.wav");
        let out = tmp.path().join("probe.wav");
        engines.synthesize(&text, &cast, &raw).await?;
        println!(
            "[{:>6.2}s] 合成完了: {} bytes",
            t0.elapsed().as_secs_f64(),
            std::fs::metadata(&raw)?.len()
        );

        let processor = s2v_audio::AudioProcessor::new(audio);
        let n = tokio::task::spawn_blocking({
            let (r, o, c, s) = (raw.clone(), out.clone(), cast.clone(), scene_config.clone());
            move || processor.process(&r, &o, &c, &s)
        })
        .await??;
        println!(
            "[{:>6.2}s] 音響処理完了: {} samples, {} bytes",
            t0.elapsed().as_secs_f64(),
            n,
            std::fs::metadata(&out)?.len()
        );

        engines.shutdown_all();
        println!("[{:>6.2}s] エンジン停止・プローブ正常終了", t0.elapsed().as_secs_f64());
        anyhow::Ok(())
    })?;
    Ok(())
}
