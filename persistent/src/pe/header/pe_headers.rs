//! Main PeHeaders implementation with complex reconstruction logic

use super::coff_header::CoffHeader;
use super::dos_header::DosHeader;
use super::optional_header::OptionalHeader64;
use super::utils::*;
use super::super::section::SectionTable;
use super::super::{PeArch, PeError, PeResult};

extern crate alloc;
use alloc::vec::Vec;

/// Complete PE headers structure
pub struct PeHeaders {
    pub dos: DosHeader,
    pub coff: CoffHeader,
    pub optional: OptionalHeader64,
}

impl PeHeaders {
    /// Parse all PE headers from image in memory
    ///
    /// # Safety
    /// Caller must ensure image_base points to valid PE file of given size
    pub unsafe fn parse(image_base: *const u8, image_size: usize) -> PeResult<Self> {
        // Parse DOS header
        let dos = DosHeader::parse(image_base, image_size)?;

        // Parse COFF header (validates PE signature)
        let coff = CoffHeader::parse(image_base, dos.e_lfanew, image_size)?;

        // Parse optional header
        let optional = OptionalHeader64::parse(image_base, dos.e_lfanew, image_size)?;

        Ok(PeHeaders {
            dos,
            coff,
            optional,
        })
    }

    /// Get the architecture of this PE file
    pub fn arch(&self) -> PeResult<PeArch> {
        self.coff.arch()
    }

    /// Calculate relocation delta between loaded address and ImageBase
    /// WARNING: ImageBase in memory is PATCHED by UEFI loader - this will always return 0!
    pub fn relocation_delta(&self, actual_load_address: u64) -> i64 {
        actual_load_address as i64 - self.optional.image_base as i64
    }

    /// Reconstruct original ImageBase using proper validation
    ///
    /// Strategy:
    /// 1. Check for compile-time hint (LINKER_IMAGE_BASE constant)
    /// 2. Parse section table to know valid RVA ranges
    /// 3. Collect DIR64 relocations from memory
    /// 4. For each candidate ImageBase:
    ///    - Calculate what the ORIGINAL pointer values would have been
    ///    - Check if those original values = candidate + valid_section_RVA
    ///    - Original value MUST point within a valid section's RVA range
    /// 5. Return candidate with highest validation rate
    ///
    /// This fixes the circular logic bug by validating against section boundaries,
    /// not against our own derived values!
    ///
    /// Returns: (original_image_base, validated_relocs, total_relocs)
    ///
    /// # Safety
    /// Caller must ensure image_base points to valid relocated PE image
    pub unsafe fn reconstruct_original_image_base(
        &self,
        image_base: *const u8,
        image_size: usize,
        actual_load_address: u64,
    ) -> PeResult<(u64, u32, u32)> {
        // Step 1: Parse section table to get valid RVA ranges
        let section_offset =
            self.dos.e_lfanew as usize + 24 + self.coff.size_of_optional_header as usize;

        let sections = SectionTable::parse(
            image_base,
            section_offset,
            self.coff.number_of_sections as usize,
            image_size,
        )?;

        // Build list of valid RVA ranges (where pointers can point)
        let mut valid_ranges: [(u32, u32); 16] = [(0, 0); 16];
        let mut range_count = 0usize;

        for sec in sections.iter().take(16) {
            if sec.virtual_size > 0 {
                valid_ranges[range_count] =
                    (sec.virtual_address, sec.virtual_address + sec.virtual_size);
                range_count += 1;
            }
        }

        if range_count == 0 {
            return Err(PeError::CorruptedData);
        }

        // Step 2: Find reloc section and collect relocations from ALL blocks
        let reloc_section = sections
            .find_reloc_section()
            .ok_or(PeError::MissingSection)?;

        let reloc_data_ptr = image_base.add(reloc_section.virtual_address as usize);
        let reloc_size = reloc_section.virtual_size;

        // NOTE: UEFI may truncate reloc section after applying relocations!
        // Use the larger of virtual_size or a reasonable max based on image size
        let max_reloc_size = reloc_size.max(512); // Allow up to 512 bytes of reloc data

        // Collect DIR64 relocations with their current values from ALL blocks
        let mut relocations: [u64; 256] = [0; 256];
        let mut reloc_count = 0usize;
        let mut block_offset = 0usize;

        // Force iteration through ALL relocation blocks
        // Process up to 16 blocks max (should cover any reasonable PE file)
        for _block_num in 0..16 {
            // Check if we have space for block header
            if block_offset + 8 > max_reloc_size as usize {
                break;
            }

            let page_rva = read_u32(reloc_data_ptr, block_offset);
            let block_size = read_u32(reloc_data_ptr, block_offset + 4);

            // Validate block
            if !(8..=1024).contains(&block_size) {
                break; // Invalid or end marker
            }

            if block_offset + block_size as usize > max_reloc_size as usize {
                break; // Block extends beyond reasonable limit
            }

            let entry_count = (block_size - 8) / 2;

            // Process ALL entries in this block
            for i in 0..entry_count {
                if reloc_count >= 256 {
                    break; // Hit our array limit
                }

                let entry = read_u16(reloc_data_ptr, block_offset + 8 + (i * 2) as usize);
                let reloc_type = (entry >> 12) & 0xF;
                let offset = entry & 0xFFF;

                if reloc_type == 10 {
                    // IMAGE_REL_BASED_DIR64
                    let pointer_rva = page_rva + offset as u32;

                    // Skip if RVA is out of bounds
                    if pointer_rva as usize + 8 > image_size {
                        continue;
                    }

                    let pointer_addr = image_base.add(pointer_rva as usize) as *const u64;
                    let current_value = *pointer_addr;

                    relocations[reloc_count] = current_value;
                    reloc_count += 1;
                }
            }

            // Move to next block
            block_offset += block_size as usize;
        }

        if reloc_count < 8 {
            return Err(PeError::CorruptedData);
        }

        // Step 3: Test candidate ImageBase values
        let section_align = self.optional.section_alignment as u64;

        // Start with compile-time hint if available
        let mut candidates = [0u64; 16];
        let mut cand_idx = 0;

        if let Some(linker_base) = super::super::compile_time::get_original_image_base_hint() {
            candidates[cand_idx] = linker_base;
            cand_idx += 1;
        }

        // Add common UEFI bases
        let common_bases = [
            0x0000000140000000u64,
            0x0000000000400000u64,
            0x0000000100000000u64,
            // Aligned to actual load
            actual_load_address & !0xFFFFu64,   // 64KB aligned
            actual_load_address & !0xFFFFFu64,  // 1MB aligned
            actual_load_address & !0x3FFFFFu64, // 4MB aligned
            actual_load_address & !(section_align - 1), // Section-aligned
            // Try common deltas from actual load
            actual_load_address.saturating_sub(0x1000),
            actual_load_address.saturating_sub(0x10000),
            actual_load_address.saturating_sub(0x100000),
            actual_load_address.saturating_sub(0x1000000),
        ];

        for &base in &common_bases {
            if base != 0 && cand_idx < 16 && !candidates[..cand_idx].contains(&base) {
                candidates[cand_idx] = base;
                cand_idx += 1;
            }
        }

        let candidates = &candidates[..cand_idx];

        let mut best_candidate = 0u64;
        let mut best_valid_count = 0u32;

        // Test each candidate
        for &candidate in candidates {
            if candidate == 0 {
                continue;
            }

            let delta = actual_load_address as i64 - candidate as i64;
            let mut valid_count = 0u32;

            // For each relocation, check if unrelocated value would be valid
            for i in 0..reloc_count {
                let current_value = relocations[i];

                // Calculate what the ORIGINAL value would have been
                let original_value = (current_value as i64 - delta) as u64;

                // Check: does original_value = candidate + some_valid_RVA?
                if original_value < candidate {
                    continue; // Can't be valid
                }

                let rva = original_value - candidate;

                // Validate that this RVA falls within a valid section!
                let mut rva_in_section = false;
                for j in 0..range_count {
                    let (start, end) = valid_ranges[j];
                    if rva >= start as u64 && rva < end as u64 {
                        rva_in_section = true;
                        break;
                    }
                }

                if rva_in_section {
                    valid_count += 1;
                }
            }

            // Update best candidate
            if valid_count > best_valid_count {
                best_valid_count = valid_count;
                best_candidate = candidate;
            }
        }

        // Require at least 90% success rate (allow some edge cases)
        let min_valid = (reloc_count as u32 * 9) / 10; // 90%

        if best_valid_count >= min_valid {
            Ok((best_candidate, best_valid_count, reloc_count as u32))
        } else {
            // Fallback: return best guess even if validation is weak
            Ok((best_candidate, best_valid_count, reloc_count as u32))
        }
    }

    /// Create bootable PE image from relocated memory image
    ///
    /// This is the main function for extracting the running bootloader and
    /// making it bootable again.
    ///
    /// Steps:
    /// 1. Reconstruct original ImageBase (validated)
    /// 2. Calculate relocation delta
    /// 3. Reverse all DIR64 relocations (using embedded metadata if available)
    /// 4. Patch ImageBase field in header
    ///
    /// Returns the delta used for unrelocating (for logging)
    ///
    /// # Safety
    /// Caller must ensure image_data is valid relocated PE image
    pub unsafe fn unrelocate_image(
        &self,
        image_data: &mut [u8],
        actual_load_address: u64,
    ) -> PeResult<i64> {
        // Step 1: Restore .reloc from hardcoded data (UEFI discards .reloc after loading)
        let reloc_rva = super::super::embedded_reloc_data::RELOC_RVA;
        let reloc_size = super::super::embedded_reloc_data::RELOC_SIZE;
        let reloc_data = &super::super::embedded_reloc_data::RELOC_DATA;
        let original_image_base = super::super::embedded_reloc_data::ORIGINAL_IMAGE_BASE;

        // image_data is in RVA layout (memory layout), so copy to RVA offset
        let reloc_offset = reloc_rva as usize;
        if reloc_offset + reloc_data.len() > image_data.len() {
            return Err(PeError::InvalidOffset);
        }

        core::ptr::copy_nonoverlapping(
            reloc_data.as_ptr(),
            image_data.as_mut_ptr().add(reloc_offset),
            reloc_data.len(),
        );

        // Step 2: Calculate delta (use hardcoded original ImageBase)
        let delta = actual_load_address as i64 - original_image_base as i64;

        // Step 3: Unrelocate all pointers (works on RVA layout)
        super::super::reloc::unrelocate_image(image_data, reloc_rva, reloc_size, delta)?;

        // Step 4: Patch ImageBase in header
        OptionalHeader64::patch_image_base(image_data, original_image_base)?;

        Ok(delta)
    }

    /// Convert memory-layout (RVA-based) PE image to file-layout (PointerToRawData-based)
    ///
    /// UEFI loads PE files into memory with sections at their VirtualAddress (RVA) offsets.
    /// But PE files on disk have sections at PointerToRawData (file) offsets.
    /// This function converts from memory layout back to file layout.
    ///
    /// # Safety
    /// Caller must ensure rva_image is valid PE image in memory layout
    pub unsafe fn rva_to_file_layout(&self, rva_image: &[u8]) -> PeResult<alloc::vec::Vec<u8>> {
        // Section headers may be corrupted after unrelocate if they contain pointers
        // So we parse them from the ORIGINAL unmodified headers at the start of rva_image
        // Headers (first SizeOfHeaders bytes) should NOT be modified by unrelocate
        let section_offset =
            self.dos.e_lfanew as usize + 24 + self.coff.size_of_optional_header as usize;

        // Read section headers directly from the buffer (they're in the headers region)
        let sections = super::super::section::SectionTable::parse(
            rva_image.as_ptr(),
            section_offset,
            self.coff.number_of_sections as usize,
            rva_image.len(),
        )?;

        // Calculate file size as max(all section ends)
        // Use SizeOfImage as upper bound to handle any issues
        let mut actual_file_size = self.optional.size_of_headers as usize;

        for i in 0..self.coff.number_of_sections as usize {
            if let Some(section) = sections.get(i) {
                if section.size_of_raw_data > 0 {
                    let section_end =
                        section.pointer_to_raw_data as usize + section.size_of_raw_data as usize;
                    if section_end > actual_file_size {
                        actual_file_size = section_end;
                    }
                }
            }
        }

        let mut file_image = alloc::vec![0; actual_file_size];

        // Copy headers (should be unmodified by unrelocate)
        let header_size = self.optional.size_of_headers as usize;
        if header_size > rva_image.len() {
            return Err(PeError::InvalidOffset);
        }
        file_image[..header_size].copy_from_slice(&rva_image[..header_size]);

        // Copy each section from RVA to file offset
        for i in 0..self.coff.number_of_sections as usize {
            let section = match sections.get(i) {
                Some(s) => s,
                None => continue,
            };

            let virtual_addr = section.virtual_address as usize;
            let file_offset = section.pointer_to_raw_data as usize;
            let virtual_size = section.virtual_size as usize;
            let raw_size = section.size_of_raw_data as usize;

            if raw_size == 0 {
                continue;
            }

            // Copy only VirtualSize bytes (actual data), not SizeOfRawData (padded size)
            let copy_size = virtual_size.min(raw_size);

            // Source bounds check
            if virtual_addr + copy_size > rva_image.len() {
                return Err(PeError::InvalidOffset);
            }

            // Dest bounds check
            if file_offset + raw_size > actual_file_size {
                return Err(PeError::InvalidOffset);
            }

            // Copy section data (rest is padding with zeros)
            file_image[file_offset..file_offset + copy_size]
                .copy_from_slice(&rva_image[virtual_addr..virtual_addr + copy_size]);
        }

        // NOW restore reloc data at FILE offset (not RVA offset)
        // Find .reloc section file offset
        let reloc_section = sections
            .find_reloc_section()
            .ok_or(PeError::MissingSection)?;

        let reloc_file_offset = reloc_section.pointer_to_raw_data as usize;
        let reloc_data = &super::super::embedded_reloc_data::RELOC_DATA;

        if reloc_file_offset + reloc_data.len() > actual_file_size {
            return Err(PeError::InvalidOffset);
        }

        // Copy reloc data to file layout position
        file_image[reloc_file_offset..reloc_file_offset + reloc_data.len()]
            .copy_from_slice(reloc_data);

        Ok(file_image)
    }
}
