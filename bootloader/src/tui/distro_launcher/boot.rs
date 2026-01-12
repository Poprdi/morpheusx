use super::entry::BootEntry;
use super::ui::DistroLauncher;
use crate::boot::loader::BootError;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{
    Screen, EFI_BLACK, EFI_CYAN, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED, EFI_YELLOW,
};
use crate::uefi::file_system::FileProtocol;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const MAX_KERNEL_BYTES: usize = 64 * 1024 * 1024;
const PAGE_SIZE: usize = 4096;

impl DistroLauncher {
    pub(super) fn boot_entry(
        &self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        boot_services: &crate::BootServices,
        system_table: *mut (),
        image_handle: *mut (),
        entry: &BootEntry,
    ) {
        screen.clear();

        if entry.cmdline.starts_with("iso:") {
            self.boot_from_iso(
                screen,
                keyboard,
                boot_services,
                system_table,
                image_handle,
                entry,
            );
            return;
        }

        if entry.cmdline.starts_with("chunked_iso:") {
            self.boot_from_chunked_iso(
                screen,
                keyboard,
                boot_services,
                system_table,
                image_handle,
                entry,
            );
            return;
        }

        screen.put_str_at(5, 10, "Loading kernel...", EFI_LIGHTGREEN, EFI_BLACK);

        morpheus_core::logger::log("kernel read start");
        let (kernel_ptr, kernel_size) = match Self::read_file_to_uefi_pages(
            boot_services,
            image_handle,
            &entry.kernel_path,
            screen,
            MAX_KERNEL_BYTES,
            12,
        ) {
            Ok((ptr, size)) => {
                morpheus_core::logger::log("kernel read ok");
                (ptr, size)
            }
            Err(e) => {
                screen.put_str_at(5, 18, "ERROR: Failed to read kernel", EFI_RED, EFI_BLACK);
                morpheus_core::logger::log("kernel read failed");
                Self::dump_logs_to_screen(screen);
                keyboard.wait_for_key();
                return;
            }
        };

        // Load initrd using same GRUB-style approach
        morpheus_core::logger::log("BEFORE initrd match");
        let (initrd_ptr, initrd_size) = match &entry.initrd_path {
            Some(path) => {
                morpheus_core::logger::log("initrd path found");

                match Self::read_file_to_uefi_pages(
                    boot_services,
                    image_handle,
                    path,
                    screen,
                    512 * 1024 * 1024,
                    19,
                ) {
                    Ok((ptr, size)) => {
                        morpheus_core::logger::log("initrd read ok");
                        (Some(ptr), size)
                    }
                    Err(e) => {
                        screen.put_str_at(
                            5,
                            25,
                            "ERROR: Failed to read initrd",
                            EFI_RED,
                            EFI_BLACK,
                        );
                        morpheus_core::logger::log("failed to read initrd");
                        Self::dump_logs_to_screen(screen);
                        keyboard.wait_for_key();
                        return;
                    }
                }
            }
            None => {
                screen.put_str_at(5, 19, "No initrd found", EFI_DARKGREEN, EFI_BLACK);
                morpheus_core::logger::log("initrd not found");
                (None, 0)
            }
        };

        screen.put_str_at(5, 18, "Booting......", EFI_LIGHTGREEN, EFI_BLACK);

        // Boot the kernel - convert raw pointers to slices (GRUB style)
        let boot_result = unsafe {
            // Clear screen area for logs
            for i in 18..30 {
                screen.put_str_at(
                    5,
                    i,
                    "                                                    ",
                    EFI_BLACK,
                    EFI_BLACK,
                );
            }

            // Convert pointers to slices for boot call
            let kernel_slice = core::slice::from_raw_parts(kernel_ptr, kernel_size);
            let initrd_slice = initrd_ptr.map(|ptr| core::slice::from_raw_parts(ptr, initrd_size));

            crate::boot::loader::boot_linux_kernel(
                boot_services,
                system_table,
                image_handle,
                kernel_slice,
                initrd_slice,
                &entry.cmdline,
                screen,
            )
        };

        if let Err(error) = boot_result {
            let detail = Self::describe_boot_error(&error);
            let msg = alloc::format!("ERROR: {}", detail);
            Self::await_failure(screen, keyboard, 18, &msg, "kernel boot failed");
        }
    }

    fn boot_from_iso(
        &self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        boot_services: &crate::BootServices,
        system_table: *mut (),
        image_handle: *mut (),
        entry: &BootEntry,
    ) {
        use crate::tui::boot_sequence::BootSequence;
        use crate::tui::widgets::progressbar::ProgressBar;

        screen.clear();
        screen.put_str_at(5, 2, "Booting from ISO", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(5, 3, "================", EFI_GREEN, EFI_BLACK);

        let mut boot_seq = BootSequence::new();
        let mut progress_bar = ProgressBar::new(5, 5, 60, "Loading ISO:");

        morpheus_core::logger::log("ISO Boot: Starting...");
        boot_seq.render(screen, 5, 8);
        progress_bar.render(screen);

        let esp_root = match unsafe { Self::get_esp_root(boot_services, image_handle) } {
            Ok(root) => root,
            Err(_) => {
                morpheus_core::logger::log("ISO Boot: FAILED to access ESP");
                boot_seq.render(screen, 5, 8);
                keyboard.wait_for_key();
                return;
            }
        };

        let iso_path = entry.cmdline.strip_prefix("iso:").unwrap_or(&entry.cmdline);

        // Use progress callback to continuously update screen
        let mut last_log_count = morpheus_core::logger::total_log_count();
        let mut last_percent = 0usize;
        let mut progress_callback = |bytes: usize, total: usize, _msg: &str| {
            if total > 0 {
                let percent = (bytes * 100) / total;
                if percent != last_percent {
                    progress_bar.set_progress(percent);
                    progress_bar.render(screen);
                    last_percent = percent;
                }
            }

            let current_log_count = morpheus_core::logger::total_log_count();
            if current_log_count != last_log_count {
                boot_seq.render(screen, 5, 8);
                last_log_count = current_log_count;
            }
        };

        // The extract function logs its own progress
        let (kernel_data, initrd_data, cmdline) = match super::iso_boot::extract_iso_with_progress(
            iso_path,
            esp_root,
            Some(&mut progress_callback),
        ) {
            Ok(files) => {
                progress_bar.set_progress(100);
                progress_bar.render(screen);
                boot_seq.render(screen, 5, 8);
                files
            }
            Err(e) => {
                morpheus_core::logger::log(alloc::format!("ISO Boot: FAILED - {:?}", e).leak());
                boot_seq.render(screen, 5, 8);
                unsafe {
                    ((*esp_root).close)(esp_root);
                }
                keyboard.wait_for_key();
                return;
            }
        };

        unsafe {
            ((*esp_root).close)(esp_root);
        }

        morpheus_core::logger::log("ISO Boot: Launching kernel...");
        boot_seq.render(screen, 5, 8);

        let boot_result = unsafe {
            let kernel_slice = core::slice::from_raw_parts(kernel_data.as_ptr(), kernel_data.len());
            let initrd_slice = initrd_data
                .as_ref()
                .map(|d| core::slice::from_raw_parts(d.as_ptr(), d.len()));

            crate::boot::loader::boot_linux_kernel(
                boot_services,
                system_table,
                image_handle,
                kernel_slice,
                initrd_slice,
                &cmdline,
                screen,
            )
        };

        if let Err(error) = boot_result {
            let detail = Self::describe_boot_error(&error);
            let msg = alloc::format!("ERROR: {}", detail);
            Self::await_failure(screen, keyboard, 18, &msg, "iso boot failed");
        }
    }

    /// Boot from a chunked ISO stored via IsoStorageManager
    fn boot_from_chunked_iso(
        &self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        boot_services: &crate::BootServices,
        system_table: *mut (),
        image_handle: *mut (),
        entry: &BootEntry,
    ) {
        use crate::tui::boot_sequence::BootSequence;
        use crate::tui::widgets::progressbar::ProgressBar;
        use crate::uefi::block_io_adapter::UefiBlockIo;
        use morpheus_core::iso::{IsoBlockIoAdapter, IsoStorageManager};

        screen.clear();
        screen.put_str_at(5, 2, "Booting from Chunked ISO", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(5, 3, "========================", EFI_GREEN, EFI_BLACK);

        let mut boot_seq = BootSequence::new();
        let mut progress_bar = ProgressBar::new(5, 5, 60, "Loading ISO:");
        boot_seq.render(screen, 5, 8);
        progress_bar.render(screen);

        morpheus_core::logger::log("Chunked ISO Boot: Starting...");

        // Parse index from cmdline: "chunked_iso:N"
        let idx_str = entry.cmdline.strip_prefix("chunked_iso:").unwrap_or("0");
        let iso_idx: usize = idx_str.parse().unwrap_or(0);

        morpheus_core::logger::log(format!("Chunked ISO: index={}", iso_idx).leak());

        // Get disk info
        let (esp_lba, disk_lba) = {
            let mut dm = morpheus_core::disk::manager::DiskManager::new();
            if crate::uefi::disk::enumerate_disks(boot_services, &mut dm).is_ok()
                && dm.disk_count() > 0
            {
                if let Some(disk) = dm.get_disk(0) {
                    (2048_u64, disk.last_block + 1)
                } else {
                    screen.put_str_at(5, 18, "ERROR: No disk found", EFI_RED, EFI_BLACK);
                    keyboard.wait_for_key();
                    return;
                }
            } else {
                screen.put_str_at(5, 18, "ERROR: Disk enumeration failed", EFI_RED, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        // Get ISO read context - load manifests from ESP first
        let mut storage = IsoStorageManager::new(esp_lba, disk_lba);

        // Load persisted manifests from ESP filesystem
        if let Err(_) = unsafe {
            crate::tui::distro_downloader::manifest_io::load_manifests_from_esp(
                boot_services,
                image_handle,
                &mut storage,
            )
        } {
            morpheus_core::logger::log("Warning: Could not load manifests from ESP");
        }

        let read_ctx = match storage.get_read_context(iso_idx) {
            Ok(ctx) => ctx,
            Err(_) => {
                screen.put_str_at(5, 18, "ERROR: ISO not found in storage", EFI_RED, EFI_BLACK);
                screen.put_str_at(
                    5,
                    19,
                    &format!("Index {} not in {} ISOs", iso_idx, storage.count()),
                    EFI_YELLOW,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
                return;
            }
        };

        morpheus_core::logger::log(
            format!(
                "Chunked ISO: {} chunks, {} bytes",
                read_ctx.num_chunks, read_ctx.total_size
            )
            .leak(),
        );

        // Debug: Log chunk partition LBAs to diagnose mount issues
        // Also find the max LBA we need to read
        let mut max_lba_needed: u64 = 0;
        for i in 0..read_ctx.num_chunks {
            let (start_lba, end_lba) = read_ctx.chunk_lbas[i];
            let size = read_ctx.chunk_sizes[i];
            morpheus_core::logger::log(
                format!(
                    "  Chunk {}: LBA {}..{}, size {} bytes",
                    i, start_lba, end_lba, size
                )
                .leak(),
            );
            if end_lba > max_lba_needed {
                max_lba_needed = end_lba;
            }
        }

        // Get block I/O protocol for disk containing the ISO data
        // We need a disk large enough to contain max_lba_needed
        let block_io_protocol = match Self::get_disk_for_lba(boot_services, max_lba_needed) {
            Some(p) => p,
            None => {
                screen.put_str_at(
                    5,
                    18,
                    "ERROR: No disk contains ISO data",
                    EFI_RED,
                    EFI_BLACK,
                );
                screen.put_str_at(
                    5,
                    19,
                    &format!("Need disk with LBA >= {}", max_lba_needed),
                    EFI_YELLOW,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
                return;
            }
        };

        // Create UEFI block I/O adapter
        // SAFETY: Protocol pointer is valid from get_first_disk_block_io
        let mut uefi_block_io = unsafe { UefiBlockIo::new(block_io_protocol) };

        // DEBUG: Log block I/O info
        morpheus_core::logger::log(
            format!(
                "UefiBlockIo: block_size={}, total_blocks={}",
                uefi_block_io.block_size_bytes(),
                uefi_block_io.total_blocks()
            )
            .leak(),
        );

        // Create ISO adapter that bridges chunked storage to iso9660
        let mut iso_adapter = IsoBlockIoAdapter::new(read_ctx, &mut uefi_block_io);

        // DEBUG: Clear screen and show debug info
        screen.clear();
        screen.put_str_at(5, 2, "=== ISO MOUNT DEBUG ===", EFI_YELLOW, EFI_BLACK);
        {
            use gpt_disk_io::BlockIo;
            let mut test_buf = [0u8; 2048];

            // Test read sectors 16, 17, 18 (volume descriptors)
            for (row, sector) in [(4, 16u64), (6, 17u64), (8, 18u64)] {
                let label = format!("Sector {}: ", sector);
                if let Err(_e) = iso_adapter.read_blocks(gpt_disk_types::Lba(sector), &mut test_buf)
                {
                    screen.put_str_at(5, row, &format!("{}READ FAILED", label), EFI_RED, EFI_BLACK);
                } else {
                    let msg = format!(
                        "{}{:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                        label,
                        test_buf[0],
                        test_buf[1],
                        test_buf[2],
                        test_buf[3],
                        test_buf[4],
                        test_buf[5],
                        test_buf[6],
                        test_buf[7]
                    );
                    let color = if &test_buf[1..6] == b"CD001" {
                        EFI_LIGHTGREEN
                    } else {
                        EFI_YELLOW
                    };
                    screen.put_str_at(5, row, &msg, color, EFI_BLACK);
                }
            }

            // Also test reading sector 16 AGAIN to check for state issues
            screen.put_str_at(5, 10, "Re-read sector 16...", EFI_CYAN, EFI_BLACK);
            if let Err(_e) = iso_adapter.read_blocks(gpt_disk_types::Lba(16), &mut test_buf) {
                screen.put_str_at(5, 11, "Re-read FAILED!", EFI_RED, EFI_BLACK);
            } else {
                let msg = format!(
                    "Re-read: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                    test_buf[0],
                    test_buf[1],
                    test_buf[2],
                    test_buf[3],
                    test_buf[4],
                    test_buf[5],
                    test_buf[6],
                    test_buf[7]
                );
                let color = if &test_buf[1..6] == b"CD001" {
                    EFI_LIGHTGREEN
                } else {
                    EFI_RED
                };
                screen.put_str_at(5, 11, &msg, color, EFI_BLACK);
            }

            screen.put_str_at(
                5,
                13,
                "Press any key to attempt mount...",
                EFI_YELLOW,
                EFI_BLACK,
            );
            keyboard.wait_for_key();
        }

        // Mount ISO filesystem
        screen.put_str_at(5, 15, "Calling iso9660::mount()...", EFI_CYAN, EFI_BLACK);

        let volume = match iso9660::mount(&mut iso_adapter, 0) {
            Ok(v) => {
                screen.put_str_at(5, 15, "Mount: SUCCESS!", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(
                    5,
                    17,
                    "Press any key to continue boot...",
                    EFI_YELLOW,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
                v
            }
            Err(e) => {
                screen.put_str_at(5, 15, "Mount: FAILED!", EFI_RED, EFI_BLACK);
                screen.put_str_at(5, 17, &format!("Error: {:?}", e), EFI_YELLOW, EFI_BLACK);
                screen.put_str_at(5, 19, "Press any key to exit...", EFI_YELLOW, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        // Clear debug and continue
        screen.clear();

        progress_bar.set_progress(15);
        progress_bar.render(screen);
        morpheus_core::logger::log("Chunked ISO: Looking for kernel...");
        screen.put_str_at(5, 7, "Searching for kernel...", EFI_CYAN, EFI_BLACK);

        // Find kernel - check common paths for various distros
        // Tails: /live/vmlinuz
        // Ubuntu: /casper/vmlinuz
        // Debian: /live/vmlinuz-*
        // Fedora: /images/pxeboot/vmlinuz or /isolinux/vmlinuz
        let kernel_paths = [
            "/vmlinuz",                  // Puppy Linux, root-based distros
            "/live/vmlinuz",             // Tails, Debian Live
            "/casper/vmlinuz",           // Ubuntu
            "/boot/vmlinuz",             // Alpine, some others
            "/isolinux/vmlinuz",         // Generic syslinux
            "/images/pxeboot/vmlinuz",   // Fedora
            "/boot/x86_64/loader/linux", // openSUSE
        ];

        let mut kernel_data: Option<(u64, usize, usize)> = None;
        for (idx, kpath) in kernel_paths.iter().enumerate() {
            morpheus_core::logger::log(format!("Trying kernel path: {}", kpath).leak());
            screen.put_str_at(
                5,
                7,
                &format!("Trying: {}        ", kpath),
                EFI_CYAN,
                EFI_BLACK,
            );

            if let Ok(file_entry) = iso9660::find_file(&mut iso_adapter, &volume, kpath) {
                let size_mb = file_entry.size / (1024 * 1024);
                morpheus_core::logger::log(
                    format!(
                        "Found kernel at {} ({} MB, LBA {})",
                        kpath, size_mb, file_entry.extent_lba
                    )
                    .leak(),
                );
                screen.put_str_at(
                    5,
                    7,
                    &format!("Loading: {} ({} MB)    ", kpath, size_mb),
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                progress_bar.set_progress(20);
                progress_bar.render(screen);

                morpheus_core::logger::log("Starting kernel read...");

                // Allocate UEFI pages for kernel (Vec allocation fails - heap is only 1MB)
                let file_size = file_entry.size as usize;
                const PAGE_SIZE: usize = 4096;
                let pages_needed = (file_size + PAGE_SIZE - 1) / PAGE_SIZE;
                let mut buffer_addr = 0u64;

                let alloc_status = (boot_services.allocate_pages)(
                    0, // EFI_ALLOCATE_ANY_PAGES
                    2, // EFI_LOADER_DATA
                    pages_needed,
                    &mut buffer_addr,
                );

                if alloc_status != 0 || buffer_addr == 0 {
                    morpheus_core::logger::log("ERROR: Failed to allocate pages for kernel");
                    continue;
                }

                // Read kernel directly into UEFI pages
                let buffer = unsafe {
                    core::slice::from_raw_parts_mut(
                        buffer_addr as *mut u8,
                        pages_needed * PAGE_SIZE,
                    )
                };

                match iso9660::read_file(&mut iso_adapter, &file_entry, buffer) {
                    Ok(bytes_read) => {
                        morpheus_core::logger::log(
                            format!("Kernel loaded: {} bytes", bytes_read).leak(),
                        );
                        // Create owned Vec from the UEFI pages (we'll need to track pages for cleanup)
                        // For now, just use the buffer directly - it stays valid until we boot or fail
                        kernel_data = Some((buffer_addr, file_size, pages_needed));
                        break;
                    }
                    Err(_) => {
                        morpheus_core::logger::log("ERROR: Failed to read kernel file");
                        (boot_services.free_pages)(buffer_addr, pages_needed);
                    }
                }
            }
        }

        let (kernel_addr, kernel_size, kernel_pages) = match kernel_data {
            Some(d) => d,
            None => {
                screen.put_str_at(5, 18, "ERROR: No kernel found in ISO", EFI_RED, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        progress_bar.set_progress(40);
        progress_bar.render(screen);
        morpheus_core::logger::log("Chunked ISO: Looking for initrd...");

        // Find initrd - check common paths for various distros
        // Tails: /live/initrd.img
        // Ubuntu: /casper/initrd (no extension)
        // Debian: /live/initrd.img-*
        let initrd_paths = [
            "/initrd.gz",                 // Puppy Linux
            "/live/initrd.img",           // Tails, Debian Live
            "/casper/initrd",             // Ubuntu (no extension)
            "/casper/initrd.lz",          // Ubuntu compressed
            "/boot/initrd.img",           // Alpine
            "/isolinux/initrd.img",       // Generic syslinux
            "/images/pxeboot/initrd.img", // Fedora
            "/boot/x86_64/loader/initrd", // openSUSE
        ];

        let mut initrd_data: Option<(u64, usize, usize)> = None;
        for ipath in &initrd_paths {
            if let Ok(file_entry) = iso9660::find_file(&mut iso_adapter, &volume, ipath) {
                let size_mb = file_entry.size / (1024 * 1024);
                morpheus_core::logger::log(
                    format!("Found initrd at {} ({} MB)", ipath, size_mb).leak(),
                );
                screen.put_str_at(
                    5,
                    7,
                    &format!("Loading initrd: {} ({} MB)   ", ipath, size_mb),
                    EFI_CYAN,
                    EFI_BLACK,
                );
                progress_bar.set_progress(50);
                progress_bar.render(screen);

                // Allocate UEFI pages for initrd
                let file_size = file_entry.size as usize;
                const PAGE_SIZE: usize = 4096;
                let pages_needed = (file_size + PAGE_SIZE - 1) / PAGE_SIZE;
                let mut buffer_addr = 0u64;

                let alloc_status = (boot_services.allocate_pages)(
                    0, // EFI_ALLOCATE_ANY_PAGES
                    2, // EFI_LOADER_DATA
                    pages_needed,
                    &mut buffer_addr,
                );

                if alloc_status != 0 || buffer_addr == 0 {
                    morpheus_core::logger::log("ERROR: Failed to allocate pages for initrd");
                    continue;
                }

                let buffer = unsafe {
                    core::slice::from_raw_parts_mut(
                        buffer_addr as *mut u8,
                        pages_needed * PAGE_SIZE,
                    )
                };

                match iso9660::read_file(&mut iso_adapter, &file_entry, buffer) {
                    Ok(bytes_read) => {
                        morpheus_core::logger::log(
                            format!("Initrd loaded: {} bytes", bytes_read).leak(),
                        );
                        initrd_data = Some((buffer_addr, file_size, pages_needed));
                        break;
                    }
                    Err(_) => {
                        morpheus_core::logger::log("ERROR: Failed to read initrd file");
                        (boot_services.free_pages)(buffer_addr, pages_needed);
                    }
                }
            }
        }

        progress_bar.set_progress(80);
        progress_bar.render(screen);
        morpheus_core::logger::log("Chunked ISO: Preparing boot...");
        boot_seq.render(screen, 5, 8);

        // Get ISO manifest and determine partition number from first chunk's LBA
        let (iso_name, partition_num) = storage
            .get(iso_idx)
            .map(|e| {
                let name = e.manifest.name_str();
                // Get partition number from first chunk's start LBA
                // ESP partition 1 ends at ~8388608 LBA (4GB)
                // Data partitions start after that, numbered sequentially
                // Each ~4GB partition is ~8388608 sectors
                let first_chunk_lba = if e.manifest.chunks.count > 0 {
                    e.manifest.chunks.chunks[0].start_lba
                } else {
                    0
                };

                // Calculate partition number based on LBA ranges
                // Partition 1 (ESP): ~0-8388608
                // Partition 2+: sequential 4GB chunks
                let part_num = if first_chunk_lba < 8_388_608 {
                    1 // ESP (shouldn't happen for ISO data)
                } else {
                    // Data partitions: (LBA - ESP_size) / partition_size + 2
                    // Simplified: estimate based on LBA position
                    ((first_chunk_lba - 8_388_608) / 8_388_608) + 2
                };

                (name, part_num)
            })
            .unwrap_or(("", 2));

        // Build device path dynamically based on actual partition
        let device_path = alloc::format!("/dev/vda{}", partition_num);

        morpheus_core::logger::log(
            alloc::format!("ISO on partition {}: {}", partition_num, device_path).leak(),
        );

        // Determine cmdline based on ISO name/type - use actual partition
        let cmdline: String = if iso_name.to_lowercase().contains("tails") {
            // Tails-specific: needs boot=live with live-media to find the ISO
            alloc::format!("boot=live live-media={} nopersistence noprompt timezone=Etc/UTC splash noautologin module=Tails console=ttyS0,115200 earlyprintk=serial,ttyS0,115200", device_path)
        } else if iso_name.to_lowercase().contains("kali") {
            // Kali Linux - uses live-boot with live-media parameter
            alloc::format!("boot=live live-media={} components console=ttyS0,115200 earlyprintk=serial,ttyS0,115200", device_path)
        } else if iso_name.to_lowercase().contains("ubuntu") {
            alloc::format!("boot=casper quiet splash console=ttyS0,115200")
        } else if iso_name.to_lowercase().contains("debian") {
            // Debian Live uses live-boot
            alloc::format!("boot=live live-media={} components console=ttyS0,115200 earlyprintk=serial,ttyS0,115200", device_path)
        } else if iso_name.to_lowercase().contains("parrot") {
            // Parrot OS - Debian-based, uses live-boot
            alloc::format!("boot=live live-media={} components console=ttyS0,115200 earlyprintk=serial,ttyS0,115200", device_path)
        } else if iso_name.to_lowercase().contains("blackarch") {
            // BlackArch - Arch-based live system
            alloc::format!(
                "archisodevice={} console=ttyS0,115200 earlyprintk=serial,ttyS0,115200",
                device_path
            )
        } else if iso_name.to_lowercase().contains("fedora") {
            alloc::format!("rd.live.image quiet console=ttyS0,115200")
        } else if iso_name.to_lowercase().contains("puppy")
            || iso_name.to_lowercase().contains("fossapup")
        {
            // Puppy Linux - use sda instead of vda for compatibility
            let sda_device = alloc::format!("sda{}", partition_num);
            alloc::format!("pmedia=usbhd pdev1={} psubdir=/ console=ttyS0,115200 earlyprintk=serial,ttyS0,115200", sda_device)
        } else {
            // Generic live boot cmdline
            alloc::format!("boot=live live-media={} components console=ttyS0,115200 earlyprintk=serial,ttyS0,115200 debug loglevel=7", device_path)
        };

        progress_bar.set_progress(100);
        progress_bar.render(screen);

        screen.put_str_at(5, 16, "Starting kernel...", EFI_LIGHTGREEN, EFI_BLACK);

        // Convert UEFI page addresses to slices for boot
        let kernel_slice =
            unsafe { core::slice::from_raw_parts(kernel_addr as *const u8, kernel_size) };

        let initrd_slice = initrd_data.map(|(addr, size, _pages)| unsafe {
            core::slice::from_raw_parts(addr as *const u8, size)
        });

        // DEBUG: Show boot parameters before jumping
        screen.clear();
        screen.put_str_at(5, 2, "=== BOOT DEBUG ===", EFI_YELLOW, EFI_BLACK);
        screen.put_str_at(
            5,
            4,
            &format!("Kernel addr: 0x{:x}", kernel_addr),
            EFI_CYAN,
            EFI_BLACK,
        );
        screen.put_str_at(
            5,
            5,
            &format!("Kernel size: {} bytes", kernel_size),
            EFI_CYAN,
            EFI_BLACK,
        );
        if let Some((iaddr, isize, _)) = initrd_data {
            screen.put_str_at(
                5,
                6,
                &format!("Initrd addr: 0x{:x}", iaddr),
                EFI_CYAN,
                EFI_BLACK,
            );
            screen.put_str_at(
                5,
                7,
                &format!("Initrd size: {} bytes", isize),
                EFI_CYAN,
                EFI_BLACK,
            );
        }
        screen.put_str_at(
            5,
            9,
            &format!("Cmdline: {}", &cmdline[..cmdline.len().min(60)]),
            EFI_CYAN,
            EFI_BLACK,
        );
        screen.put_str_at(
            5,
            10,
            &format!("Device: {}", device_path),
            EFI_CYAN,
            EFI_BLACK,
        );
        // Check kernel magic
        let magic = u16::from_le_bytes([kernel_slice[510], kernel_slice[511]]);
        screen.put_str_at(
            5,
            11,
            &format!("Boot sector magic: 0x{:04x}", magic),
            EFI_CYAN,
            EFI_BLACK,
        );
        if magic == 0xAA55 {
            screen.put_str_at(5, 12, "Magic: VALID", EFI_LIGHTGREEN, EFI_BLACK);
        } else {
            screen.put_str_at(
                5,
                12,
                "Magic: INVALID (expected 0xAA55)",
                EFI_RED,
                EFI_BLACK,
            );
        }
        // Check for bzImage header at offset 0x202
        let hdr_magic = u32::from_le_bytes([
            kernel_slice[0x202],
            kernel_slice[0x203],
            kernel_slice[0x204],
            kernel_slice[0x205],
        ]);
        screen.put_str_at(
            5,
            13,
            &format!("Header magic: 0x{:08x}", hdr_magic),
            EFI_CYAN,
            EFI_BLACK,
        );
        if hdr_magic == 0x53726448 {
            // "HdrS"
            screen.put_str_at(5, 14, "bzImage header: VALID", EFI_LIGHTGREEN, EFI_BLACK);
        } else {
            screen.put_str_at(5, 14, "bzImage header: NOT FOUND", EFI_RED, EFI_BLACK);
        }
        screen.put_str_at(5, 16, "Press any key to boot...", EFI_YELLOW, EFI_BLACK);
        keyboard.wait_for_key();

        // SAFETY: Calling kernel boot with properly loaded kernel/initrd data
        let boot_result = unsafe {
            crate::boot::loader::boot_linux_kernel(
                boot_services,
                system_table,
                image_handle,
                kernel_slice,
                initrd_slice,
                &cmdline,
                screen,
            )
        };

        if let Err(error) = boot_result {
            let detail = Self::describe_boot_error(&error);
            let msg = format!("ERROR: {}", detail);
            Self::await_failure(screen, keyboard, 18, &msg, "chunked iso boot failed");
        }
    }

    /// Get BlockIoProtocol pointer for first physical disk
    fn get_first_disk_block_io(
        boot_services: &crate::BootServices,
    ) -> Option<*mut crate::uefi::block_io::BlockIoProtocol> {
        use crate::uefi::block_io::{BlockIoProtocol, EFI_BLOCK_IO_PROTOCOL_GUID};

        // Get buffer size needed for all Block I/O handles
        let mut buffer_size: usize = 0;
        let _ = (boot_services.locate_handle)(
            2, // ByProtocol
            &EFI_BLOCK_IO_PROTOCOL_GUID,
            core::ptr::null(),
            &mut buffer_size,
            core::ptr::null_mut(),
        );

        if buffer_size == 0 {
            return None;
        }

        // Allocate buffer for handles
        let mut handle_buffer: *mut u8 = core::ptr::null_mut();
        let alloc_status = (boot_services.allocate_pool)(2, buffer_size, &mut handle_buffer);

        if alloc_status != 0 || handle_buffer.is_null() {
            return None;
        }

        // Get all Block I/O handles
        let status = (boot_services.locate_handle)(
            2,
            &EFI_BLOCK_IO_PROTOCOL_GUID,
            core::ptr::null(),
            &mut buffer_size,
            handle_buffer as *mut *mut (),
        );

        if status != 0 {
            (boot_services.free_pool)(handle_buffer);
            return None;
        }

        // Iterate through handles and find physical disks
        let handles = handle_buffer as *const *mut ();
        let handle_count = buffer_size / core::mem::size_of::<*mut ()>();

        // Find first physical disk (not a partition)
        let mut result = None;
        unsafe {
            for i in 0..handle_count {
                let handle = *handles.add(i);
                let mut block_io_ptr: *mut () = core::ptr::null_mut();

                let proto_status = (boot_services.handle_protocol)(
                    handle,
                    &EFI_BLOCK_IO_PROTOCOL_GUID,
                    &mut block_io_ptr,
                );

                if proto_status == 0 && !block_io_ptr.is_null() {
                    let block_io = &*(block_io_ptr as *const BlockIoProtocol);
                    let media = &*block_io.media;

                    // Only use physical disks, not partitions
                    if !media.logical_partition && media.media_present {
                        result = Some(block_io_ptr as *mut BlockIoProtocol);
                        break;
                    }
                }
            }
        }

        (boot_services.free_pool)(handle_buffer);
        result
    }

    /// Get BlockIoProtocol for a disk that can contain the given LBA
    /// This finds a physical disk (not partition) where last_block >= required_lba
    fn get_disk_for_lba(
        boot_services: &crate::BootServices,
        required_lba: u64,
    ) -> Option<*mut crate::uefi::block_io::BlockIoProtocol> {
        use crate::uefi::block_io::{BlockIoProtocol, EFI_BLOCK_IO_PROTOCOL_GUID};

        // Get buffer size needed for all Block I/O handles
        let mut buffer_size: usize = 0;
        let _ = (boot_services.locate_handle)(
            2, // ByProtocol
            &EFI_BLOCK_IO_PROTOCOL_GUID,
            core::ptr::null(),
            &mut buffer_size,
            core::ptr::null_mut(),
        );

        if buffer_size == 0 {
            return None;
        }

        // Allocate buffer for handles
        let mut handle_buffer: *mut u8 = core::ptr::null_mut();
        let alloc_status = (boot_services.allocate_pool)(2, buffer_size, &mut handle_buffer);

        if alloc_status != 0 || handle_buffer.is_null() {
            return None;
        }

        // Get all Block I/O handles
        let status = (boot_services.locate_handle)(
            2,
            &EFI_BLOCK_IO_PROTOCOL_GUID,
            core::ptr::null(),
            &mut buffer_size,
            handle_buffer as *mut *mut (),
        );

        if status != 0 {
            (boot_services.free_pool)(handle_buffer);
            return None;
        }

        // Iterate through handles and find disk with required capacity
        let handles = handle_buffer as *const *mut ();
        let handle_count = buffer_size / core::mem::size_of::<*mut ()>();

        let mut result = None;
        unsafe {
            for i in 0..handle_count {
                let handle = *handles.add(i);
                let mut block_io_ptr: *mut () = core::ptr::null_mut();

                let proto_status = (boot_services.handle_protocol)(
                    handle,
                    &EFI_BLOCK_IO_PROTOCOL_GUID,
                    &mut block_io_ptr,
                );

                if proto_status == 0 && !block_io_ptr.is_null() {
                    let block_io = &*(block_io_ptr as *const BlockIoProtocol);
                    let media = &*block_io.media;

                    // Only use physical disks, not partitions
                    // AND check if disk is large enough for required_lba
                    if !media.logical_partition && media.media_present {
                        morpheus_core::logger::log(
                            alloc::format!(
                                "Disk {}: last_block={}, required={}",
                                i,
                                media.last_block,
                                required_lba
                            )
                            .leak(),
                        );

                        if media.last_block >= required_lba {
                            morpheus_core::logger::log(
                                alloc::format!("Selected disk {} for ISO boot", i).leak(),
                            );
                            result = Some(block_io_ptr as *mut BlockIoProtocol);
                            break;
                        }
                    }
                }
            }
        }

        (boot_services.free_pool)(handle_buffer);
        result
    }

    unsafe fn get_esp_root(
        boot_services: &crate::BootServices,
        image_handle: *mut (),
    ) -> Result<*mut FileProtocol, ()> {
        use crate::uefi::file_system::{
            get_file_system_protocol, get_loaded_image, open_root_volume,
        };

        let loaded_image = get_loaded_image(boot_services, image_handle)?;
        let device_handle = (*loaded_image).device_handle;

        let fs_protocol = get_file_system_protocol(boot_services, device_handle)?;
        open_root_volume(fs_protocol)
    }
    fn read_file_to_uefi_pages(
        boot_services: &crate::BootServices,
        image_handle: *mut (),
        path: &str,
        screen: &mut Screen,
        max_size: usize,
        start_line: usize, // where to render status messages
    ) -> Result<(*mut u8, usize), &'static str> {
        use crate::uefi::file_system::*;

        unsafe {
            let loaded_image = get_loaded_image(boot_services, image_handle).map_err(|_| {
                morpheus_core::logger::log("FAIL: get_loaded_image");
                "Failed to get loaded image"
            })?;

            let device_handle = (*loaded_image).device_handle;
            let fs_protocol =
                get_file_system_protocol(boot_services, device_handle).map_err(|_| {
                    morpheus_core::logger::log("FAIL: get_file_system_protocol");
                    "Failed to get file system protocol"
                })?;
            let root = open_root_volume(fs_protocol).map_err(|_| {
                morpheus_core::logger::log("FAIL: open_root_volume");
                "Failed to open root volume"
            })?;

            screen.put_str_at(5, start_line, "Opening file...", EFI_DARKGREEN, EFI_BLACK);

            let mut utf16_path = [0u16; 256];
            ascii_to_utf16(path, &mut utf16_path);

            let file = open_file_read(root, &utf16_path).map_err(|status| {
                morpheus_core::logger::log("FAIL: open_file_read");
                if status == 0x80000000000000 | 14 {
                    "File not found"
                } else {
                    "Failed to open file"
                }
            })?;

            screen.put_str_at(
                5,
                start_line + 1,
                "Allocating pages...",
                EFI_DARKGREEN,
                EFI_BLACK,
            );

            // Here we try to allocate UEFI pages for the file read buffer.
            // Start with max_size, then try 256MB, 128MB, 64MB chunks
            // until one works. If none work, return error. This is not "beatifull" but it works so get of my ass.            const PAGE_SIZE: usize = 4096;
            let chunk_sizes = [
                max_size,
                256 * 1024 * 1024, // 256MB
                128 * 1024 * 1024, // 128MB
                64 * 1024 * 1024,  // 64MB
            ];

            let mut buffer_addr = 0u64;
            let mut chunk_size = 0usize;
            let alloc_type = 0; // EFI_ALLOCATE_ANY_PAGES - less fragmentation
            let mem_type = 2; // EFI_LOADER_DATA

            // Try progressively smaller allocations
            for &size in &chunk_sizes {
                let pages_needed = (size + PAGE_SIZE - 1) / PAGE_SIZE;
                let mut addr = 0u64; // ANY_PAGES doesn't need initial address

                let status =
                    (boot_services.allocate_pages)(alloc_type, mem_type, pages_needed, &mut addr);

                if status == 0 {
                    buffer_addr = addr;
                    chunk_size = size;
                    break;
                }
            }

            if buffer_addr == 0 {
                morpheus_core::logger::log("FAIL: all allocate_pages attempts");
                close_file(file).ok();
                close_file(root).ok();
                return Err("Failed to allocate UEFI pages");
            }

            let buffer_ptr = buffer_addr as *mut u8;
            let mut bytes_to_read = chunk_size;

            screen.put_str_at(
                5,
                start_line + 2,
                "Reading file...",
                EFI_DARKGREEN,
                EFI_BLACK,
            );

            // Read up to chunk_size - UEFI will update bytes_to_read with actual amount
            let status = ((*file).read)(file, &mut bytes_to_read, buffer_ptr);

            if status != 0 {
                morpheus_core::logger::log("FAIL: file.read");
                let pages = (chunk_size + PAGE_SIZE - 1) / PAGE_SIZE;
                (boot_services.free_pages)(buffer_addr, pages);
                close_file(file).ok();
                close_file(root).ok();
                return Err("Failed to read file");
            }

            close_file(file).ok();
            close_file(root).ok();

            screen.put_str_at(5, start_line + 3, "Success!", EFI_GREEN, EFI_BLACK);

            // Return pointer + actual bytes read
            Ok((buffer_ptr, bytes_to_read))
        }
    }
}
