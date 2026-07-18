//! Script2Voice の音声・字幕・シーン画像から動画を合成するロジック。
//! Python 版 scripts/video_compose を移植したもの。
pub mod compose;
pub mod ffmpeg_cmd;
pub mod scene_map;
pub mod srt_timing;

pub use compose::ComposeOptions;
pub use scene_map::{Asset, AssetKind};
