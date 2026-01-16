//! Distribution Catalog
//!
//! Catalog of popular Linux distributions available for download.
//! Includes official mirror URLs, file sizes, and metadata.
//!
//! This module is pure Rust with no UEFI dependencies - fully unit testable.

/// Category of Linux distribution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistroCategory {
    Privacy,
    Security,
}

impl DistroCategory {
    /// Get display name for category
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Privacy => "Privacy",
            Self::Security => "Security/Pentest",
        }
    }

    /// Get short code for category (for UI)
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Privacy => "PRV",
            Self::Security => "SEC",
        }
    }
}

/// A downloadable distribution entry
#[derive(Debug, Clone)]
pub struct DistroEntry {
    /// Display name
    pub name: &'static str,
    /// Short description
    pub description: &'static str,
    /// Version string
    pub version: &'static str,
    /// Primary download URL
    pub url: &'static str,
    /// Mirror URLs (fallbacks)
    pub mirrors: &'static [&'static str],
    /// Expected file size in bytes (approximate)
    pub size_bytes: u64,
    /// Filename to save as
    pub filename: &'static str,
    /// SHA256 checksum (hex string, if known)
    pub sha256: Option<&'static str>,
    /// Category
    pub category: DistroCategory,
    /// Architecture (x86_64, aarch64, etc.)
    pub arch: &'static str,
    /// Whether this is a live ISO
    pub is_live: bool,
}

impl DistroEntry {
    /// Create a new distro entry with defaults
    pub const fn new(
        name: &'static str,
        description: &'static str,
        version: &'static str,
        url: &'static str,
        size_bytes: u64,
        filename: &'static str,
        category: DistroCategory,
    ) -> Self {
        Self {
            name,
            description,
            version,
            url,
            mirrors: &[],
            size_bytes,
            filename,
            sha256: None,
            category,
            arch: "x86_64",
            is_live: true,
        }
    }

    /// Add mirror URLs
    pub const fn with_mirrors(mut self, mirrors: &'static [&'static str]) -> Self {
        self.mirrors = mirrors;
        self
    }

    /// Add SHA256 checksum
    pub const fn with_sha256(mut self, sha256: &'static str) -> Self {
        self.sha256 = Some(sha256);
        self
    }

    /// Set architecture
    pub const fn with_arch(mut self, arch: &'static str) -> Self {
        self.arch = arch;
        self
    }

    /// Set live ISO flag
    pub const fn with_live(mut self, is_live: bool) -> Self {
        self.is_live = is_live;
        self
    }

    /// Human-readable size string
    pub fn size_str(&self) -> &'static str {
        if self.size_bytes < 100 * 1024 * 1024 {
            "< 100 MB"
        } else if self.size_bytes < 500 * 1024 * 1024 {
            "100-500 MB"
        } else if self.size_bytes < 1024 * 1024 * 1024 {
            "500 MB - 1 GB"
        } else if self.size_bytes < 2 * 1024 * 1024 * 1024 {
            "1-2 GB"
        } else if self.size_bytes < 4 * 1024 * 1024 * 1024 {
            "2-4 GB"
        } else {
            "> 4 GB"
        }
    }

    /// Check if URL is valid (basic check)
    pub fn is_valid_url(&self) -> bool {
        self.url.starts_with("http://") || self.url.starts_with("https://")
    }

    /// Check if filename is valid
    pub fn is_valid_filename(&self) -> bool {
        self.filename.ends_with(".iso") && !self.filename.contains('/')
    }

    /// Get the total number of available URLs (primary + mirrors)
    pub fn url_count(&self) -> usize {
        1 + self.mirrors.len()
    }

    /// Get URL by index (0 = primary, 1+ = mirrors)
    pub fn get_url(&self, index: usize) -> Option<&'static str> {
        if index == 0 {
            Some(self.url)
        } else {
            self.mirrors.get(index - 1).copied()
        }
    }
}

/// Static catalog of available distributions
pub static DISTRO_CATALOG: &[DistroEntry] = &[

    // ============ Privacy ============
    DistroEntry::new(
        "Tails",
        "Amnesic Incognito Live System - Privacy focused",
        "6.10",
        "http://mirror.freedif.org/Tails/tails/stable/tails-amd64-7.4/tails-amd64-7.4.iso",
        1_400_000_000,
        "tails-6.10.iso",
        DistroCategory::Privacy,
    )
    .with_mirrors(&[
        "http://ftp.acc.umu.se/mirror/tails.boum.org/tails/stable/tails-amd64-6.10/tails-amd64-6.10.iso",
    ]),

    DistroEntry::new(
        "Parrot OS",
        "Security and privacy focused",
        "5.0",
        "http://ftp.belnet.be/mirror/archive.parrotsec.org/parrot/iso/7.0/Parrot-security-7.0_amd64.iso",
        7_500_000_000,
        "parrot-5.0.iso",
        DistroCategory::Security,
    ),

    DistroEntry::new(
        "BlackArch Linux",
        "Penetration testing and security research",
        "2024.12.01",
        "http://blackarch.mirror.garr.it/mirrors/blackarch/iso/blackarch-linux-slim-2023.05.01-x86_64.iso",
        5_860_831_232,
        "blackarch-2024.12.01.iso",
        DistroCategory::Security,
    ),

    DistroEntry::new(
        "Kali Linux",
        "Penetration testing and security auditing",
        "2024.4",
        "http://mirror.vcu.edu/pub/gnu_linux/kali-images/current/kali-linux-2025.1a-live-amd64.iso",
        4_000_000_000,
        "kali-2025.1a.live.iso",
        DistroCategory::Security,
    )
    .with_mirrors(&[
        "http://mirror.vcu.edu/pub/gnu_linux/kali-images/current/kali-linux-2025.1a-live-amd64.iso",
    ]),

];

/// All available categories
pub static CATEGORIES: &[DistroCategory] = &[DistroCategory::Privacy, DistroCategory::Security];

/// Get distributions by category
pub fn get_by_category(category: DistroCategory) -> impl Iterator<Item = &'static DistroEntry> {
    DISTRO_CATALOG
        .iter()
        .filter(move |d| d.category == category)
}

/// Count distributions in a category
pub fn count_by_category(category: DistroCategory) -> usize {
    DISTRO_CATALOG
        .iter()
        .filter(|d| d.category == category)
        .count()
}

/// Find a distribution by filename
pub fn find_by_filename(filename: &str) -> Option<&'static DistroEntry> {
    morpheus_core::logger::log("catalog::find_by_filename()");
    let result = DISTRO_CATALOG.iter().find(|d| d.filename == filename);
    if result.is_some() {
        morpheus_core::logger::log("catalog::find_by_filename() - found");
    } else {
        morpheus_core::logger::log("catalog::find_by_filename() - not found");
    }
    result
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- DistroCategory Tests ---

    #[test]
    fn test_category_name_privacy() {
        assert_eq!(DistroCategory::Privacy.name(), "Privacy");
    }

    #[test]
    fn test_category_name_security() {
        assert_eq!(DistroCategory::Security.name(), "Security/Pentest");
    }

    #[test]
    fn test_category_name_server() {
        assert_eq!(DistroCategory::Server.name(), "Server");
    }

    #[test]
    fn test_category_codes() {
        assert_eq!(DistroCategory::Privacy.code(), "PRV");
        assert_eq!(DistroCategory::Security.code(), "SEC");
    }

    #[test]
    fn test_entry_with_sha256() {
        let entry = DistroEntry::new(
            "Test",
            "Test",
            "1.0",
            "https://example.com/test.iso",
            100_000_000,
            "test.iso",
            DistroCategory::Security,
        )
        .with_sha256("abcd1234");

        assert_eq!(entry.sha256, Some("abcd1234"));
    }

    #[test]
    fn test_entry_with_live() {
        let entry = DistroEntry::new(
            "Test",
            "Test",
            "1.0",
            "https://example.com/test.iso",
            100_000_000,
            "test.iso",
            DistroCategory::Server,
        )
        .with_live(false);

        assert!(!entry.is_live);
    }
}
