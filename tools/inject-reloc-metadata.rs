// Inject .reloc metadata into .morpheus section
// 
// Usage: inject-reloc bootloader.efi

use std::fs;
use std::io::{self, Write};

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <bootloader.efi>", args[0]);
        std::process::exit(1);
    }
    
    let efi_path = &args[1];
    let mut data = fs::read(efi_path)?;
    
    println!("Processing: {}", efi_path);
    println!("Original size: {} bytes", data.len());
    
    // Parse PE
    let pe_offset = u32::from_le_bytes([data[0x3C], data[0x3D], data[0x3E], data[0x3F]]) as usize;
    let section_count = u16::from_le_bytes([data[pe_offset + 6], data[pe_offset + 7]]) as usize;
    let opt_header_size = u16::from_le_bytes([data[pe_offset + 20], data[pe_offset + 21]]) as usize;
    let section_table_offset = pe_offset + 24 + opt_header_size;
    
    // Find .reloc section
    let mut reloc_rva = 0u32;
    let mut reloc_virtual_size = 0u32;
    let mut reloc_raw_size = 0u32;
    let mut reloc_file_offset = 0u32;
    
    for i in 0..section_count {
        let sec_off = section_table_offset + (i * 40);
        if &data[sec_off..sec_off + 6] == b".reloc" {
            reloc_virtual_size = u32::from_le_bytes([data[sec_off + 8], data[sec_off + 9], data[sec_off + 10], data[sec_off + 11]]);
            reloc_rva = u32::from_le_bytes([data[sec_off + 12], data[sec_off + 13], data[sec_off + 14], data[sec_off + 15]]);
            reloc_raw_size = u32::from_le_bytes([data[sec_off + 16], data[sec_off + 17], data[sec_off + 18], data[sec_off + 19]]);
            reloc_file_offset = u32::from_le_bytes([data[sec_off + 20], data[sec_off + 21], data[sec_off + 22], data[sec_off + 23]]);
            break;
        }
    }
    
    if reloc_raw_size == 0 {
        eprintln!("ERROR: No .reloc section found");
        std::process::exit(1);
    }
    
    println!("Found .reloc: RVA=0x{:X}, VirtualSize=0x{:X}, RawSize=0x{:X}, FileOffset=0x{:X}", 
             reloc_rva, reloc_virtual_size, reloc_raw_size, reloc_file_offset);
    
    // Extract reloc data
    let reloc_data = &data[reloc_file_offset as usize..(reloc_file_offset + reloc_raw_size) as usize];
    
    // Create morpheus section data
    let mut morpheus_data = Vec::new();
    morpheus_data.extend_from_slice(b"MRPH");  // Magic
    morpheus_data.extend_from_slice(&reloc_rva.to_le_bytes());
    morpheus_data.extend_from_slice(&reloc_raw_size.to_le_bytes());
    morpheus_data.extend_from_slice(reloc_data);
    
    // Pad to file alignment (512 bytes)
    let file_alignment = 512usize;
    let padded_size = (morpheus_data.len() + file_alignment - 1) / file_alignment * file_alignment;
    morpheus_data.resize(padded_size, 0);
    
    println!("Morpheus section: {} bytes (padded to {})", reloc_raw_size + 12, padded_size);
    
    // Calculate new section RVA (after last section)
    let mut last_section_end_rva = 0u32;
    let mut last_section_end_file = 0u32;
    
    for i in 0..section_count {
        let sec_off = section_table_offset + (i * 40);
        let virt_size = u32::from_le_bytes([data[sec_off + 8], data[sec_off + 9], data[sec_off + 10], data[sec_off + 11]]);
        let virt_addr = u32::from_le_bytes([data[sec_off + 12], data[sec_off + 13], data[sec_off + 14], data[sec_off + 15]]);
        let raw_size = u32::from_le_bytes([data[sec_off + 16], data[sec_off + 17], data[sec_off + 18], data[sec_off + 19]]);
        let raw_addr = u32::from_le_bytes([data[sec_off + 20], data[sec_off + 21], data[sec_off + 22], data[sec_off + 23]]);
        
        let section_align = 0x1000u32;
        let end_rva = (virt_addr + virt_size + section_align - 1) / section_align * section_align;
        if end_rva > last_section_end_rva {
            last_section_end_rva = end_rva;
        }
        
        let end_file = raw_addr + raw_size;
        if end_file > last_section_end_file {
            last_section_end_file = end_file;
        }
    }
    
    let morpheus_rva = last_section_end_rva;
    let morpheus_file_offset = last_section_end_file;
    
    println!("New .morpheus section: RVA=0x{:X}, FileOffset=0x{:X}", morpheus_rva, morpheus_file_offset);
    
    // Add morpheus section header
    let mut section_header = Vec::new();
    section_header.extend_from_slice(b".morphe\0");  // Name (8 bytes)
    section_header.extend_from_slice(&(morpheus_data.len() as u32).to_le_bytes());  // Virtual size
    section_header.extend_from_slice(&morpheus_rva.to_le_bytes());  // Virtual address
    section_header.extend_from_slice(&(morpheus_data.len() as u32).to_le_bytes());  // Raw size
    section_header.extend_from_slice(&morpheus_file_offset.to_le_bytes());  // Raw address
    section_header.extend_from_slice(&[0u8; 12]);  // Relocations, line numbers, etc
    section_header.extend_from_slice(&0x40000040u32.to_le_bytes());  // Characteristics: READABLE | INITIALIZED_DATA
    
    // Update section count
    data[pe_offset + 6] = ((section_count + 1) & 0xFF) as u8;
    data[pe_offset + 7] = (((section_count + 1) >> 8) & 0xFF) as u8;
    
    // Update SizeOfImage
    let size_of_image_offset = pe_offset + 24 + 56;
    let new_size_of_image = morpheus_rva + 0x1000;  // Round up to section alignment
    data[size_of_image_offset] = (new_size_of_image & 0xFF) as u8;
    data[size_of_image_offset + 1] = ((new_size_of_image >> 8) & 0xFF) as u8;
    data[size_of_image_offset + 2] = ((new_size_of_image >> 16) & 0xFF) as u8;
    data[size_of_image_offset + 3] = ((new_size_of_image >> 24) & 0xFF) as u8;
    
    // Insert section header after existing section headers
    let insert_pos = section_table_offset + (section_count * 40);
    data.splice(insert_pos..insert_pos, section_header);
    
    // Append morpheus data to end of file
    data.extend_from_slice(&morpheus_data);
    
    // Write modified file
    fs::write(efi_path, &data)?;
    
    println!("✓ Injected .morpheus section successfully");
    println!("Final size: {} bytes", data.len());
    
    Ok(())
}
