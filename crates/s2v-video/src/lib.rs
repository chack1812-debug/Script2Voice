//! Script2Voice の音声・字幕・シーン画像から動画を合成するロジック。
//! Python 版 scripts/video_compose を移植したもの。
pub mod compose;
pub mod ffmpeg_cmd;
pub mod scene_map;
pub mod srt_timing;

// NOTE: `ComposeOptions` (Task 5) / `Asset`, `AssetKind` (Task 3) はまだ実装されていないため、
// 現時点では re-export しない。各タスクでモジュールに型を実装した際にここへ追加すること。
// pub use compose::ComposeOptions;
// pub use scene_map::{Asset, AssetKind};
