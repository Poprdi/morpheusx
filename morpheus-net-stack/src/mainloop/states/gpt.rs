//! Claim disk space (GPT partition) before download; relocates on overlap.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::time::Instant;

use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};
use morpheus_block::device::UnifiedBlockDevice;
use morpheus_block::transfer::disk::{DiskError, GptOps};
use morpheus_block::unified_block_io::UnifiedBlockIo;
use morpheus_nic::traits::NetworkDriver;

use super::{FailedState, LinkWaitState};

const GPT_DMA_BUFFER_SIZE: usize = 64 * 1024;
static mut GPT_DMA_BUFFER: [u8; GPT_DMA_BUFFER_SIZE] = [0u8; GPT_DMA_BUFFER_SIZE];

pub(crate) struct GptPrepState {
    started: bool,
    completed: bool,
}

impl GptPrepState {
    pub fn new() -> Self {
        Self {
            started: false,
            completed: false,
        }
    }

    /// Returns the actual range to use; relocates if requested overlaps.
    fn verify_or_find_space(
        &self,
        blk: &mut UnifiedBlockDevice,
        requested_start: u64,
        requested_end: u64,
    ) -> Result<(u64, u64), &'static str> {
        let (dma_buffer, dma_buffer_phys) = unsafe {
            let buf = core::slice::from_raw_parts_mut(
                (&raw mut GPT_DMA_BUFFER).cast::<u8>(),
                GPT_DMA_BUFFER_SIZE,
            );
            let phys = (&raw const GPT_DMA_BUFFER).cast::<u8>() as u64;
            (buf, phys)
        };
        let timeout_ticks = 100_000_000u64;

        let mut adapter = match UnifiedBlockIo::new(blk, dma_buffer, dma_buffer_phys, timeout_ticks)
        {
            Ok(a) => a,
            Err(_) => return Err("failed to create BlockIo adapter"),
        };

        match GptOps::verify_range_free(&mut adapter, requested_start, requested_end) {
            Ok(true) => {
                serial::println("[GPT] ✓ Range verified free");
                return Ok((requested_start, requested_end));
            },
            Ok(false) => {
                serial::println("[GPT] WARNING: Requested range overlaps existing partition");
                serial::print("[GPT] Requested: ");
                serial::print_hex(requested_start);
                serial::print(" - ");
                serial::print_hex(requested_end);
                serial::println("");
            },
            Err(e) => {
                serial::print("[GPT] ERROR: Could not verify range: ");
                serial::println(match e {
                    DiskError::IoError => "IO error",
                    DiskError::InvalidGpt => "Invalid GPT",
                    _ => "Unknown error",
                });
                return Err("range verification failed");
            },
        }

        serial::println("[GPT] Searching for alternative free space...");
        match GptOps::find_free_space(&mut adapter) {
            Ok((free_start, free_end)) => {
                let free_size = free_end - free_start + 1;
                let needed_size = requested_end - requested_start + 1;

                if free_size >= needed_size {
                    serial::print("[GPT] ✓ Found suitable free space: ");
                    serial::print_hex(free_start);
                    serial::print(" - ");
                    serial::print_hex(free_end);
                    serial::print(" (");
                    serial::print_u32((free_size * 512 / (1024 * 1024 * 1024)) as u32);
                    serial::println(" GB)");

                    // Align to 1 MB (2048 sectors).
                    let aligned_start = free_start.div_ceil(2048) * 2048;
                    let aligned_end = aligned_start + needed_size - 1;

                    serial::print("[GPT] Using aligned range: ");
                    serial::print_hex(aligned_start);
                    serial::print(" - ");
                    serial::print_hex(aligned_end);
                    serial::println("");

                    Ok((aligned_start, aligned_end))
                } else {
                    serial::print("[GPT] ERROR: Free space too small (");
                    serial::print_u32((free_size * 512 / (1024 * 1024 * 1024)) as u32);
                    serial::print(" GB < ");
                    serial::print_u32((needed_size * 512 / (1024 * 1024 * 1024)) as u32);
                    serial::println(" GB needed)");
                    Err("insufficient free space")
                }
            },
            Err(e) => {
                serial::print("[GPT] ERROR: Could not find free space: ");
                serial::println(match e {
                    DiskError::IoError => "IO error",
                    DiskError::InvalidGpt => "Invalid GPT",
                    DiskError::NoFreeSpace => "No free space",
                    _ => "Unknown error",
                });
                Err("no free space found")
            },
        }
    }

    fn create_partition(
        &self,
        blk: &mut UnifiedBlockDevice,
        start_sector: u64,
        end_sector: u64,
    ) -> Result<[u8; 16], &'static str> {
        use morpheus_storage_format::disk::gpt_ops::create_partition;
        use morpheus_storage_format::disk::partition::PartitionType;

        let (dma_buffer, dma_buffer_phys) = unsafe {
            let buf = core::slice::from_raw_parts_mut(
                (&raw mut GPT_DMA_BUFFER).cast::<u8>(),
                GPT_DMA_BUFFER_SIZE,
            );
            let phys = (&raw const GPT_DMA_BUFFER).cast::<u8>() as u64;
            (buf, phys)
        };
        let timeout_ticks = 100_000_000u64;

        let adapter = match UnifiedBlockIo::new(blk, dma_buffer, dma_buffer_phys, timeout_ticks) {
            Ok(a) => a,
            Err(_) => return Err("failed to create BlockIo adapter"),
        };

        serial::println("[GPT] Writing partition entry to GPT...");

        match create_partition(adapter, PartitionType::BasicData, start_sector, end_sector) {
            Ok(()) => {
                serial::println("[GPT] ───────────────────────────────────────");
                serial::println("[GPT] PARTITION CREATED SUCCESSFULLY");
                serial::println("[GPT] ───────────────────────────────────────");
                serial::print("[GPT] Location: sectors ");
                serial::print_hex(start_sector);
                serial::print(" - ");
                serial::print_hex(end_sector);
                serial::println("");
                serial::println("[GPT] Type: Microsoft Basic Data");
                serial::println("[GPT] Status: Active in GPT partition table");

                Ok([
                    0x12, 0x34, 0x56, 0x78, 0x12, 0x34, 0x56, 0x78, 0x12, 0x34, 0x56, 0x78, 0x12,
                    0x34, 0x56, 0x78,
                ])
            },
            Err(e) => {
                serial::print("[GPT] ERROR: Failed to create partition: ");
                serial::println(match e {
                    morpheus_storage_format::disk::gpt_ops::GptError::IoError => "IO error",
                    morpheus_storage_format::disk::gpt_ops::GptError::InvalidHeader => {
                        "Invalid GPT header"
                    },
                    morpheus_storage_format::disk::gpt_ops::GptError::InvalidSize => {
                        "Invalid size/range"
                    },
                    morpheus_storage_format::disk::gpt_ops::GptError::NoSpace => {
                        "No free partition slot"
                    },
                    morpheus_storage_format::disk::gpt_ops::GptError::PartitionNotFound => {
                        "Partition not found"
                    },
                    morpheus_storage_format::disk::gpt_ops::GptError::OverlappingPartitions => {
                        "Range overlaps"
                    },
                    morpheus_storage_format::disk::gpt_ops::GptError::AlignmentError => {
                        "Alignment error"
                    },
                });
                Err("partition creation failed")
            },
        }
    }
}

impl Default for GptPrepState {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: NetworkDriver> State<D> for GptPrepState {
    fn step(
        mut self: Box<Self>,
        ctx: &mut Context<'_>,
        _iface: &mut Interface,
        _sockets: &mut SocketSet<'_>,
        _adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        _tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if self.completed {
            serial::println("[GPT] -> LinkWait");
            return (Box::new(LinkWaitState::new()), StepResult::Transition);
        }

        if !self.started {
            self.started = true;

            if !ctx.config.write_to_disk {
                serial::println("[GPT] Disk writes disabled, skipping partition setup");
                self.completed = true;
                return (self, StepResult::Continue);
            }

            let blk = match &mut ctx.blk_device {
                Some(b) => b,
                None => {
                    serial::println("[GPT] No block device available, skipping partition setup");
                    self.completed = true;
                    return (self, StepResult::Continue);
                },
            };

            serial::println("=================================");
            serial::println("   GPT PARTITION PREPARATION     ");
            serial::println("=================================");
            serial::print("[GPT] ISO name: ");
            serial::println(ctx.config.iso_name);
            serial::print("[GPT] Requested start sector: ");
            serial::print_hex(ctx.config.target_start_sector);
            serial::println("");

            let size_bytes = if ctx.config.expected_size > 0 {
                ctx.config.expected_size
            } else {
                8 * 1024 * 1024 * 1024 // default 8 GB
            };
            let sectors_needed = size_bytes.div_ceil(512);
            let requested_end = ctx.config.target_start_sector + sectors_needed - 1;

            serial::print("[GPT] Requested end sector: ");
            serial::print_hex(requested_end);
            serial::println("");
            serial::print("[GPT] Size: ");
            serial::print_u32((size_bytes / (1024 * 1024 * 1024)) as u32);
            serial::println(" GB");

            let (actual_start, actual_end) =
                match self.verify_or_find_space(blk, ctx.config.target_start_sector, requested_end)
                {
                    Ok((s, e)) => (s, e),
                    Err(msg) => {
                        serial::print("[GPT] ");
                        serial::println(msg);
                        serial::println("[GPT] ABORTING: Cannot safely create partition");
                        return (
                            Box::new(FailedState::new("gpt prep failed")),
                            StepResult::Failed("gpt"),
                        );
                    },
                };

            ctx.actual_start_sector = actual_start;

            // Re-borrow after dropping verify_or_find_space's loan.
            let blk = ctx.blk_device.as_mut().unwrap();
            match self.create_partition(blk, actual_start, actual_end) {
                Ok(uuid) => {
                    serial::println("[GPT] ISO partition created and claimed");
                    let _ = uuid;
                },
                Err(msg) => {
                    serial::print("[GPT] WARNING: ");
                    serial::println(msg);
                    serial::println("[GPT] Continuing anyway (data may be in unmapped space)");
                },
            }

            self.completed = true;
        }

        (self, StepResult::Continue)
    }

    fn name(&self) -> &'static str {
        "GptPrep"
    }
}
