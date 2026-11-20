//! ESP flash image reader and writer: a hand-built image, round trips,
//! the blinky ELF converted, and malformed inputs.

use of_firmware::{ElfImage, EspImage, ImageError, ImageSegment, sha256};

/// One segment (4 bytes at 0x3fc8_0000), no digest, built byte by byte
/// from the documented layout.
fn hand_built() -> Vec<u8> {
    let mut f = vec![0xe9, 1, 2, 0x20];
    f.extend(0x4038_0000u32.to_le_bytes());
    f.extend([0xee, 0, 0, 0]); // wp pin, pin drive
    f.extend(5u16.to_le_bytes()); // ESP32-C3
    f.push(0); // min_chip_rev
    f.extend(3u16.to_le_bytes()); // min_chip_rev_full v0.3
    f.extend(199u16.to_le_bytes()); // max_chip_rev_full v1.99
    f.extend([0; 4]);
    f.push(0); // no hash
    assert_eq!(f.len(), 24);
    f.extend(0x3fc8_0000u32.to_le_bytes());
    f.extend(4u32.to_le_bytes());
    f.extend([1, 2, 3, 4]);
    f.resize(47, 0);
    f.push(0xef ^ 1 ^ 2 ^ 3 ^ 4);
    f
}

#[test]
fn parses_a_hand_built_image() {
    let img = EspImage::parse(&hand_built()).unwrap();
    assert_eq!(img.entry, 0x4038_0000);
    assert_eq!(img.chip_id, 5);
    assert_eq!((img.spi_mode, img.spi_speed, img.spi_size), (2, 0, 2));
    assert_eq!(img.wp_pin, 0xee);
    assert_eq!((img.min_chip_rev_full, img.max_chip_rev_full), (3, 199));
    assert!(!img.hash_appended);
    assert_eq!(
        img.segments,
        vec![ImageSegment {
            addr: 0x3fc8_0000,
            data: vec![1, 2, 3, 4]
        }]
    );
    assert_eq!(img.to_bytes(), hand_built(), "writer produces the same bytes");
    // Trailing flash contents are ignored.
    let mut padded = hand_built();
    padded.extend([0xff; 100]);
    assert_eq!(EspImage::parse(&padded), Ok(img));
}

#[test]
fn digest_is_appended_and_checked() {
    let mut img = EspImage::parse(&hand_built()).unwrap();
    img.hash_appended = true;
    let bytes = img.to_bytes();
    assert_eq!(bytes.len(), 48 + 32);
    assert_eq!(bytes[23], 1);
    assert_eq!(&bytes[48..], &sha256(&bytes[..48]));
    assert_eq!(EspImage::parse(&bytes), Ok(img));
    let mut bad = bytes.clone();
    bad[79] ^= 1;
    assert_eq!(EspImage::parse(&bad), Err(ImageError::Digest));
    assert!(matches!(EspImage::parse(&bytes[..70]), Err(ImageError::Truncated(_))));
}

#[test]
fn checksum_lands_on_the_last_byte_of_a_16_byte_block() {
    // Segment lengths 0..=20 move the end of the data across block edges.
    for len in 0..=20u8 {
        let mut img = EspImage::new(5, 0x4200_0000);
        img.segments.push(ImageSegment {
            addr: 0x4200_0000,
            data: (0..len).collect(),
        });
        img.hash_appended = false;
        let bytes = img.to_bytes();
        assert_eq!(bytes.len() % 16, 0, "len {len}");
        assert!(bytes.len() > 24 + 8 + usize::from(len));
        assert_eq!(*bytes.last().unwrap(), img.checksum());
        assert_eq!(EspImage::parse(&bytes), Ok(img));
    }
}

#[test]
fn rejects_malformed_images() {
    let good = hand_built();
    let mut bad = good.clone();
    bad[0] = 0xe8;
    assert_eq!(EspImage::parse(&bad), Err(ImageError::BadMagic(0xe8)));
    let mut bad = good.clone();
    bad[33] ^= 0xff;
    assert!(matches!(EspImage::parse(&bad), Err(ImageError::Checksum { .. })));
    let mut bad = good.clone();
    bad[1] = 17;
    assert_eq!(EspImage::parse(&bad), Err(ImageError::TooManySegments(17)));
    let mut bad = good.clone();
    bad[28..32].copy_from_slice(&u32::MAX.to_le_bytes()); // segment length
    assert!(matches!(EspImage::parse(&bad), Err(ImageError::Truncated(_))));
    for cut in [0, 10, 23, 30, 40, 47] {
        assert!(
            matches!(EspImage::parse(&good[..cut]), Err(ImageError::Truncated(_))),
            "cut at {cut}"
        );
    }
}

#[test]
fn blinky_elf_converts_and_round_trips() {
    let elf = ElfImage::parse(include_bytes!("../../../firmware/esp32c3-blinky/blinky.elf")).unwrap();
    let img = EspImage::from_elf(&elf, 5);
    assert_eq!(img.entry, 0x4200_0020);
    let addrs: Vec<u32> = img.segments.iter().map(|s| s.addr).collect();
    assert_eq!(addrs, [0x4200_0020, 0x3c01_0020, 0x3fc8_0000]);
    let bytes = img.to_bytes();
    assert_eq!(EspImage::parse(&bytes), Ok(img));
}

#[test]
fn flash_mapped_segments_are_aligned_to_mmu_pages() {
    let mut img = EspImage::new(5, 0x4200_0020);
    img.segments = vec![
        ImageSegment {
            addr: 0x3c01_0020,
            data: vec![1; 40],
        },
        ImageSegment {
            addr: 0x3fc8_0000,
            data: vec![2; 12],
        },
        ImageSegment {
            addr: 0x4202_0020,
            data: vec![3; 100],
        },
    ];
    let mapped = |a: u32| (0x3c00_0000..0x3c80_0000).contains(&a) || (0x4200_0000..0x4280_0000).contains(&a);
    img.align_flash_segments(0x1_0000, mapped).expect("fits");
    let offsets = img.data_offsets();
    for (seg, off) in img.segments.iter().zip(&offsets) {
        if mapped(seg.addr) {
            assert_eq!((0x1_0000 + off) % 0x1_0000, seg.addr % 0x1_0000, "{:#x}", seg.addr);
        }
    }
    assert!(img.segments.iter().any(|s| s.addr == 0), "a padding segment was needed");
    // Round trip keeps the padding and the layout.
    let parsed = EspImage::parse(&img.to_bytes()).expect("valid");
    assert_eq!(parsed.data_offsets(), offsets);
    assert_eq!(parsed.segments.len(), img.segments.len());
}
