//! ELF reader against a hand-built image and malformed inputs.

use of_firmware::{ElfError, ElfImage};

/// Build a minimal ELF32 RISC-V executable: one code segment, one
/// data+bss segment, and a symbol table with two symbols.
fn tiny_elf() -> Vec<u8> {
    let code: Vec<u8> = [0x0000_0013u32, 0x0000_006f]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect(); // nop; j .
    let data = [1u8, 2, 3, 4];
    let strtab = b"\0_start\0counter\0.symtab\0.strtab\0".to_vec();
    let mut symtab = vec![0u8; 16]; // null symbol
    for (name, value, size, info) in [(1u32, 0x4200_0000u32, 8u32, 0x12u8), (8, 0x3fc8_0000, 4, 0x11)] {
        symtab.extend(name.to_le_bytes());
        symtab.extend(value.to_le_bytes());
        symtab.extend(size.to_le_bytes());
        symtab.extend([info, 0]);
        symtab.extend(1u16.to_le_bytes());
    }
    let phoff = 52u32;
    let code_off = phoff + 2 * 32;
    let data_off = code_off + code.len() as u32;
    let sym_off = data_off + data.len() as u32;
    let str_off = sym_off + symtab.len() as u32;
    let shoff = str_off + strtab.len() as u32;
    let mut f = Vec::new();
    f.extend(b"\x7fELF\x01\x01\x01\0\0\0\0\0\0\0\0\0");
    f.extend(2u16.to_le_bytes()); // ET_EXEC
    f.extend(243u16.to_le_bytes()); // EM_RISCV
    f.extend(1u32.to_le_bytes());
    f.extend(0x4200_0000u32.to_le_bytes()); // entry
    f.extend(phoff.to_le_bytes());
    f.extend(shoff.to_le_bytes());
    f.extend(1u32.to_le_bytes()); // e_flags: RVC
    f.extend(52u16.to_le_bytes());
    f.extend(32u16.to_le_bytes());
    f.extend(2u16.to_le_bytes());
    f.extend(40u16.to_le_bytes());
    f.extend(3u16.to_le_bytes()); // null, symtab, strtab
    f.extend(2u16.to_le_bytes()); // e_shstrndx: .strtab also names sections
    let ph = |off: u32, addr: u32, filesz: u32, memsz: u32, flags: u32| {
        let mut v = Vec::new();
        for x in [1, off, addr, addr, filesz, memsz, flags, 4] {
            v.extend(u32::to_le_bytes(x));
        }
        v
    };
    f.extend(ph(code_off, 0x4200_0000, code.len() as u32, code.len() as u32, 5));
    f.extend(ph(data_off, 0x3fc8_0000, 4, 16, 6));
    f.extend(&code);
    f.extend(data);
    f.extend(&symtab);
    f.extend(&strtab);
    let sh = |name: u32, typ: u32, off: u32, size: u32, link: u32, entsize: u32| {
        let mut v = Vec::new();
        for x in [name, typ, 0, 0, off, size, link, 0, 4, entsize] {
            v.extend(u32::to_le_bytes(x));
        }
        v
    };
    f.extend(vec![0u8; 40]);
    f.extend(sh(16, 2, sym_off, symtab.len() as u32, 2, 16));
    f.extend(sh(24, 3, str_off, strtab.len() as u32, 0, 0));
    f
}

#[test]
fn parses_entry_segments_and_symbols() {
    let img = ElfImage::parse(&tiny_elf()).unwrap();
    assert_eq!(img.entry, 0x4200_0000);
    assert_eq!(img.flags, 1);
    assert_eq!(img.segments.len(), 2);
    let code = &img.segments[0];
    assert!(code.is_exec());
    assert_eq!(code.vaddr, 0x4200_0000);
    assert_eq!(code.data.len(), 8);
    let data = &img.segments[1];
    assert!(!data.is_exec());
    assert_eq!((data.data.as_slice(), data.mem_size), (&[1u8, 2, 3, 4][..], 16));
    assert_eq!(img.symbol("_start").unwrap().value, 0x4200_0000);
    assert_eq!(img.symbol("counter").unwrap().size, 4);
    assert!(img.symbol("missing").is_none());
    let names: Vec<_> = img.sections.iter().map(|s| (s.name.as_str(), s.kind)).collect();
    assert_eq!(names, [(".symtab", 2), (".strtab", 3)]);
    let strtab = img.section(".strtab").unwrap();
    assert_eq!(strtab.data.len() as u32, strtab.size);
    assert!(strtab.data.starts_with(b"\0_start\0"));
}

#[test]
fn rejects_wrong_files_without_panicking() {
    assert_eq!(ElfImage::parse(b"hello"), Err(ElfError::NotElf));
    let mut f = tiny_elf();
    f[18] = 62; // x86-64
    assert!(matches!(ElfImage::parse(&f), Err(ElfError::Unsupported(_))));
    let f = tiny_elf();
    // Every truncation is an error, never a panic.
    for n in 4..f.len() {
        let r = ElfImage::parse(&f[..n]);
        assert!(r.is_err(), "truncated to {n} bytes parsed");
    }
    let mut f = tiny_elf();
    f[52 + 16] = 0xff; // p_filesz of segment 0 past the end
    f[52 + 17] = 0xff;
    assert!(matches!(ElfImage::parse(&f), Err(ElfError::Malformed(_))));
}
