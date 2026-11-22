use std::env;
use std::fs;
use std::path::PathBuf;

struct ResLeaf {
    id: u16,
    data: Vec<u8>,
}

fn push_dir_header(buf: &mut Vec<u8>, num_id_entries: u16) {
    buf.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    buf.extend_from_slice(&0u32.to_le_bytes()); // TimeDateStamp
    buf.extend_from_slice(&0u16.to_le_bytes()); // MajorVersion
    buf.extend_from_slice(&0u16.to_le_bytes()); // MinorVersion
    buf.extend_from_slice(&0u16.to_le_bytes()); // NumberOfNamedEntries
    buf.extend_from_slice(&num_id_entries.to_le_bytes());
}

fn push_dir_entry(buf: &mut Vec<u8>, id: u32, offset_or_subdir: u32) {
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&offset_or_subdir.to_le_bytes());
}

fn build_coff_rsrc_from_ico(ico: &[u8]) -> Result<Vec<u8>, String> {
    if ico.len() < 6 {
        return Err("ICO too small".into());
    }
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    if count == 0 || ico.len() < 6 + count * 16 {
        return Err("Invalid ICO directory".into());
    }

    let mut icons: Vec<ResLeaf> = Vec::with_capacity(count);
    let mut grp = Vec::new();
    grp.extend_from_slice(&0u16.to_le_bytes()); // idReserved
    grp.extend_from_slice(&1u16.to_le_bytes()); // idType = ICON
    grp.extend_from_slice(&(count as u16).to_le_bytes());

    for i in 0..count {
        let base = 6 + i * 16;
        let width = ico[base];
        let height = ico[base + 1];
        let color_count = ico[base + 2];
        let reserved = ico[base + 3];
        let planes = u16::from_le_bytes([ico[base + 4], ico[base + 5]]);
        let bit_count = u16::from_le_bytes([ico[base + 6], ico[base + 7]]);
        let bytes_in_res =
            u32::from_le_bytes([ico[base + 8], ico[base + 9], ico[base + 10], ico[base + 11]]);
        let img_offset = u32::from_le_bytes([
            ico[base + 12],
            ico[base + 13],
            ico[base + 14],
            ico[base + 15],
        ]) as usize;

        let end = img_offset + bytes_in_res as usize;
        if end > ico.len() {
            return Err("ICO image out of bounds".into());
        }
        let id = (i + 1) as u16;
        icons.push(ResLeaf {
            id,
            data: ico[img_offset..end].to_vec(),
        });

        grp.push(width);
        grp.push(height);
        grp.push(color_count);
        grp.push(reserved);
        grp.extend_from_slice(&(if planes == 0 { 1 } else { planes }).to_le_bytes());
        grp.extend_from_slice(&(if bit_count == 0 { 32 } else { bit_count }).to_le_bytes());
        grp.extend_from_slice(&bytes_in_res.to_le_bytes());
        grp.extend_from_slice(&id.to_le_bytes());
    }

    // Sorted ascending resource types: RT_ICON = 3, RT_GROUP_ICON = 14
    let types: Vec<(u16, Vec<ResLeaf>)> = vec![
        (3, icons),
        (
            14,
            vec![ResLeaf {
                id: 1,
                data: grp,
            }],
        ),
    ];

    // Precompute deterministic layout offsets inside `.rsrc` section
    let root_off = 0usize;
    let root_size = 16 + types.len() * 8;

    let mut cur = root_off + root_size;
    let mut type_subdir_offs = Vec::new();
    for (_, leaves) in &types {
        type_subdir_offs.push(cur);
        cur += 16 + leaves.len() * 8;
    }

    let mut lang_subdir_offs = Vec::new();
    for (_, leaves) in &types {
        let mut v = Vec::new();
        for _ in leaves {
            v.push(cur);
            cur += 16 + 8; // 1 lang entry (0x0409)
        }
        lang_subdir_offs.push(v);
    }

    let mut data_entry_offs = Vec::new();
    for (_, leaves) in &types {
        let mut v = Vec::new();
        for _ in leaves {
            v.push(cur);
            cur += 16; // IMAGE_RESOURCE_DATA_ENTRY
        }
        data_entry_offs.push(v);
    }

    while cur % 8 != 0 {
        cur += 1;
    }

    let mut payload_offs = Vec::new();
    for (_, leaves) in &types {
        let mut v = Vec::new();
        for leaf in leaves {
            v.push(cur);
            cur += leaf.data.len();
            while cur % 8 != 0 {
                cur += 1;
            }
        }
        payload_offs.push(v);
    }

    // Build `.rsrc` raw bytes
    let mut rsrc = Vec::with_capacity(cur);
    let mut relocs: Vec<u32> = Vec::new();

    // 1. Root dir
    push_dir_header(&mut rsrc, types.len() as u16);
    for (ti, (type_id, _)) in types.iter().enumerate() {
        push_dir_entry(
            &mut rsrc,
            *type_id as u32,
            0x8000_0000u32 | type_subdir_offs[ti] as u32,
        );
    }

    // 2. Type subdirs (Resource IDs)
    for (ti, (_, leaves)) in types.iter().enumerate() {
        push_dir_header(&mut rsrc, leaves.len() as u16);
        for (li, leaf) in leaves.iter().enumerate() {
            push_dir_entry(
                &mut rsrc,
                leaf.id as u32,
                0x8000_0000u32 | lang_subdir_offs[ti][li] as u32,
            );
        }
    }

    // 3. Language subdirs (en-US = 0x0409)
    for (ti, (_, leaves)) in types.iter().enumerate() {
        for li in 0..leaves.len() {
            push_dir_header(&mut rsrc, 1);
            push_dir_entry(&mut rsrc, 0x0409, data_entry_offs[ti][li] as u32);
        }
    }

    // 4. IMAGE_RESOURCE_DATA_ENTRY records (with relocations on OffsetToData)
    for (ti, (_, leaves)) in types.iter().enumerate() {
        for (li, leaf) in leaves.iter().enumerate() {
            let reloc_va = rsrc.len() as u32;
            relocs.push(reloc_va);
            rsrc.extend_from_slice(&(payload_offs[ti][li] as u32).to_le_bytes()); // OffsetToData (relocated via ADDR32NB)
            rsrc.extend_from_slice(&(leaf.data.len() as u32).to_le_bytes()); // Size
            rsrc.extend_from_slice(&0u32.to_le_bytes()); // CodePage
            rsrc.extend_from_slice(&0u32.to_le_bytes()); // Reserved
        }
    }

    while rsrc.len() % 8 != 0 {
        rsrc.push(0);
    }

    // 5. Payloads
    for (_, leaves) in &types {
        for leaf in leaves {
            rsrc.extend_from_slice(&leaf.data);
            while rsrc.len() % 8 != 0 {
                rsrc.push(0);
            }
        }
    }

    // Assemble COFF `.o` file (`x86_64`)
    let raw_ptr = 60u32;
    let relocs_ptr = raw_ptr + rsrc.len() as u32;
    let symtab_ptr = relocs_ptr + (relocs.len() * 10) as u32;

    let mut coff = Vec::with_capacity(symtab_ptr as usize + 22);
    // IMAGE_FILE_HEADER (20 bytes)
    coff.extend_from_slice(&0x8664u16.to_le_bytes()); // Machine = AMD64
    coff.extend_from_slice(&1u16.to_le_bytes()); // NumberOfSections = 1
    coff.extend_from_slice(&0u32.to_le_bytes()); // TimeDateStamp
    coff.extend_from_slice(&symtab_ptr.to_le_bytes()); // PointerToSymbolTable
    coff.extend_from_slice(&1u32.to_le_bytes()); // NumberOfSymbols
    coff.extend_from_slice(&0u16.to_le_bytes()); // SizeOfOptionalHeader
    coff.extend_from_slice(&0u16.to_le_bytes()); // Characteristics

    // IMAGE_SECTION_HEADER `.rsrc` (40 bytes)
    coff.extend_from_slice(b".rsrc\0\0\0");
    coff.extend_from_slice(&0u32.to_le_bytes()); // VirtualSize
    coff.extend_from_slice(&0u32.to_le_bytes()); // VirtualAddress
    coff.extend_from_slice(&(rsrc.len() as u32).to_le_bytes()); // SizeOfRawData
    coff.extend_from_slice(&raw_ptr.to_le_bytes()); // PointerToRawData
    coff.extend_from_slice(&relocs_ptr.to_le_bytes()); // PointerToRelocations
    coff.extend_from_slice(&0u32.to_le_bytes()); // PointerToLinenumbers
    coff.extend_from_slice(&(relocs.len() as u16).to_le_bytes()); // NumberOfRelocations
    coff.extend_from_slice(&0u16.to_le_bytes()); // NumberOfLinenumbers
    coff.extend_from_slice(&0x4000_0040u32.to_le_bytes()); // IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ

    // Section raw bytes
    coff.extend_from_slice(&rsrc);

    // Relocations (`IMAGE_REL_AMD64_ADDR32NB` = 0x0003)
    for va in relocs {
        coff.extend_from_slice(&va.to_le_bytes());
        coff.extend_from_slice(&0u32.to_le_bytes()); // symbol index 0 (`.rsrc`)
        coff.extend_from_slice(&0x0003u16.to_le_bytes());
    }

    // Symbol table (1 static section symbol `.rsrc`)
    coff.extend_from_slice(b".rsrc\0\0\0");
    coff.extend_from_slice(&0u32.to_le_bytes()); // Value
    coff.extend_from_slice(&1i16.to_le_bytes()); // SectionNumber (1-based)
    coff.extend_from_slice(&0u16.to_le_bytes()); // Type
    coff.push(3u8); // IMAGE_SYM_CLASS_STATIC
    coff.push(0u8); // NumberOfAuxSymbols

    // Empty string table (4 bytes)
    coff.extend_from_slice(&4u32.to_le_bytes());
    Ok(coff)
}

fn main() {
    println!("cargo:rerun-if-changed=ui/cinder.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64")
    {
        let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let ico_path = manifest_dir.join("ui").join("cinder.ico");
        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
        let obj_path = out_dir.join("cinder_rsrc.o");
        if let Ok(ico) = fs::read(&ico_path) {
            if let Ok(coff_bytes) = build_coff_rsrc_from_ico(&ico) {
                if fs::write(&obj_path, coff_bytes).is_ok() {
                    println!("cargo:rustc-link-arg-bins={}", obj_path.display());
                }
            }
        }
    }
}
