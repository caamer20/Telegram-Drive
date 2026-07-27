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

    /// Timeout cho mỗi lần download (giây), 0 = không timeout
    pub download_timeout_secs: u64,
    /// Timeout cho mỗi lần xử lý video (giây), 0 = không timeout
    pub process_timeout_secs: u64,
    /// Timeout cho mỗi lần upload Telegram (giây), 0 = không timeout
    pub upload_timeout_secs: u64,

    /// Số lần retry tối đa cho network error tạm thời
    pub max_network_retries: u32,
    /// Delay giữa các lần retry (giây): [2, 5, 15]
    pub retry_delay_secs: Vec<u64>,

    /// Thời gian chờ giữa các lần retry FloodWait
    pub flood_wait_max_retries: u32,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            planner_concurrency: 1,
            download_concurrency: 2,
            processing_concurrency: 1,
            upload_concurrency: 1,
            local_finalizer_concurrency: 1,

            download_queue_capacity: 8,
            processing_queue_capacity: 4,
            upload_queue_capacity: 4,
            local_finalizer_queue_capacity: 8,

            download_timeout_secs: 600, // 10 phút
            process_timeout_secs: 3600, // 1 giờ
            upload_timeout_secs: 600,   // 10 phút

            max_network_retries: 3,
            retry_delay_secs: vec![2, 5, 15],

            flood_wait_max_retries: 10,
        }
    }
}
