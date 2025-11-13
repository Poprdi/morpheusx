//! Embed relocation metadata in custom PE section
//! 
//! This extracts .reloc section data and embeds it in a .morpheus section
//! that UEFI won't discard after loading

use std::fs;
use std::io::{self, Read, Write, Seek, SeekFrom};

const SECTION_ALIGNMENT: u32 = 0x1000;
const FILE_ALIGNMENT: u32 = 0x200;

pub fn embed_reloc_metadata(efi_path: &str) -> io::Result<()> {
    let mut data = fs::read(efi_path)?;
    
    // Parse PE headers
    let dos_magic = u16::from_le_bytes([data[0], data[1]]);
    if dos_magic != 0x5A4D {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Not a PE file"));
    }
    
    let pe_offset = u32::from_le_bytes([data[0x3C], data[0x3D], data[0x3E], data[0x3F]]) as usize;
    
    let pe_sig = u32::from_le_bytes([
        data[pe_offset],
        data[pe_offset + 1],
        data[pe_offset + 2],
        data[pe_offset + 3],
    ]);
    
    if pe_sig != 0x4550 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid PE signature"));
    }
    
    // Get section count and optional header size
    let section_count = u16::from_le_bytes([
        data[pe_offset + 6],
        data[pe_offset + 7],
    ]) as usize;
    
    let opt_header_size = u16::from_le_bytes([
        data[pe_offset + 20],
        data[pe_offset + 21],
    ]) as usize;
    
    // Section table starts after optional header
    let section_table_offset = pe_offset + 24 + opt_header_size;
    
    // Find .reloc section
    let mut reloc_rva = 0u32;
    let mut reloc_size = 0u32;
    let mut reloc_file_offset = 0u32;
    
    for i in 0..section_count {
        let sec_offset = section_table_offset + (i * 40);
        let name = &data[sec_offset..sec_offset + 8];
        
        if &name[0..6] == b".reloc" {
            reloc_size = u32::from_le_bytes([
                data[sec_offset + 8],
                data[sec_offset + 9],
                data[sec_offset + 10],
                data[sec_offset + 11],
            ]);
            
            reloc_rva = u32::from_le_bytes([
                data[sec_offset + 12],
                data[sec_offset + 13],
                data[sec_offset + 14],
                data[sec_offset + 15],
            ]);
            
            reloc_file_offset = u32::from_le_bytes([
                data[sec_offset + 20],
                data[sec_offset + 21],
                data[sec_offset + 22],
                data[sec_offset + 23],
            ]);
            
            break;
        }
    }
    
    if reloc_size == 0 {
        return Err(io::Error::new(io::ErrorKind::NotFound, "No .reloc section"));
    }
    
    println!("Found .reloc: RVA=0x{:X}, Size=0x{:X}, FileOffset=0x{:X}", 
             reloc_rva, reloc_size, reloc_file_offset);
    
    // Extract reloc data
    let reloc_data = &data[reloc_file_offset as usize..(reloc_file_offset + reloc_size) as usize];
    
    // Create .morpheus section with reloc metadata
    let morpheus_data = create_morpheus_section(reloc_data, reloc_rva);
    
    // Add new section (simplified - you'd need to update headers properly)
    println!("Reloc metadata size: {} bytes", morpheus_data.len());
    println!("This would be embedded in .morpheus section");
    
    // TODO: Actually append section to PE file
    
    Ok(())
}

fn create_morpheus_section(reloc_data: &[u8], reloc_rva: u32) -> Vec<u8> {
    let mut result = Vec::new();
    
    // Header: magic + original RVA + size
    result.extend_from_slice(b"MRPH");  // Magic
    result.extend_from_slice(&reloc_rva.to_le_bytes());
    result.extend_from_slice(&(reloc_data.len() as u32).to_le_bytes());
    
    // Copy reloc data
    result.extend_from_slice(reloc_data);
    
    result
}
