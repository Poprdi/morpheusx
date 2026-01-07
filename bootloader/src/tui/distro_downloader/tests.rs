//! Tests for Distro Downloader Module
//!
//! TDD tests - written before implementation.

#[cfg(test)]
mod catalog_tests {
    use super::super::catalog::*;

    #[test]
    fn test_distro_entry_creation() {
        let entry = DistroEntry::new(
            "Test Distro",
            "A test distribution",
            "1.0",
            "http://example.com/test.iso",
            1_000_000_000, // 1GB
            "test.iso",
            DistroCategory::General,
        );

        assert_eq!(entry.name, "Test Distro");
        assert_eq!(entry.description, "A test distribution");
        assert_eq!(entry.version, "1.0");
        assert_eq!(entry.url, "http://example.com/test.iso");
        assert_eq!(entry.size_bytes, 1_000_000_000);
        assert_eq!(entry.filename, "test.iso");
        assert_eq!(entry.category, DistroCategory::General);
        assert_eq!(entry.arch, "x86_64"); // default
        assert!(entry.is_live); // default
        assert!(entry.mirrors.is_empty());
        assert!(entry.sha256.is_none());
    }

    #[test]
    fn test_distro_entry_with_mirrors() {
        let entry = DistroEntry::new(
            "Test",
            "Test",
            "1.0",
            "http://primary.com/test.iso",
            500_000_000,
            "test.iso",
            DistroCategory::Privacy,
        )
        .with_mirrors(&["http://mirror1.com/test.iso", "http://mirror2.com/test.iso"]);

        assert_eq!(entry.mirrors.len(), 2);
        assert_eq!(entry.mirrors[0], "http://mirror1.com/test.iso");
        assert_eq!(entry.mirrors[1], "http://mirror2.com/test.iso");
    }

    #[test]
    fn test_distro_entry_with_sha256() {
        let entry = DistroEntry::new(
            "Test",
            "Test",
            "1.0",
            "http://example.com/test.iso",
            500_000_000,
            "test.iso",
            DistroCategory::Security,
        )
        .with_sha256("abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234");

        assert!(entry.sha256.is_some());
        assert_eq!(
            entry.sha256.unwrap(),
            "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234"
        );
    }

    #[test]
    fn test_distro_entry_with_arch() {
        let entry = DistroEntry::new(
            "Test",
            "Test",
            "1.0",
            "http://example.com/test.iso",
            500_000_000,
            "test.iso",
            DistroCategory::Minimal,
        )
        .with_arch("aarch64");

        assert_eq!(entry.arch, "aarch64");
    }

    #[test]
    fn test_category_names() {
        assert_eq!(DistroCategory::Privacy.name(), "Privacy");
        assert_eq!(DistroCategory::General.name(), "General Purpose");
        assert_eq!(DistroCategory::Security.name(), "Security/Pentest");
        assert_eq!(DistroCategory::Minimal.name(), "Minimal");
        assert_eq!(DistroCategory::Server.name(), "Server");
    }

    #[test]
    fn test_size_str_under_100mb() {
        let entry = DistroEntry::new(
            "Tiny",
            "Tiny distro",
            "1.0",
            "http://example.com/tiny.iso",
            50_000_000, // 50 MB
            "tiny.iso",
            DistroCategory::Minimal,
        );
        assert_eq!(entry.size_str(), "< 100 MB");
    }

    #[test]
    fn test_size_str_100_500mb() {
        let entry = DistroEntry::new(
            "Small",
            "Small distro",
            "1.0",
            "http://example.com/small.iso",
            250_000_000, // 250 MB
            "small.iso",
            DistroCategory::Minimal,
        );
        assert_eq!(entry.size_str(), "100-500 MB");
    }

    #[test]
    fn test_size_str_500mb_1gb() {
        let entry = DistroEntry::new(
            "Medium",
            "Medium distro",
            "1.0",
            "http://example.com/medium.iso",
            750_000_000, // 750 MB
            "medium.iso",
            DistroCategory::General,
        );
        assert_eq!(entry.size_str(), "500 MB - 1 GB");
    }

    #[test]
    fn test_size_str_1_2gb() {
        let entry = DistroEntry::new(
            "Large",
            "Large distro",
            "1.0",
            "http://example.com/large.iso",
            1_500_000_000, // 1.5 GB
            "large.iso",
            DistroCategory::General,
        );
        assert_eq!(entry.size_str(), "1-2 GB");
    }

    #[test]
    fn test_size_str_2_4gb() {
        let entry = DistroEntry::new(
            "XLarge",
            "Extra large distro",
            "1.0",
            "http://example.com/xlarge.iso",
            3_000_000_000, // 3 GB
            "xlarge.iso",
            DistroCategory::General,
        );
        assert_eq!(entry.size_str(), "2-4 GB");
    }

    #[test]
    fn test_size_str_over_4gb() {
        let entry = DistroEntry::new(
            "Huge",
            "Huge distro",
            "1.0",
            "http://example.com/huge.iso",
            5_000_000_000, // 5 GB
            "huge.iso",
            DistroCategory::General,
        );
        assert_eq!(entry.size_str(), "> 4 GB");
    }

    #[test]
    fn test_catalog_not_empty() {
        assert!(!DISTRO_CATALOG.is_empty());
    }

    #[test]
    fn test_catalog_has_privacy_distros() {
        let privacy_count = DISTRO_CATALOG
            .iter()
            .filter(|d| d.category == DistroCategory::Privacy)
            .count();
        assert!(privacy_count > 0, "Should have at least one privacy distro");
    }

    #[test]
    fn test_catalog_has_general_distros() {
        let general_count = DISTRO_CATALOG
            .iter()
            .filter(|d| d.category == DistroCategory::General)
            .count();
        assert!(general_count > 0, "Should have at least one general distro");
    }

    #[test]
    fn test_catalog_has_minimal_distros() {
        let minimal_count = DISTRO_CATALOG
            .iter()
            .filter(|d| d.category == DistroCategory::Minimal)
            .count();
        assert!(minimal_count > 0, "Should have at least one minimal distro");
    }

    #[test]
    fn test_catalog_all_have_valid_urls() {
        for entry in DISTRO_CATALOG.iter() {
            assert!(
                entry.url.starts_with("http://") || entry.url.starts_with("https://"),
                "Entry {} has invalid URL: {}",
                entry.name,
                entry.url
            );
        }
    }

    #[test]
    fn test_catalog_all_have_filenames() {
        for entry in DISTRO_CATALOG.iter() {
            assert!(
                entry.filename.ends_with(".iso"),
                "Entry {} filename should end with .iso: {}",
                entry.name,
                entry.filename
            );
        }
    }

    #[test]
    fn test_catalog_all_have_reasonable_sizes() {
        for entry in DISTRO_CATALOG.iter() {
            assert!(
                entry.size_bytes > 10_000_000, // > 10 MB
                "Entry {} has suspiciously small size: {}",
                entry.name,
                entry.size_bytes
            );
            assert!(
                entry.size_bytes < 20_000_000_000, // < 20 GB
                "Entry {} has suspiciously large size: {}",
                entry.name,
                entry.size_bytes
            );
        }
    }

    #[test]
    fn test_get_by_category() {
        let privacy_distros: Vec<_> = get_by_category(DistroCategory::Privacy).collect();
        assert!(!privacy_distros.is_empty());
        for distro in privacy_distros {
            assert_eq!(distro.category, DistroCategory::Privacy);
        }
    }

    #[test]
    fn test_categories_list() {
        assert_eq!(CATEGORIES.len(), 5);
        assert!(CATEGORIES.contains(&DistroCategory::Privacy));
        assert!(CATEGORIES.contains(&DistroCategory::General));
        assert!(CATEGORIES.contains(&DistroCategory::Security));
        assert!(CATEGORIES.contains(&DistroCategory::Minimal));
        assert!(CATEGORIES.contains(&DistroCategory::Server));
    }

    #[test]
    fn test_tails_in_catalog() {
        let tails = DISTRO_CATALOG.iter().find(|d| d.name == "Tails");
        assert!(tails.is_some(), "Tails should be in catalog");
        let tails = tails.unwrap();
        assert_eq!(tails.category, DistroCategory::Privacy);
        assert!(tails.url.contains("tails"));
    }

    #[test]
    fn test_alpine_in_catalog() {
        let alpine = DISTRO_CATALOG.iter().find(|d| d.name == "Alpine Linux");
        assert!(alpine.is_some(), "Alpine should be in catalog");
        let alpine = alpine.unwrap();
        assert_eq!(alpine.category, DistroCategory::Minimal);
        assert!(alpine.size_bytes < 300_000_000, "Alpine should be small");
    }
}

#[cfg(test)]
mod download_state_tests {
    use super::super::state::*;

    #[test]
    fn test_download_state_initial() {
        let state = DownloadState::new();
        assert!(matches!(state.status, DownloadStatus::Idle));
        assert_eq!(state.bytes_downloaded, 0);
        assert!(state.total_bytes.is_none());
        assert!(state.error_message.is_none());
    }

    #[test]
    fn test_download_state_start() {
        let mut state = DownloadState::new();
        state.start("test.iso", Some(1_000_000));
        
        assert!(matches!(state.status, DownloadStatus::Downloading));
        assert_eq!(state.current_file, Some("test.iso"));
        assert_eq!(state.total_bytes, Some(1_000_000));
        assert_eq!(state.bytes_downloaded, 0);
    }

    #[test]
    fn test_download_state_progress() {
        let mut state = DownloadState::new();
        state.start("test.iso", Some(1_000_000));
        state.update_progress(500_000);
        
        assert_eq!(state.bytes_downloaded, 500_000);
        assert_eq!(state.progress_percent(), 50);
    }

    #[test]
    fn test_download_state_progress_unknown_total() {
        let mut state = DownloadState::new();
        state.start("test.iso", None);
        state.update_progress(500_000);
        
        assert_eq!(state.bytes_downloaded, 500_000);
        assert_eq!(state.progress_percent(), 0); // Unknown total = 0%
    }

    #[test]
    fn test_download_state_complete() {
        let mut state = DownloadState::new();
        state.start("test.iso", Some(1_000_000));
        state.update_progress(1_000_000);
        state.complete();
        
        assert!(matches!(state.status, DownloadStatus::Complete));
        assert_eq!(state.progress_percent(), 100);
    }

    #[test]
    fn test_download_state_error() {
        let mut state = DownloadState::new();
        state.start("test.iso", Some(1_000_000));
        state.fail("Connection timeout");
        
        assert!(matches!(state.status, DownloadStatus::Failed));
        assert_eq!(state.error_message, Some("Connection timeout"));
    }

    #[test]
    fn test_download_state_reset() {
        let mut state = DownloadState::new();
        state.start("test.iso", Some(1_000_000));
        state.update_progress(500_000);
        state.reset();
        
        assert!(matches!(state.status, DownloadStatus::Idle));
        assert_eq!(state.bytes_downloaded, 0);
        assert!(state.total_bytes.is_none());
        assert!(state.current_file.is_none());
    }
}

#[cfg(test)]
mod ui_state_tests {
    use super::super::state::*;
    use super::super::catalog::*;

    #[test]
    fn test_ui_state_initial() {
        let state = UiState::new();
        assert_eq!(state.selected_category, 0);
        assert_eq!(state.selected_distro, 0);
        assert_eq!(state.scroll_offset, 0);
        assert!(matches!(state.mode, UiMode::Browse));
    }

    #[test]
    fn test_ui_state_next_category() {
        let mut state = UiState::new();
        let num_categories = CATEGORIES.len();
        
        state.next_category(num_categories);
        assert_eq!(state.selected_category, 1);
        
        // Selection should reset when changing category
        assert_eq!(state.selected_distro, 0);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn test_ui_state_next_category_wraps() {
        let mut state = UiState::new();
        let num_categories = CATEGORIES.len();
        
        // Go to last category
        for _ in 0..num_categories - 1 {
            state.next_category(num_categories);
        }
        assert_eq!(state.selected_category, num_categories - 1);
        
        // Should not wrap (stay at last)
        state.next_category(num_categories);
        assert_eq!(state.selected_category, num_categories - 1);
    }

    #[test]
    fn test_ui_state_prev_category() {
        let mut state = UiState::new();
        let num_categories = CATEGORIES.len();
        
        state.next_category(num_categories);
        state.next_category(num_categories);
        assert_eq!(state.selected_category, 2);
        
        state.prev_category();
        assert_eq!(state.selected_category, 1);
    }

    #[test]
    fn test_ui_state_prev_category_at_zero() {
        let mut state = UiState::new();
        assert_eq!(state.selected_category, 0);
        
        state.prev_category();
        assert_eq!(state.selected_category, 0); // Should stay at 0
    }

    #[test]
    fn test_ui_state_next_distro() {
        let mut state = UiState::new();
        let num_distros = 10;
        let visible = 5;
        
        state.next_distro(num_distros, visible);
        assert_eq!(state.selected_distro, 1);
    }

    #[test]
    fn test_ui_state_next_distro_scrolls() {
        let mut state = UiState::new();
        let num_distros = 10;
        let visible = 5;
        
        // Move to item 5 (should trigger scroll)
        for _ in 0..5 {
            state.next_distro(num_distros, visible);
        }
        
        assert_eq!(state.selected_distro, 5);
        assert!(state.scroll_offset > 0);
    }

    #[test]
    fn test_ui_state_prev_distro() {
        let mut state = UiState::new();
        let num_distros = 10;
        let visible = 5;
        
        state.next_distro(num_distros, visible);
        state.next_distro(num_distros, visible);
        assert_eq!(state.selected_distro, 2);
        
        state.prev_distro();
        assert_eq!(state.selected_distro, 1);
    }

    #[test]
    fn test_ui_state_prev_distro_at_zero() {
        let mut state = UiState::new();
        assert_eq!(state.selected_distro, 0);
        
        state.prev_distro();
        assert_eq!(state.selected_distro, 0);
    }

    #[test]
    fn test_ui_state_mode_transitions() {
        let mut state = UiState::new();
        assert!(matches!(state.mode, UiMode::Browse));
        
        state.start_download();
        assert!(matches!(state.mode, UiMode::Downloading));
        
        state.show_confirm();
        assert!(matches!(state.mode, UiMode::Confirm));
        
        state.return_to_browse();
        assert!(matches!(state.mode, UiMode::Browse));
    }

    #[test]
    fn test_ui_state_get_current_category() {
        let state = UiState::new();
        let category = state.current_category();
        assert_eq!(category, CATEGORIES[0]);
    }
}
