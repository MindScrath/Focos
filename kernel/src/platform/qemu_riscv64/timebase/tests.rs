//! 独立组装 FDT 字节，覆盖格式、路径、完整遍历与截断；不访问裸地址。
extern crate std;
use super::*;
use std::{vec, vec::Vec};

fn word(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_be_bytes());
}
fn begin(out: &mut Vec<u8>, name: &[u8]) {
    word(out, 1);
    out.extend(name);
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}
fn prop(out: &mut Vec<u8>, name: u32, value: &[u8]) {
    word(out, 3);
    word(out, value.len() as u32);
    word(out, name);
    out.extend(value);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}
fn blob(structure: &[u8], strings: &[u8]) -> Vec<u8> {
    // 40 字节头、16 字节空保留表、结构、字符串；每个偏移独立填入。
    let mut out = Vec::new();
    for v in [
        0xd00dfeed,
        (56 + structure.len() + strings.len()) as u32,
        56,
        (56 + structure.len()) as u32,
        40,
        17,
        16,
        0,
        strings.len() as u32,
        structure.len() as u32,
    ] {
        word(&mut out, v);
    }
    out.extend([0; 16]);
    out.extend(structure);
    out.extend(strings);
    out
}
fn structure(freq: Option<&[u8]>) -> Vec<u8> {
    let mut s = Vec::new();
    begin(&mut s, b"");
    begin(&mut s, b"cpus");
    if let Some(f) = freq {
        prop(&mut s, 0, f);
    }
    begin(&mut s, b"cpu@0");
    word(&mut s, 2);
    word(&mut s, 2);
    word(&mut s, 2);
    word(&mut s, 9);
    s
}
fn valid() -> Vec<u8> {
    blob(
        &structure(Some(&10_000_000u32.to_be_bytes())),
        b"timebase-frequency\0",
    )
}
fn set(b: &mut [u8], offset: usize, value: u32) {
    b[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

#[test]
fn reads_real_qemu_shape_and_non_default_frequency() {
    for f in [1, 10_000_000, 32_768, u32::MAX] {
        let b = blob(&structure(Some(&f.to_be_bytes())), b"timebase-frequency\0");
        assert_eq!(parse_timebase_frequency(&b).unwrap().get(), f);
    }
}
#[test]
fn every_truncation_is_rejected() {
    let b = valid();
    for n in 0..b.len() {
        assert_eq!(
            parse_timebase_frequency(&b[..n]),
            Err(TimebaseError::MalformedDtb),
            "length {n}"
        );
    }
}
#[test]
fn frequency_errors_are_distinct() {
    assert_eq!(
        parse_timebase_frequency(&blob(&structure(None), b"timebase-frequency\0")),
        Err(TimebaseError::MissingFrequency)
    );
    for bytes in [vec![], vec![0; 3], vec![0; 4], vec![0; 8]] {
        assert_eq!(
            parse_timebase_frequency(&blob(&structure(Some(&bytes)), b"timebase-frequency\0")),
            Err(TimebaseError::InvalidFrequency)
        );
    }
}
#[test]
fn rejects_bad_header_and_block_ranges() {
    for (offset, value) in [
        (0, 0),
        (4, 39),
        (4, u32::MAX),
        (8, 40),
        (8, 57),
        (8, u32::MAX),
        (12, 56),
        (12, u32::MAX),
        (16, 41),
        (16, u32::MAX),
        (32, u32::MAX),
        (36, u32::MAX),
    ] {
        let mut b = valid();
        set(&mut b, offset, value);
        assert_eq!(
            parse_timebase_frequency(&b),
            Err(TimebaseError::MalformedDtb),
            "header {offset}={value}"
        );
    }
}
#[test]
fn rejects_unsupported_versions_but_accepts_compatible_future_header() {
    for (version, compat) in [(16, 16), (18, 18), (17, 18)] {
        let mut b = valid();
        set(&mut b, 20, version);
        set(&mut b, 24, compat);
        assert_eq!(
            parse_timebase_frequency(&b),
            Err(TimebaseError::UnsupportedVersion)
        );
    }
    let mut b = valid();
    set(&mut b, 20, 18);
    assert_eq!(parse_timebase_frequency(&b).unwrap().get(), 10_000_000);
}
#[test]
fn validates_tail_even_after_finding_frequency() {
    let mut s = structure(Some(&10_000_000u32.to_be_bytes()));
    let n = s.len();
    set(&mut s, n - 4, 0xdeadbeef);
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"timebase-frequency\0")),
        Err(TimebaseError::MalformedDtb)
    );
    let mut s = structure(Some(&10_000_000u32.to_be_bytes()));
    let n = s.len();
    s.drain(n - 8..n - 4);
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"timebase-frequency\0")),
        Err(TimebaseError::MalformedDtb)
    );
}
#[test]
fn rejects_duplicate_target_nodes_and_properties() {
    for duplicate_node in [false, true] {
        let mut s = Vec::new();
        begin(&mut s, b"");
        begin(&mut s, b"cpus");
        prop(&mut s, 0, &1u32.to_be_bytes());
        if duplicate_node {
            word(&mut s, 2);
            begin(&mut s, b"cpus");
        }
        prop(&mut s, 0, &2u32.to_be_bytes());
        word(&mut s, 2);
        word(&mut s, 2);
        word(&mut s, 9);
        assert_eq!(
            parse_timebase_frequency(&blob(&s, b"timebase-frequency\0")),
            Err(TimebaseError::MalformedDtb)
        );
    }
}
#[test]
fn only_exact_path_counts_and_unknown_properties_are_checked() {
    let mut s = Vec::new();
    begin(&mut s, b"");
    begin(&mut s, b"other");
    begin(&mut s, b"cpus");
    prop(&mut s, 0, &1u32.to_be_bytes());
    word(&mut s, 2);
    word(&mut s, 2);
    word(&mut s, 2);
    word(&mut s, 9);
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"timebase-frequency\0")),
        Err(TimebaseError::MissingFrequency)
    );
    let mut s = structure(Some(&1u32.to_be_bytes()));
    set(&mut s, 28, u32::MAX); // 属性 nameoff
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"timebase-frequency\0")),
        Err(TimebaseError::MalformedDtb)
    );
}
#[test]
fn rejects_unterminated_names_and_reservation_table() {
    let s = structure(Some(&1u32.to_be_bytes()));
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"timebase-frequency")),
        Err(TimebaseError::MalformedDtb)
    );
    let mut b = valid();
    b[40..56].fill(1);
    assert_eq!(
        parse_timebase_frequency(&b),
        Err(TimebaseError::MalformedDtb)
    );
    let mut s = Vec::new();
    word(&mut s, 1);
    s.extend([b'a'; 8]);
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"")),
        Err(TimebaseError::MalformedDtb)
    );
}
#[test]
fn rejects_wrong_root_and_invalid_token_order() {
    let cases = [vec![2, 9], vec![3, 0, 0, 9], vec![9]];
    for tokens in cases {
        let mut s = Vec::new();
        for t in tokens {
            word(&mut s, t);
        }
        assert_eq!(
            parse_timebase_frequency(&blob(&s, b"")),
            Err(TimebaseError::MalformedDtb)
        );
    }
    let mut s = Vec::new();
    begin(&mut s, b"not-root");
    word(&mut s, 2);
    word(&mut s, 9);
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"")),
        Err(TimebaseError::MalformedDtb)
    );
}

#[test]
fn physical_range_rejects_overflow_and_ram_boundary_before_access() {
    assert_eq!(checked_boot_range(0x80210000, 0x87e00000, 40), Ok(()));
    assert_eq!(checked_boot_range(0x80210000, 0x87e00000, 0x200000), Ok(()));
    for (start, len) in [
        (0, 40),
        (0x87e00001, 40),
        (0x88000000, 40),
        (0x87fffff8, 40),
        (0x87e00000, 39),
        (0x87e00000, 0x200001),
        (0x87e00000, usize::MAX),
        (usize::MAX - 7, 40),
        (0x80200000, 40),
    ] {
        assert_eq!(
            checked_boot_range(0x80210000, start, len),
            Err(TimebaseError::InvalidRange)
        );
    }
}

#[test]
fn ignores_nops_and_accepts_exactly_adjacent_blocks() {
    let mut s = structure(Some(&123u32.to_be_bytes()));
    s.splice(0..0, 4u32.to_be_bytes());
    let b = blob(&s, b"timebase-frequency\0");
    assert_eq!(parse_timebase_frequency(&b).unwrap().get(), 123);
}

#[test]
fn rejects_properties_after_children_and_second_root() {
    let mut s = Vec::new();
    begin(&mut s, b"");
    begin(&mut s, b"cpus");
    begin(&mut s, b"cpu@0");
    word(&mut s, 2);
    prop(&mut s, 0, &1u32.to_be_bytes());
    word(&mut s, 2);
    word(&mut s, 2);
    word(&mut s, 9);
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"timebase-frequency\0")),
        Err(TimebaseError::MalformedDtb)
    );
    let mut s = Vec::new();
    begin(&mut s, b"");
    word(&mut s, 2);
    begin(&mut s, b"");
    word(&mut s, 2);
    word(&mut s, 9);
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"")),
        Err(TimebaseError::MalformedDtb)
    );
}

#[test]
fn ignores_nested_frequency_but_validates_its_payload_bounds() {
    let mut s = Vec::new();
    begin(&mut s, b"");
    begin(&mut s, b"cpus");
    prop(&mut s, 0, &123u32.to_be_bytes());
    begin(&mut s, b"cpu@0");
    prop(&mut s, 0, &456u32.to_be_bytes());
    word(&mut s, 2);
    word(&mut s, 2);
    word(&mut s, 2);
    word(&mut s, 9);
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"timebase-frequency\0"))
            .unwrap()
            .get(),
        123
    );
    let offset = s
        .windows(4)
        .position(|v| v == 456u32.to_be_bytes())
        .unwrap();
    set(&mut s, offset - 8, u32::MAX);
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"timebase-frequency\0")),
        Err(TimebaseError::MalformedDtb)
    );
}

#[test]
fn firmware_alignment_padding_is_not_semantic_data() {
    let mut s = Vec::new();
    begin(&mut s, b"");
    begin(&mut s, b"cpus");
    prop(&mut s, 0, &123u32.to_be_bytes());
    prop(&mut s, 19, b"x");
    let n = s.len();
    s[n - 3..].fill(0xa5);
    word(&mut s, 2);
    word(&mut s, 2);
    word(&mut s, 9);
    assert_eq!(
        parse_timebase_frequency(&blob(&s, b"timebase-frequency\0other\0"))
            .unwrap()
            .get(),
        123
    );
}

/// 重建每个截断输入的头部，使其通过总长/块范围检查，真正进入内部解析。
#[test]
fn truncated_structure_and_strings_fail_inside_valid_outer_bounds() {
    let s = structure(Some(&123u32.to_be_bytes()));
    let names = b"timebase-frequency\0";
    for end in 0..s.len() {
        assert_eq!(
            parse_timebase_frequency(&blob(&s[..end], names)),
            Err(MalformedDtb),
            "structure truncated at {end}"
        );
    }
    for end in 0..names.len() {
        assert_eq!(
            parse_timebase_frequency(&blob(&s, &names[..end])),
            Err(MalformedDtb),
            "strings truncated at {end}"
        );
    }
}

/// 数据还在输入中，但位于声明的结构/字符串边界外时仍不能被消费。
#[test]
fn ignores_bytes_outside_declared_blocks_and_total() {
    let original = valid();
    for (field, value) in [
        (4, (original.len() - 1) as u32),
        (32, (b"timebase-frequency\0".len() - 1) as u32),
        (36, (structure(Some(&1u32.to_be_bytes())).len() - 4) as u32),
    ] {
        let mut b = original.clone();
        set(&mut b, field, value);
        assert_eq!(parse_timebase_frequency(&b), Err(MalformedDtb));
    }
    let mut b = original;
    b.extend([0xff; 64]);
    assert_eq!(parse_timebase_frequency(&b).unwrap().get(), 10_000_000);
}

/// 非空保留表、间隙和不按 4 字节对齐的字符串块均不应误拒绝。
#[test]
fn accepts_reserved_entries_gaps_and_unaligned_strings() {
    let mut b = valid();
    b.splice(
        40..40,
        [0x80000000u64.to_be_bytes(), 0x1000u64.to_be_bytes()].concat(),
    );
    let struct_start = 72;
    let strings_start = struct_start + structure(Some(&1u32.to_be_bytes())).len();
    set(&mut b, 8, struct_start as u32);
    set(&mut b, 12, strings_start as u32 + 1);
    b.insert(strings_start, 0xff);
    let total = b.len() as u32;
    set(&mut b, 4, total);
    assert_eq!(parse_timebase_frequency(&b).unwrap().get(), 10_000_000);
}

#[test]
fn boot_reader_checks_header_range_before_any_read() {
    for address in [
        0,
        0x80200000,
        0x87e00001,
        0x87fffff8,
        0x88000000,
        usize::MAX - 7,
    ] {
        let result = read_boot_frequency(0x80210000, address, |_, _| {
            panic!("invalid address must not trigger a read")
        });
        assert_eq!(result, Err(TimebaseError::InvalidRange));
    }
}

#[test]
fn boot_reader_rejects_bad_total_after_only_the_header_read() {
    for total in [0, 39, 0x200001, u32::MAX] {
        let mut b = valid();
        set(&mut b, 4, total);
        let mut calls = Vec::new();
        let result = read_boot_frequency(0x80210000, 0x87e00000, |address, len| {
            calls.push((address, len));
            assert_eq!(len, HEADER_LEN, "invalid total must not cause a body read");
            &b[..len]
        });
        assert_eq!(calls, [(0x87e00000, HEADER_LEN)]);
        assert_eq!(
            result,
            Err(if total < 40 {
                MalformedDtb
            } else {
                TimebaseError::InvalidRange
            })
        );
    }
    let mut b = valid();
    set(&mut b, 0, 0);
    let mut reads = 0;
    assert_eq!(
        read_boot_frequency(0x80210000, 0x87e00000, |_, len| {
            reads += 1;
            assert_eq!(len, HEADER_LEN);
            &b[..len]
        }),
        Err(MalformedDtb)
    );
    assert_eq!(reads, 1);
}

#[test]
fn boot_reader_uses_two_bounded_reads_and_does_not_write() {
    let b = valid();
    let original = b.clone();
    let mut calls = Vec::new();
    let result = read_boot_frequency(0x80210000, 0x87e00000, |address, len| {
        calls.push((address, len));
        &b[..len]
    });
    assert_eq!(result.unwrap().get(), 10_000_000);
    assert_eq!(calls, [(0x87e00000, HEADER_LEN), (0x87e00000, b.len())]);
    assert_eq!(b, original);
}
