//! Minimal ELF32 reader for RISC-V executables.
//!
//! Only what a simulator needs to boot firmware: the header, `PT_LOAD`
//! program headers, the section headers (names, addresses, contents) and
//! the `SHT_SYMTAB` symbol table. No relocation, no
//! dynamic linking. Every offset and size is bounds-checked, so a truncated
//! or hostile file produces an [`ElfError`], never a panic.

use thiserror::Error;

/// `e_machine` for RISC-V.
pub const EM_RISCV: u16 = 243;
const PT_LOAD: u32 = 1;
const SHT_SYMTAB: u32 = 2;
/// `sh_type` of a section with contents in the file.
pub const SHT_PROGBITS: u32 = 1;
/// `sh_type` of a section that occupies memory but no file space (`.bss`).
pub const SHT_NOBITS: u32 = 8;
/// `sh_flags` bit: the section occupies memory at run time.
pub const SHF_ALLOC: u32 = 2;
const ET_EXEC: u16 = 2;

/// Why an ELF file was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ElfError {
    /// Not an ELF file (bad magic).
    #[error("not an ELF file")]
    NotElf,
    /// ELF, but not 32-bit little-endian RISC-V executable.
    #[error("unsupported ELF: {0}")]
    Unsupported(String),
    /// A header, table or segment points outside the file.
    #[error("truncated or malformed ELF: {0}")]
    Malformed(String),
}

/// One loadable segment (`PT_LOAD`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Virtual (execution) address, `p_vaddr`.
    pub vaddr: u32,
    /// Physical (load) address, `p_paddr`.
    pub paddr: u32,
    /// Bytes from the file (`p_filesz`).
    pub data: Vec<u8>,
    /// Size in memory (`p_memsz`); bytes past `data` are zero (`.bss`).
    pub mem_size: u32,
    /// `p_flags`: bit 0 execute, bit 1 write, bit 2 read.
    pub flags: u32,
}

impl Segment {
    /// Whether the segment is executable.
    pub fn is_exec(&self) -> bool {
        self.flags & 1 != 0
    }
}

/// A symbol from `.symtab`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Name.
    pub name: String,
    /// Value (address for functions and objects).
    pub value: u32,
    /// Size in bytes (0 if unknown).
    pub size: u32,
}

/// One section header (`.text`, `.data`, ...), with its file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Name from the section header string table.
    pub name: String,
    /// `sh_type` (for example [`SHT_PROGBITS`] or [`SHT_NOBITS`]).
    pub kind: u32,
    /// `sh_flags` (for example [`SHF_ALLOC`]).
    pub flags: u32,
    /// `sh_addr`: the run-time address, 0 for non-memory sections.
    pub addr: u32,
    /// `sh_size`.
    pub size: u32,
    /// Contents from the file; empty for [`SHT_NOBITS`] sections.
    pub data: Vec<u8>,
}

/// A parsed ELF executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfImage {
    /// Entry point, `e_entry`.
    pub entry: u32,
    /// `e_flags` (RISC-V ABI flags: RVC bit 0, float ABI bits 1 to 2).
    pub flags: u32,
    /// Loadable segments in file order; empty ones are skipped.
    pub segments: Vec<Segment>,
    /// Named symbols (sections and file symbols excluded).
    pub symbols: Vec<Symbol>,
    /// Section headers in file order (index 0, the null section, is
    /// skipped). Some files, such as Espressif's ROM ELFs, keep data that is
    /// not in any `PT_LOAD` segment here.
    pub sections: Vec<Section>,
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn bytes(&self, off: u64, len: u64, what: &str) -> Result<&[u8], ElfError> {
        let end = off.checked_add(len).filter(|&e| e <= self.0.len() as u64);
        match end {
            Some(e) => Ok(&self.0[off as usize..e as usize]),
            None => Err(ElfError::Malformed(format!(
                "{what} at {off:#x}+{len:#x} is outside the file"
            ))),
        }
    }
    fn u16(&self, off: u64) -> Result<u16, ElfError> {
        let b = self.bytes(off, 2, "field")?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&self, off: u64) -> Result<u32, ElfError> {
        let b = self.bytes(off, 4, "field")?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

impl ElfImage {
    /// Parse an ELF32 little-endian RISC-V executable.
    pub fn parse(data: &[u8]) -> Result<Self, ElfError> {
        let r = Reader(data);
        if data.len() < 4 || &data[..4] != b"\x7fELF" {
            return Err(ElfError::NotElf);
        }
        if data.len() < 52 {
            return Err(ElfError::Malformed("header shorter than 52 bytes".into()));
        }
        if data[4] != 1 {
            return Err(ElfError::Unsupported("not 32-bit (EI_CLASS)".into()));
        }
        if data[5] != 1 {
            return Err(ElfError::Unsupported("not little-endian (EI_DATA)".into()));
        }
        let e_type = r.u16(16)?;
        let machine = r.u16(18)?;
        if machine != EM_RISCV {
            return Err(ElfError::Unsupported(format!(
                "e_machine {machine}, expected RISC-V ({EM_RISCV})"
            )));
        }
        if e_type != ET_EXEC {
            return Err(ElfError::Unsupported(format!(
                "e_type {e_type}, expected an executable (2)"
            )));
        }
        let entry = r.u32(24)?;
        let phoff = u64::from(r.u32(28)?);
        let shoff = u64::from(r.u32(32)?);
        let flags = r.u32(36)?;
        let phentsize = u64::from(r.u16(42)?);
        let phnum = u64::from(r.u16(44)?);
        let shentsize = u64::from(r.u16(46)?);
        let shnum = u64::from(r.u16(48)?);

        let mut segments = Vec::new();
        if phnum > 0 && phentsize < 32 {
            return Err(ElfError::Malformed(format!("e_phentsize {phentsize} < 32")));
        }
        for i in 0..phnum {
            let ph = phoff + i * phentsize;
            r.bytes(ph, 32, "program header")?;
            if r.u32(ph)? != PT_LOAD {
                continue;
            }
            let offset = u64::from(r.u32(ph + 4)?);
            let vaddr = r.u32(ph + 8)?;
            let paddr = r.u32(ph + 12)?;
            let filesz = r.u32(ph + 16)?;
            let mem_size = r.u32(ph + 20)?;
            let seg_flags = r.u32(ph + 24)?;
            if filesz > mem_size {
                return Err(ElfError::Malformed(format!("segment {i}: p_filesz > p_memsz")));
            }
            if mem_size == 0 {
                continue;
            }
            if u64::from(vaddr) + u64::from(mem_size) > 1 << 32 {
                return Err(ElfError::Malformed(format!("segment {i} wraps the address space")));
            }
            let bytes = r.bytes(offset, u64::from(filesz), "segment data")?;
            segments.push(Segment {
                vaddr,
                paddr,
                data: bytes.to_vec(),
                mem_size,
                flags: seg_flags,
            });
        }

        let sections = Self::parse_sections(&r, shoff, shentsize, shnum, u64::from(r.u16(50)?))?;
        let mut symbols = Vec::new();
        if shnum > 0 && shentsize >= 40 {
            for i in 0..shnum {
                let sh = shoff + i * shentsize;
                r.bytes(sh, 40, "section header")?;
                if r.u32(sh + 4)? != SHT_SYMTAB {
                    continue;
                }
                let off = u64::from(r.u32(sh + 16)?);
                let size = u64::from(r.u32(sh + 20)?);
                let link = u64::from(r.u32(sh + 24)?);
                let entsize = u64::from(r.u32(sh + 36)?).max(16);
                if link >= shnum {
                    return Err(ElfError::Malformed("symtab sh_link out of range".into()));
                }
                let strsh = shoff + link * shentsize;
                r.bytes(strsh, 40, "string table header")?;
                let stroff = u64::from(r.u32(strsh + 16)?);
                let strsize = u64::from(r.u32(strsh + 20)?);
                let strtab = r.bytes(stroff, strsize, "string table")?;
                for k in 0..size / entsize {
                    let s = off + k * entsize;
                    r.bytes(s, 16, "symbol")?;
                    let name_off = r.u32(s)? as usize;
                    let value = r.u32(s + 4)?;
                    let sym_size = r.u32(s + 8)?;
                    let typ = data[(s + 12) as usize] & 0xf;
                    // Skip sections (3) and files (4); keep notype, object, func.
                    if name_off == 0 || typ == 3 || typ == 4 || name_off >= strtab.len() {
                        continue;
                    }
                    let end = strtab[name_off..]
                        .iter()
                        .position(|&b| b == 0)
                        .map_or(strtab.len(), |p| name_off + p);
                    let name = String::from_utf8_lossy(&strtab[name_off..end]).into_owned();
                    symbols.push(Symbol {
                        name,
                        value,
                        size: sym_size,
                    });
                }
            }
        }
        Ok(Self {
            entry,
            flags,
            segments,
            symbols,
            sections,
        })
    }

    fn parse_sections(
        r: &Reader<'_>,
        shoff: u64,
        shentsize: u64,
        shnum: u64,
        shstrndx: u64,
    ) -> Result<Vec<Section>, ElfError> {
        if shnum == 0 || shentsize < 40 {
            return Ok(Vec::new());
        }
        let names = if shstrndx < shnum {
            let sh = shoff + shstrndx * shentsize;
            r.bytes(sh, 40, "section header")?;
            let off = u64::from(r.u32(sh + 16)?);
            let size = u64::from(r.u32(sh + 20)?);
            r.bytes(off, size, "section name table")?
        } else {
            &[]
        };
        let mut out = Vec::new();
        for i in 1..shnum {
            let sh = shoff + i * shentsize;
            r.bytes(sh, 40, "section header")?;
            let name_off = r.u32(sh)? as usize;
            let kind = r.u32(sh + 4)?;
            let flags = r.u32(sh + 8)?;
            let addr = r.u32(sh + 12)?;
            let off = u64::from(r.u32(sh + 16)?);
            let size = r.u32(sh + 20)?;
            let data = if kind == SHT_NOBITS || kind == 0 {
                Vec::new()
            } else {
                r.bytes(off, u64::from(size), "section data")?.to_vec()
            };
            let name = names.get(name_off..).map_or_else(String::new, |rest| {
                let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
                String::from_utf8_lossy(&rest[..end]).into_owned()
            });
            out.push(Section {
                name,
                kind,
                flags,
                addr,
                size,
                data,
            });
        }
        Ok(out)
    }

    /// Look up a section by name.
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// Look up a symbol by name.
    pub fn symbol(&self, name: &str) -> Option<&Symbol> {
        self.symbols.iter().find(|s| s.name == name)
    }
}
