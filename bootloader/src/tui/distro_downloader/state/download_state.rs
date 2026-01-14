//! Download state tracking

use super::DownloadStatus;

/// Download state tracking
#[derive(Debug, Clone)]
pub struct DownloadState {
    /// Current status
    pub status: DownloadStatus,
    /// Current file being downloaded
    pub current_file: Option<&'static str>,
    /// Bytes downloaded so far
    pub bytes_downloaded: usize,
    /// Total bytes expected (if known)
    pub total_bytes: Option<usize>,
    /// Error message if failed
    pub error_message: Option<&'static str>,
    /// Current mirror index being used
    pub mirror_index: usize,
    /// Number of retry attempts
    pub retry_count: usize,
}

impl DownloadState {
    /// Create new idle download state
    pub fn new() -> Self {
        Self {
            status: DownloadStatus::Idle,
            current_file: None,
            bytes_downloaded: 0,
            total_bytes: None,
            error_message: None,
            mirror_index: 0,
            retry_count: 0,
        }
    }

    /// Start checking a file
    pub fn start_check(&mut self, filename: &'static str) {
        morpheus_core::logger::log("DownloadState::start_check()");
        self.status = DownloadStatus::Checking;
        self.current_file = Some(filename);
        self.bytes_downloaded = 0;
        self.total_bytes = None;
        self.error_message = None;
    }

    /// Start downloading after check
    pub fn start_download(&mut self, total: Option<usize>) {
        morpheus_core::logger::log("DownloadState::start_download()");
        self.status = DownloadStatus::Downloading;
        self.total_bytes = total;
        self.bytes_downloaded = 0;
    }

    /// Update download progress
    pub fn update_progress(&mut self, bytes: usize) {
        self.bytes_downloaded = bytes;
    }

    /// Mark download as complete
    pub fn complete(&mut self) {
        morpheus_core::logger::log("DownloadState::complete()");
        self.status = DownloadStatus::Complete;
        if let Some(total) = self.total_bytes {
            self.bytes_downloaded = total;
        }
    }

    /// Mark download as failed
    pub fn fail(&mut self, message: &'static str) {
        morpheus_core::logger::log("DownloadState::fail()");
        self.status = DownloadStatus::Failed;
        self.error_message = Some(message);
    }

    /// Reset to idle state
    pub fn reset(&mut self) {
        morpheus_core::logger::log("DownloadState::reset()");
        self.status = DownloadStatus::Idle;
        self.current_file = None;
        self.bytes_downloaded = 0;
        self.total_bytes = None;
        self.error_message = None;
        self.mirror_index = 0;
        self.retry_count = 0;
    }

    /// Try next mirror
    pub fn try_next_mirror(&mut self, max_mirrors: usize) -> bool {
        if self.mirror_index + 1 < max_mirrors {
            self.mirror_index += 1;
            self.retry_count += 1;
            self.status = DownloadStatus::Checking;
            self.error_message = None;
            morpheus_core::logger::log("DownloadState::try_next_mirror() - switching mirror");
            true
        } else {
            false
        }
    }

    /// Get progress percentage (0-100)
    pub fn progress_percent(&self) -> usize {
        match self.total_bytes {
            Some(total) if total > 0 => {
                let percent = (self.bytes_downloaded * 100) / total;
                percent.min(100)
            }
            _ => 0,
        }
    }

    /// Get bytes remaining
    pub fn bytes_remaining(&self) -> Option<usize> {
        self.total_bytes
            .map(|t| t.saturating_sub(self.bytes_downloaded))
    }

    /// Format progress as string (e.g., "150 MB / 500 MB")
    pub fn progress_string(&self) -> (&'static str, &'static str) {
        // Returns static strings for simplicity in no_std
        let downloaded = Self::size_bucket(self.bytes_downloaded);
        let total = self.total_bytes.map(Self::size_bucket).unwrap_or("???");
        (downloaded, total)
    }

    /// Bucket size into human readable string
    fn size_bucket(bytes: usize) -> &'static str {
        if bytes < 1024 * 1024 {
            "< 1 MB"
        } else if bytes < 10 * 1024 * 1024 {
            "1-10 MB"
        } else if bytes < 50 * 1024 * 1024 {
            "10-50 MB"
        } else if bytes < 100 * 1024 * 1024 {
            "50-100 MB"
        } else if bytes < 250 * 1024 * 1024 {
            "100-250 MB"
        } else if bytes < 500 * 1024 * 1024 {
            "250-500 MB"
        } else if bytes < 1024 * 1024 * 1024 {
            "500 MB - 1 GB"
        } else if bytes < 2 * 1024 * 1024 * 1024 {
            "1-2 GB"
        } else {
            "> 2 GB"
        }
    }
}

impl Default for DownloadState {
    fn default() -> Self {
        Self::new()
    }
}
