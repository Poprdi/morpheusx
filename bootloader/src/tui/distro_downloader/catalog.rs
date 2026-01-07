//! Distribution Catalog
//!
//! Catalog of popular Linux distributions available for download.
//! Includes official mirror URLs, file sizes, and metadata.

use alloc::string::String;

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
    /// Create a new distro entry
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

    /// Add mirrors
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

    /// Human-readable size string
    pub fn size_str(&self) -> &'static str {
        // Return approximate size category
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
}

/// Static catalog of available distributions
pub static DISTRO_CATALOG: &[DistroEntry] = &[
    // ============ Privacy ============
    DistroEntry::new(
        "Tails",
        "Amnesic Incognito Live System - Privacy focused",
        "6.10",
        "https://download.tails.net/tails/stable/tails-amd64-6.10/tails-amd64-6.10.iso",
        1_400_000_000, // ~1.4 GB
        "tails-6.10.iso",
        DistroCategory::Privacy,
    )
    .with_mirrors(&[
        "https://mirrors.edge.kernel.org/tails/stable/tails-amd64-6.10/tails-amd64-6.10.iso",
    ]),

    DistroEntry::new(
        "Whonix Gateway",
        "Tor gateway for anonymous networking",
        "17",
        "https://download.whonix.org/linux/17/Whonix-Gateway-Xfce-17.2.3.2.iso",
        1_200_000_000, // ~1.2 GB
        "whonix-gateway-17.iso",
        DistroCategory::Privacy,
    ),

    // ============ General Purpose ============
    DistroEntry::new(
        "Ubuntu Desktop",
        "Popular user-friendly Linux distribution",
        "24.04 LTS",
        "https://releases.ubuntu.com/24.04/ubuntu-24.04.1-desktop-amd64.iso",
        5_800_000_000, // ~5.8 GB
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
        2_600_000_000, // ~2.6 GB
        "ubuntu-24.04-server.iso",
        DistroCategory::Server,
    ),

    DistroEntry::new(
        "Fedora Workstation",
        "Cutting-edge Linux workstation",
        "41",
        "https://download.fedoraproject.org/pub/fedora/linux/releases/41/Workstation/x86_64/iso/Fedora-Workstation-Live-x86_64-41-1.4.iso",
        2_300_000_000, // ~2.3 GB
        "fedora-41-workstation.iso",
        DistroCategory::General,
    ),

    DistroEntry::new(
        "Debian",
        "The Universal Operating System - Stable",
        "12.8",
        "https://cdimage.debian.org/debian-cd/current/amd64/iso-cd/debian-12.8.0-amd64-netinst.iso",
        660_000_000, // ~660 MB (netinst)
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
        2_800_000_000, // ~2.8 GB
        "linuxmint-22.iso",
        DistroCategory::General,
    ),

    DistroEntry::new(
        "Arch Linux",
        "Lightweight and flexible - rolling release",
        "2024.12",
        "https://geo.mirror.pkgbuild.com/iso/latest/archlinux-x86_64.iso",
        1_100_000_000, // ~1.1 GB
        "archlinux-latest.iso",
        DistroCategory::General,
    ),

    // ============ Security/Pentest ============
    DistroEntry::new(
        "Kali Linux",
        "Penetration testing and security auditing",
        "2024.4",
        "https://cdimage.kali.org/kali-2024.4/kali-linux-2024.4-live-amd64.iso",
        4_000_000_000, // ~4 GB
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
        5_200_000_000, // ~5.2 GB
        "parrot-security-6.2.iso",
        DistroCategory::Security,
    ),

    // ============ Minimal ============
    DistroEntry::new(
        "Alpine Linux",
        "Security-oriented, lightweight distro",
        "3.21",
        "https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/x86_64/alpine-standard-3.21.0-x86_64.iso",
        220_000_000, // ~220 MB
        "alpine-3.21.iso",
        DistroCategory::Minimal,
    ),

    DistroEntry::new(
        "Tiny Core Linux",
        "Extremely minimal - runs in RAM",
        "15.0",
        "http://tinycorelinux.net/15.x/x86_64/release/TinyCorePure64-15.0.iso",
        25_000_000, // ~25 MB
        "tinycore-15.0.iso",
        DistroCategory::Minimal,
    ),

    DistroEntry::new(
        "Puppy Linux",
        "Complete OS that fits on small media",
        "FossaPup64",
        "https://distro.ibiblio.org/puppylinux/puppy-fossa/fossapup64-9.5.iso",
        430_000_000, // ~430 MB
        "fossapup64-9.5.iso",
        DistroCategory::Minimal,
    ),
];

/// Get distributions by category
pub fn get_by_category(category: DistroCategory) -> impl Iterator<Item = &'static DistroEntry> {
    DISTRO_CATALOG.iter().filter(move |d| d.category == category)
}

/// Get all category names
pub const CATEGORIES: &[DistroCategory] = &[
    DistroCategory::Privacy,
    DistroCategory::General,
    DistroCategory::Security,
    DistroCategory::Minimal,
    DistroCategory::Server,
];
