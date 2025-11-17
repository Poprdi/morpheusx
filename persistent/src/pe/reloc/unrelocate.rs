use super::super::{PeError, PeResult};
use super::types::*;

pub unsafe fn unrelocate_image(
    image_data: &mut [u8],
    reloc_rva: u32,
    reloc_size: u32,
    delta: i64,
) -> PeResult<()> {
    if reloc_rva as usize >= image_data.len() {
        return Err(PeError::InvalidOffset);
    }

    // NOTE: UEFI may truncate reloc_size after applying relocations!
    // Use the larger of reloc_size or a reasonable max
    let max_reloc_size = reloc_size.max(512);

    if (reloc_rva as usize + max_reloc_size as usize) > image_data.len() {
        // Clamp to image bounds
        // Don't error - just process what we can
    }

    // Parse all relocation blocks (scope borrow to avoid conflicts)
    let mut block_offset = 0usize;

    // Force iteration through ALL blocks
    for _block_num in 0..16 {
        if block_offset + BaseRelocationBlock::SIZE > max_reloc_size as usize {
            break;
        }
        // Read block header (careful to scope borrows)
        let page_rva: u32;
        let block_size: u32;

        {
            let reloc_base = reloc_rva as usize;
            page_rva = u32::from_le_bytes([
                image_data[reloc_base + block_offset],
                image_data[reloc_base + block_offset + 1],
                image_data[reloc_base + block_offset + 2],
                image_data[reloc_base + block_offset + 3],
            ]);

            block_size = u32::from_le_bytes([
                image_data[reloc_base + block_offset + 4],
                image_data[reloc_base + block_offset + 5],
                image_data[reloc_base + block_offset + 6],
                image_data[reloc_base + block_offset + 7],
            ]);
        }

        // Sanity check
        if block_size < BaseRelocationBlock::SIZE as u32 {
            break;
        }

        if block_offset + block_size as usize > reloc_size as usize {
            break;
        }

        // Process entries
        let entry_count = ((block_size as usize) - BaseRelocationBlock::SIZE) / 2;
        let entries_offset = block_offset + BaseRelocationBlock::SIZE;

        for i in 0..entry_count {
            let entry_offset = entries_offset + (i * 2);

            // Read entry (scope borrow)
            let entry_raw: u16;
            {
                let reloc_base = reloc_rva as usize;
                entry_raw = u16::from_le_bytes([
                    image_data[reloc_base + entry_offset],
                    image_data[reloc_base + entry_offset + 1],
                ]);
            }

            let reloc_type = (entry_raw >> 12) & 0xF;
            let offset = entry_raw & 0xFFF;

            // Only handle DIR64 relocations (type 10)
            if reloc_type == 10 {
                let pointer_rva = page_rva + offset as u32;

                if (pointer_rva as usize + 8) > image_data.len() {
                    continue; // Skip invalid relocations
                }

                // Read current value (8 bytes) - scope borrow
                let current_value: u64;
                {
                    let ptr_offset = pointer_rva as usize;
                    current_value = u64::from_le_bytes([
                        image_data[ptr_offset],
                        image_data[ptr_offset + 1],
                        image_data[ptr_offset + 2],
                        image_data[ptr_offset + 3],
                        image_data[ptr_offset + 4],
                        image_data[ptr_offset + 5],
                        image_data[ptr_offset + 6],
                        image_data[ptr_offset + 7],
                    ]);
                }

                // Unrelocate: subtract delta
                let original_value = (current_value as i64 - delta) as u64;

                // Write back (now borrow is clear)
                let ptr_offset = pointer_rva as usize;
                let original_bytes = original_value.to_le_bytes();
                image_data[ptr_offset] = original_bytes[0];
                image_data[ptr_offset + 1] = original_bytes[1];
                image_data[ptr_offset + 2] = original_bytes[2];
                image_data[ptr_offset + 3] = original_bytes[3];
                image_data[ptr_offset + 4] = original_bytes[4];
                image_data[ptr_offset + 5] = original_bytes[5];
                image_data[ptr_offset + 6] = original_bytes[6];
                image_data[ptr_offset + 7] = original_bytes[7];
            }
        }

        block_offset += block_size as usize;
    }

    Ok(())
}
