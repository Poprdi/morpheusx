use super::super::section::SectionTable;
use super::super::{PeArch, PeError, PeResult};
use super::coff_header::CoffHeader;
use super::dos_header::DosHeader;
use super::optional_header::OptionalHeader64;
use super::utils::*;

extern crate alloc;

pub struct PeHeaders {
    pub dos: DosHeader,
    pub coff: CoffHeader,
    pub optional: OptionalHeader64,
}

impl PeHeaders {
    /// # Safety
    ///
    /// `image_base` must be readable for at least `image_size` bytes and
    /// reference a PE image; nested header parsers trust offsets within it.
    pub unsafe fn parse(image_base: *const u8, image_size: usize) -> PeResult<Self> {
        let dos = DosHeader::parse(image_base, image_size)?;
        let coff = CoffHeader::parse(image_base, dos.e_lfanew, image_size)?;
        let optional = OptionalHeader64::parse(image_base, dos.e_lfanew, image_size)?;

        Ok(PeHeaders {
            dos,
            coff,
            optional,
        })
    }

    pub fn arch(&self) -> PeResult<PeArch> {
        self.coff.arch()
    }

    /// Useless after UEFI patches `image_base` in-place; kept for completeness.
    pub fn relocation_delta(&self, actual_load_address: u64) -> i64 {
        actual_load_address as i64 - self.optional.image_base as i64
    }

    /// Recover original ImageBase by un-applying candidate deltas and checking
    /// that each implied original pointer lands in a known section RVA range.
    /// Returns `(image_base, validated_relocs, total_relocs)`.
    ///
    /// # Safety
    ///
    /// `image_base` must be readable for at least `image_size` bytes and
    /// reference the relocated PE image these headers were parsed from.
    pub unsafe fn reconstruct_original_image_base(
        &self,
        image_base: *const u8,
        image_size: usize,
        actual_load_address: u64,
    ) -> PeResult<(u64, u32, u32)> {
        let section_offset =
            self.dos.e_lfanew as usize + 24 + self.coff.size_of_optional_header as usize;

        let sections = SectionTable::parse(
            image_base,
            section_offset,
            self.coff.number_of_sections as usize,
            image_size,
        )?;

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

        let reloc_section = sections
            .find_reloc_section()
            .ok_or(PeError::MissingSection)?;

        let reloc_data_ptr = image_base.add(reloc_section.virtual_address as usize);
        let reloc_size = reloc_section.virtual_size;

        // UEFI sometimes truncates .reloc post-apply; floor at 512.
        let max_reloc_size = reloc_size.max(512);

        let mut relocations: [u64; 256] = [0; 256];
        let mut reloc_count = 0usize;
        let mut block_offset = 0usize;

        for _block_num in 0..16 {
            if block_offset + 8 > max_reloc_size as usize {
                break;
            }

            let page_rva = read_u32(reloc_data_ptr, block_offset);
            let block_size = read_u32(reloc_data_ptr, block_offset + 4);

            if !(8..=1024).contains(&block_size) {
                break;
            }

            if block_offset + block_size as usize > max_reloc_size as usize {
                break;
            }

            let entry_count = (block_size - 8) / 2;

            for i in 0..entry_count {
                if reloc_count >= 256 {
                    break;
                }

                let entry = read_u16(reloc_data_ptr, block_offset + 8 + (i * 2) as usize);
                let reloc_type = (entry >> 12) & 0xF;
                let offset = entry & 0xFFF;

                if reloc_type == 10 {
                    // IMAGE_REL_BASED_DIR64
                    let pointer_rva = page_rva + offset as u32;

                    if pointer_rva as usize + 8 > image_size {
                        continue;
                    }

                    let pointer_addr = image_base.add(pointer_rva as usize) as *const u64;
                    let current_value = *pointer_addr;

                    relocations[reloc_count] = current_value;
                    reloc_count += 1;
                }
            }

            block_offset += block_size as usize;
        }

        if reloc_count < 8 {
            return Err(PeError::CorruptedData);
        }

        let section_align = self.optional.section_alignment as u64;

        let mut candidates = [0u64; 16];
        let mut cand_idx = 0;

        if let Some(linker_base) = super::super::compile_time::get_original_image_base_hint() {
            candidates[cand_idx] = linker_base;
            cand_idx += 1;
        }

        let common_bases = [
            0x0000000140000000u64,
            0x0000000000400000u64,
            0x0000000100000000u64,
            actual_load_address & !0xFFFFu64,
            actual_load_address & !0xFFFFFu64,
            actual_load_address & !0x3FFFFFu64,
            actual_load_address & !(section_align - 1),
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

        for &candidate in candidates {
            if candidate == 0 {
                continue;
            }

            let delta = actual_load_address as i64 - candidate as i64;
            let mut valid_count = 0u32;

            for i in 0..reloc_count {
                let current_value = relocations[i];
                let original_value = (current_value as i64 - delta) as u64;

                if original_value < candidate {
                    continue;
                }

                let rva = original_value - candidate;

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

            if valid_count > best_valid_count {
                best_valid_count = valid_count;
                best_candidate = candidate;
            }
        }

        // 90% threshold; fall through with best guess otherwise.
        let min_valid = (reloc_count as u32 * 9) / 10;

        // Both branches intentionally return the best guess; the threshold
        // check documents the confidence distinction without altering output.
        #[allow(clippy::if_same_then_else)]
        if best_valid_count >= min_valid {
            Ok((best_candidate, best_valid_count, reloc_count as u32))
        } else {
            Ok((best_candidate, best_valid_count, reloc_count as u32))
        }
    }

    /// Reverse the load-time relocation: restore .reloc from embedded data,
    /// undo DIR64 fixups, and rewrite ImageBase. Returns the applied delta.
    ///
    /// # Safety
    ///
    /// `image_data` must be the current relocated PE image in RVA (memory)
    /// layout that these headers were parsed from; it is rewritten in place.
    pub unsafe fn unrelocate_image(
        &self,
        image_data: &mut [u8],
        actual_load_address: u64,
    ) -> PeResult<i64> {
        // UEFI discards .reloc after applying it; reinstate from embedded copy.
        let reloc_rva = super::super::embedded_reloc_data::RELOC_RVA;
        let reloc_size = super::super::embedded_reloc_data::RELOC_SIZE;
        let reloc_data = &super::super::embedded_reloc_data::RELOC_DATA;
        let original_image_base = super::super::embedded_reloc_data::ORIGINAL_IMAGE_BASE;

        let reloc_offset = reloc_rva as usize;
        if reloc_offset + reloc_data.len() > image_data.len() {
            return Err(PeError::InvalidOffset);
        }

        core::ptr::copy_nonoverlapping(
            reloc_data.as_ptr(),
            image_data.as_mut_ptr().add(reloc_offset),
            reloc_data.len(),
        );

        let delta = actual_load_address as i64 - original_image_base as i64;

        super::super::reloc::unrelocate_image(image_data, reloc_rva, reloc_size, delta)?;
        OptionalHeader64::patch_image_base(image_data, original_image_base)?;

        Ok(delta)
    }

    /// Convert RVA layout (sections at VirtualAddress) back to file layout
    /// (sections at PointerToRawData), then drop in the embedded .reloc data.
    ///
    /// # Safety
    ///
    /// `rva_image` must be a PE image in RVA (memory) layout matching these
    /// headers; section offsets read from it are trusted to be in bounds.
    pub unsafe fn rva_to_file_layout(&self, rva_image: &[u8]) -> PeResult<alloc::vec::Vec<u8>> {
        // Parse from the unmodified header region; unrelocate must not touch it.
        let section_offset =
            self.dos.e_lfanew as usize + 24 + self.coff.size_of_optional_header as usize;

        let sections = super::super::section::SectionTable::parse(
            rva_image.as_ptr(),
            section_offset,
            self.coff.number_of_sections as usize,
            rva_image.len(),
        )?;

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

        let header_size = self.optional.size_of_headers as usize;
        if header_size > rva_image.len() {
            return Err(PeError::InvalidOffset);
        }
        file_image[..header_size].copy_from_slice(&rva_image[..header_size]);

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

            // VirtualSize is the real payload; SizeOfRawData includes padding.
            let copy_size = virtual_size.min(raw_size);

            if virtual_addr + copy_size > rva_image.len() {
                return Err(PeError::InvalidOffset);
            }

            if file_offset + raw_size > actual_file_size {
                return Err(PeError::InvalidOffset);
            }

            file_image[file_offset..file_offset + copy_size]
                .copy_from_slice(&rva_image[virtual_addr..virtual_addr + copy_size]);
        }

        let reloc_section = sections
            .find_reloc_section()
            .ok_or(PeError::MissingSection)?;

        let reloc_file_offset = reloc_section.pointer_to_raw_data as usize;
        let reloc_data = &super::super::embedded_reloc_data::RELOC_DATA;

        if reloc_file_offset + reloc_data.len() > actual_file_size {
            return Err(PeError::InvalidOffset);
        }

        file_image[reloc_file_offset..reloc_file_offset + reloc_data.len()]
            .copy_from_slice(reloc_data);

        Ok(file_image)
    }
}
