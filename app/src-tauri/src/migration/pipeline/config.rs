#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub planner_concurrency: usize,
    pub download_concurrency: usize,
    pub processing_concurrency: usize,
    pub upload_concurrency: usize,
    pub local_finalizer_concurrency: usize,

    pub download_queue_capacity: usize,
    pub processing_queue_capacity: usize,
    pub upload_queue_capacity: usize,
    pub local_finalizer_queue_capacity: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            planner_concurrency: 1,
            download_concurrency: 2,
            processing_concurrency: 1,
            upload_concurrency: 1,
            local_finalizer_concurrency: 2,

            download_queue_capacity: 4,
            processing_queue_capacity: 2,
            upload_queue_capacity: 2,
            local_finalizer_queue_capacity: 4,
        }
    }
}
