//! UI mode

/// UI mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMode {
    /// Browsing distro list for download
    Browse,
    /// Showing confirmation dialog
    Confirm,
    /// Download in progress
    Downloading,
    /// Showing result (success/error)
    Result,
    /// Managing downloaded ISOs
    Manage,
    /// Confirm delete ISO
    ConfirmDelete,
}

impl UiMode {
    /// Get display string
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Browse => "Browse",
            Self::Confirm => "Confirm",
            Self::Downloading => "Downloading",
            Self::Result => "Result",
            Self::Manage => "Manage",
            Self::ConfirmDelete => "Confirm Delete",
        }
    }

    /// Check if in management submenu
    pub const fn is_manage_related(&self) -> bool {
        matches!(self, Self::Manage | Self::ConfirmDelete)
    }
}
