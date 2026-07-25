use crate::migration::pipeline_v2::stages::{MediaInspector, VideoMetadata, VideoProcessor};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub struct FFmpegMediaAdapter;

impl MediaInspector for FFmpegMediaAdapter {
    fn inspect_file(
        &self,
        _path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<VideoMetadata, String>> + Send>> {
        Box::pin(async {
            Ok(VideoMetadata::default())
        })
    }
}

impl VideoProcessor for FFmpegMediaAdapter {
    fn process_video(
        &self,
        _input_path: &Path,
        _output_path: &Path,
        _decision: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
        Box::pin(async {
            Ok("fake_sha256".to_string())
        })
    }
}
