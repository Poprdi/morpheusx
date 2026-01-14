//! Download status enum

/// Download status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    /// No download in progress
    Idle,
    /// Checking file existence/size
    Checking,
    /// Download in progress
    Downloading,
    /// Download complete
    Complete,
    /// Download failed
    Failed,
}

impl DownloadStatus {
    /// Get display string for status
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Checking => "Checking...",
            Self::Downloading => "Downloading...",
            Self::Complete => "Complete",
            Self::Failed => "Failed",
        }
    }

    /// Check if download is active
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading)
    }

    /// Check if download is finished (success or failure)
    pub const fn is_finished(&self) -> bool {
        matches!(self, Self::Complete | Self::Failed)
    }
}
