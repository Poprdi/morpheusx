//! Distribution Catalog
//!
//! Catalog of popular Linux distributions available for download.
//! Includes official mirror URLs, file sizes, and metadata.
//!
//! This module is pure Rust with no UEFI dependencies - fully unit testable.

/// Category of Linux distribution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistroCategory {
    /// Privacy-focused distributions (Tails, Whonix)
    Privacy,
    /// General purpose distributions (Ubuntu, Fedora)
    General,
    /// Security/Penetration testing (Kali, Parrot)
    Security,
    /// Minimal/Lightweight distributions (Alpine, Tiny Core)
    Minimal,
    /// Server distributions
    Server,
}

impl DistroCategory {
    /// Get display name for category
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Privacy => "Privacy",
            Self::General => "General Purpose",
            Self::Security => "Security/Pentest",
            Self::Minimal => "Minimal",
            Self::Server => "Server",
        }
    }

    /// Get short code for category (for UI)
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Privacy => "PRV",
            Self::General => "GEN",
            Self::Security => "SEC",
            Self::Minimal => "MIN",
            Self::Server => "SRV",
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
    // ============ TEST ENTRIES (HTTP - for development) ============
    DistroEntry::new(
        "Test ISO (Small)",
        "Small test file for network testing",
        "1.0",
        "http://speedtest.tele2.net/1MB.zip",  // HTTP test endpoint
        1_000_000,
        "test-1mb.iso",
        DistroCategory::Minimal,
    ),
    
    DistroEntry::new(
        "Test ISO (10MB)",
        "Medium test file for network testing",
        "1.0",
        "http://speedtest.tele2.net/10MB.zip",
        10_000_000,
        "test-10mb.iso",
        DistroCategory::Minimal,
    ),

    // ============ Privacy ============
    DistroEntry::new(
        "Tails",
        "Amnesic Incognito Live System - Privacy focused",
        "6.10",
        "http://mirror.fcix.net/tails/stable/tails-amd64-6.10/tails-amd64-6.10.iso",
        1_400_000_000,
        "tails-6.10.iso",
        DistroCategory::Privacy,
    )
    .with_mirrors(&[
        "http://ftp.acc.umu.se/mirror/tails.boum.org/tails/stable/tails-amd64-6.10/tails-amd64-6.10.iso",
    ]),

    DistroEntry::new(
        "Whonix Gateway",
        "Tor gateway for anonymous networking",
        "17",
        "https://download.whonix.org/linux/17/Whonix-Gateway-Xfce-17.2.3.2.iso",
        1_200_000_000,
        "whonix-gateway-17.iso",
        DistroCategory::Privacy,
    ),

    // ============ General Purpose ============
    DistroEntry::new(
        "Ubuntu Desktop",
        "Popular user-friendly Linux distribution",
        "24.04 LTS",
        "https://releases.ubuntu.com/24.04/ubuntu-24.04.1-desktop-amd64.iso",
        5_800_000_000,
        "ubuntu-24.04-desktop.iso",
        DistroCategory::General,
    )
    .with_mirrors(&[
        "https://mirror.us.leaseweb.net/ubuntu-releases/24.04/ubuntu-24.04.1-desktop-amd64.iso",
        "https://mirrors.kernel.org/ubuntu-releases/24.04/ubuntu-24.04.1-desktop-amd64.iso",
    ]),

    DistroEntry::new(
        "Ubuntu Server",
        "Ubuntu for servers - minimal install",
        "24.04 LTS",
        "https://releases.ubuntu.com/24.04/ubuntu-24.04.1-live-server-amd64.iso",
        2_600_000_000,
        "ubuntu-24.04-server.iso",
        DistroCategory::Server,
    ),

    DistroEntry::new(
        "Fedora Workstation",
        "Cutting-edge Linux workstation",
        "41",
        "https://download.fedoraproject.org/pub/fedora/linux/releases/41/Workstation/x86_64/iso/Fedora-Workstation-Live-x86_64-41-1.4.iso",
        2_300_000_000,
        "fedora-41-workstation.iso",
        DistroCategory::General,
    ),

    DistroEntry::new(
        "Debian",
        "The Universal Operating System - Stable",
        "12.8",
        "https://cdimage.debian.org/debian-cd/current/amd64/iso-cd/debian-12.8.0-amd64-netinst.iso",
        660_000_000,
        "debian-12.8-netinst.iso",
        DistroCategory::General,
    )
    .with_mirrors(&[
        "https://mirror.us.leaseweb.net/debian-cd/current/amd64/iso-cd/debian-12.8.0-amd64-netinst.iso",
    ]),

    DistroEntry::new(
        "Linux Mint",
        "Elegant, easy to use desktop",
        "22",
        "https://mirrors.kernel.org/linuxmint/stable/22/linuxmint-22-cinnamon-64bit.iso",
        2_800_000_000,
        "linuxmint-22.iso",
        DistroCategory::General,
    ),

    DistroEntry::new(
        "Arch Linux",
        "Lightweight and flexible - rolling release",
        "2024.12",
        "https://geo.mirror.pkgbuild.com/iso/latest/archlinux-x86_64.iso",
        1_100_000_000,
        "archlinux-latest.iso",
        DistroCategory::General,
    ),

    // ============ Security/Pentest ============
    DistroEntry::new(
        "Kali Linux",
        "Penetration testing and security auditing",
        "2024.4",
        "https://cdimage.kali.org/kali-2024.4/kali-linux-2024.4-live-amd64.iso",
        4_000_000_000,
        "kali-2024.4.iso",
        DistroCategory::Security,
    )
    .with_mirrors(&[
        "https://kali.download/base-images/kali-2024.4/kali-linux-2024.4-live-amd64.iso",
    ]),

    DistroEntry::new(
        "Parrot Security",
        "Security, development, and privacy",
        "6.2",
        "https://deb.parrot.sh/parrot/iso/6.2/Parrot-security-6.2_amd64.iso",
        5_200_000_000,
        "parrot-security-6.2.iso",
        DistroCategory::Security,
    ),

    // ============ Minimal ============
    DistroEntry::new(
        "Alpine Linux",
        "Security-oriented, lightweight distro",
        "3.21",
        "https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/x86_64/alpine-standard-3.21.0-x86_64.iso",
        220_000_000,
        "alpine-3.21.iso",
        DistroCategory::Minimal,
    ),

    DistroEntry::new(
        "Tiny Core Linux",
        "Extremely minimal - runs in RAM",
        "15.0",
        "http://tinycorelinux.net/15.x/x86_64/release/TinyCorePure64-15.0.iso",
        25_000_000,
        "tinycore-15.0.iso",
        DistroCategory::Minimal,
    ),

    DistroEntry::new(
        "Puppy Linux",
        "Complete OS that fits on small media",
        "FossaPup64",
        "https://distro.ibiblio.org/puppylinux/puppy-fossa/fossapup64-9.5.iso",
        430_000_000,
        "fossapup64-9.5.iso",
        DistroCategory::Minimal,
    ),
];

/// All available categories
pub static CATEGORIES: &[DistroCategory] = &[
    DistroCategory::Privacy,
    DistroCategory::General,
    DistroCategory::Security,
    DistroCategory::Minimal,
    DistroCategory::Server,
];

/// Get distributions by category
pub fn get_by_category(category: DistroCategory) -> impl Iterator<Item = &'static DistroEntry> {
    DISTRO_CATALOG.iter().filter(move |d| d.category == category)
}

/// Count distributions in a category
pub fn count_by_category(category: DistroCategory) -> usize {
    DISTRO_CATALOG.iter().filter(|d| d.category == category).count()
}

/// Find a distribution by name
pub fn find_by_name(name: &str) -> Option<&'static DistroEntry> {
    morpheus_core::logger::log("catalog::find_by_name()");
    let result = DISTRO_CATALOG.iter().find(|d| d.name == name);
    if result.is_some() {
        morpheus_core::logger::log("catalog::find_by_name() - found");
    } else {
        morpheus_core::logger::log("catalog::find_by_name() - not found");
    }
    result
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
    fn test_category_name_general() {
        assert_eq!(DistroCategory::General.name(), "General Purpose");
    }

    #[test]
    fn test_category_name_security() {
        assert_eq!(DistroCategory::Security.name(), "Security/Pentest");
    }

    #[test]
    fn test_category_name_minimal() {
        assert_eq!(DistroCategory::Minimal.name(), "Minimal");
    }

    #[test]
    fn test_category_name_server() {
        assert_eq!(DistroCategory::Server.name(), "Server");
    }

    #[test]
    fn test_category_codes() {
        assert_eq!(DistroCategory::Privacy.code(), "PRV");
        assert_eq!(DistroCategory::General.code(), "GEN");
        assert_eq!(DistroCategory::Security.code(), "SEC");
        assert_eq!(DistroCategory::Minimal.code(), "MIN");
        assert_eq!(DistroCategory::Server.code(), "SRV");
    }

    // --- DistroEntry Creation Tests ---

    #[test]
    fn test_entry_new_defaults() {
        let entry = DistroEntry::new(
            "Test",
            "A test distro",
            "1.0",
            "https://example.com/test.iso",
            500_000_000,
            "test.iso",
            DistroCategory::General,
        );

        assert_eq!(entry.name, "Test");
        assert_eq!(entry.description, "A test distro");
        assert_eq!(entry.version, "1.0");
        assert_eq!(entry.url, "https://example.com/test.iso");
        assert_eq!(entry.size_bytes, 500_000_000);
        assert_eq!(entry.filename, "test.iso");
        assert_eq!(entry.category, DistroCategory::General);
        assert_eq!(entry.arch, "x86_64"); // default
        assert!(entry.is_live); // default
        assert!(entry.mirrors.is_empty());
        assert!(entry.sha256.is_none());
    }

    #[test]
    fn test_entry_with_mirrors() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0",
            "https://primary.com/test.iso",
            100_000_000, "test.iso", DistroCategory::General,
        )
        .with_mirrors(&["https://mirror1.com/test.iso", "https://mirror2.com/test.iso"]);

        assert_eq!(entry.mirrors.len(), 2);
        assert_eq!(entry.mirrors[0], "https://mirror1.com/test.iso");
    }

    #[test]
    fn test_entry_with_sha256() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0",
            "https://example.com/test.iso",
            100_000_000, "test.iso", DistroCategory::Security,
        )
        .with_sha256("abcd1234");

        assert_eq!(entry.sha256, Some("abcd1234"));
    }

    #[test]
    fn test_entry_with_arch() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0",
            "https://example.com/test.iso",
            100_000_000, "test.iso", DistroCategory::Minimal,
        )
        .with_arch("aarch64");

        assert_eq!(entry.arch, "aarch64");
    }

    #[test]
    fn test_entry_with_live() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0",
            "https://example.com/test.iso",
            100_000_000, "test.iso", DistroCategory::Server,
        )
        .with_live(false);

        assert!(!entry.is_live);
    }

    // --- Size String Tests ---

    #[test]
    fn test_size_str_under_100mb() {
        let entry = DistroEntry::new(
            "Tiny", "Tiny", "1.0", "https://x.com/t.iso",
            50_000_000, "t.iso", DistroCategory::Minimal,
        );
        assert_eq!(entry.size_str(), "< 100 MB");
    }

    #[test]
    fn test_size_str_100_500mb() {
        let entry = DistroEntry::new(
            "Small", "Small", "1.0", "https://x.com/s.iso",
            250_000_000, "s.iso", DistroCategory::Minimal,
        );
        assert_eq!(entry.size_str(), "100-500 MB");
    }

    #[test]
    fn test_size_str_500mb_1gb() {
        let entry = DistroEntry::new(
            "Med", "Med", "1.0", "https://x.com/m.iso",
            750_000_000, "m.iso", DistroCategory::General,
        );
        assert_eq!(entry.size_str(), "500 MB - 1 GB");
    }

    #[test]
    fn test_size_str_1_2gb() {
        let entry = DistroEntry::new(
            "Large", "Large", "1.0", "https://x.com/l.iso",
            1_500_000_000, "l.iso", DistroCategory::General,
        );
        assert_eq!(entry.size_str(), "1-2 GB");
    }

    #[test]
    fn test_size_str_2_4gb() {
        let entry = DistroEntry::new(
            "XL", "XL", "1.0", "https://x.com/xl.iso",
            3_000_000_000, "xl.iso", DistroCategory::General,
        );
        assert_eq!(entry.size_str(), "2-4 GB");
    }

    #[test]
    fn test_size_str_over_4gb() {
        let entry = DistroEntry::new(
            "Huge", "Huge", "1.0", "https://x.com/h.iso",
            5_000_000_000, "h.iso", DistroCategory::General,
        );
        assert_eq!(entry.size_str(), "> 4 GB");
    }

    // --- URL Validation Tests ---

    #[test]
    fn test_valid_https_url() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0",
            "https://example.com/test.iso",
            100_000_000, "test.iso", DistroCategory::General,
        );
        assert!(entry.is_valid_url());
    }

    #[test]
    fn test_valid_http_url() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0",
            "http://example.com/test.iso",
            100_000_000, "test.iso", DistroCategory::General,
        );
        assert!(entry.is_valid_url());
    }

    #[test]
    fn test_invalid_url_ftp() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0",
            "ftp://example.com/test.iso",
            100_000_000, "test.iso", DistroCategory::General,
        );
        assert!(!entry.is_valid_url());
    }

    #[test]
    fn test_invalid_url_no_scheme() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0",
            "example.com/test.iso",
            100_000_000, "test.iso", DistroCategory::General,
        );
        assert!(!entry.is_valid_url());
    }

    // --- Filename Validation Tests ---

    #[test]
    fn test_valid_filename() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0", "https://x.com/test.iso",
            100_000_000, "test-1.0.iso", DistroCategory::General,
        );
        assert!(entry.is_valid_filename());
    }

    #[test]
    fn test_invalid_filename_no_iso() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0", "https://x.com/test.iso",
            100_000_000, "test.img", DistroCategory::General,
        );
        assert!(!entry.is_valid_filename());
    }

    #[test]
    fn test_invalid_filename_with_path() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0", "https://x.com/test.iso",
            100_000_000, "path/test.iso", DistroCategory::General,
        );
        assert!(!entry.is_valid_filename());
    }

    // --- URL Index Tests ---

    #[test]
    fn test_url_count_no_mirrors() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0", "https://x.com/test.iso",
            100_000_000, "test.iso", DistroCategory::General,
        );
        assert_eq!(entry.url_count(), 1);
    }

    #[test]
    fn test_url_count_with_mirrors() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0", "https://primary.com/test.iso",
            100_000_000, "test.iso", DistroCategory::General,
        )
        .with_mirrors(&["https://m1.com/test.iso", "https://m2.com/test.iso"]);

        assert_eq!(entry.url_count(), 3);
    }

    #[test]
    fn test_get_url_primary() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0", "https://primary.com/test.iso",
            100_000_000, "test.iso", DistroCategory::General,
        )
        .with_mirrors(&["https://mirror.com/test.iso"]);

        assert_eq!(entry.get_url(0), Some("https://primary.com/test.iso"));
    }

    #[test]
    fn test_get_url_mirror() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0", "https://primary.com/test.iso",
            100_000_000, "test.iso", DistroCategory::General,
        )
        .with_mirrors(&["https://mirror.com/test.iso"]);

        assert_eq!(entry.get_url(1), Some("https://mirror.com/test.iso"));
    }

    #[test]
    fn test_get_url_out_of_bounds() {
        let entry = DistroEntry::new(
            "Test", "Test", "1.0", "https://primary.com/test.iso",
            100_000_000, "test.iso", DistroCategory::General,
        );

        assert_eq!(entry.get_url(1), None);
        assert_eq!(entry.get_url(99), None);
    }

    // --- Catalog Tests ---

    #[test]
    fn test_catalog_not_empty() {
        assert!(!DISTRO_CATALOG.is_empty());
        assert!(DISTRO_CATALOG.len() >= 10, "Should have at least 10 distros");
    }

    #[test]
    fn test_catalog_all_valid_urls() {
        for entry in DISTRO_CATALOG.iter() {
            assert!(
                entry.is_valid_url(),
                "{} has invalid URL: {}",
                entry.name, entry.url
            );
        }
    }

    #[test]
    fn test_catalog_all_valid_filenames() {
        for entry in DISTRO_CATALOG.iter() {
            assert!(
                entry.is_valid_filename(),
                "{} has invalid filename: {}",
                entry.name, entry.filename
            );
        }
    }

    #[test]
    fn test_catalog_reasonable_sizes() {
        for entry in DISTRO_CATALOG.iter() {
            assert!(
                entry.size_bytes >= 10_000_000,
                "{} too small: {}",
                entry.name, entry.size_bytes
            );
            assert!(
                entry.size_bytes <= 15_000_000_000,
                "{} too large: {}",
                entry.name, entry.size_bytes
            );
        }
    }

    // --- Category Filter Tests ---

    #[test]
    fn test_categories_list() {
        assert_eq!(CATEGORIES.len(), 5);
    }

    #[test]
    fn test_get_by_category_privacy() {
        let count = get_by_category(DistroCategory::Privacy).count();
        assert!(count >= 1, "Should have privacy distros");
    }

    #[test]
    fn test_get_by_category_general() {
        let count = get_by_category(DistroCategory::General).count();
        assert!(count >= 3, "Should have multiple general distros");
    }

    #[test]
    fn test_get_by_category_filters_correctly() {
        for entry in get_by_category(DistroCategory::Minimal) {
            assert_eq!(entry.category, DistroCategory::Minimal);
        }
    }

    #[test]
    fn test_count_by_category() {
        let count = count_by_category(DistroCategory::Security);
        assert!(count >= 1);
    }

    // --- Find Tests ---

    #[test]
    fn test_find_by_name_exists() {
        let tails = find_by_name("Tails");
        assert!(tails.is_some());
        assert_eq!(tails.unwrap().category, DistroCategory::Privacy);
    }

    #[test]
    fn test_find_by_name_not_exists() {
        let result = find_by_name("NonExistentDistro");
        assert!(result.is_none());
    }

    #[test]
    fn test_find_by_filename() {
        let alpine = find_by_filename("alpine-3.21.iso");
        assert!(alpine.is_some());
        assert_eq!(alpine.unwrap().name, "Alpine Linux");
    }

    // --- Specific Distro Tests ---

    #[test]
    fn test_tails_entry() {
        let tails = find_by_name("Tails").unwrap();
        assert_eq!(tails.category, DistroCategory::Privacy);
        assert!(tails.url.contains("tails"));
        assert!(tails.size_bytes > 1_000_000_000); // > 1GB
        assert!(tails.mirrors.len() >= 1);
    }

    #[test]
    fn test_alpine_entry() {
        let alpine = find_by_name("Alpine Linux").unwrap();
        assert_eq!(alpine.category, DistroCategory::Minimal);
        assert!(alpine.size_bytes < 300_000_000); // < 300MB (it's small)
    }

    #[test]
    fn test_tinycore_entry() {
        let tinycore = find_by_name("Tiny Core Linux").unwrap();
        assert_eq!(tinycore.category, DistroCategory::Minimal);
        assert!(tinycore.size_bytes < 50_000_000); // < 50MB (very small)
    }

    #[test]
    fn test_kali_entry() {
        let kali = find_by_name("Kali Linux").unwrap();
        assert_eq!(kali.category, DistroCategory::Security);
        assert!(kali.size_bytes > 3_000_000_000); // > 3GB
    }
}
