use super::logging::strip_ansi_codes;

#[test]
fn strips_sgr_sequences() {
    let input = "\u{1b}[38;5;87mhello\u{1b}[0m";
    assert_eq!(strip_ansi_codes(input.as_bytes()), "hello");
}

#[test]
fn strips_osc_sequences() {
    let input = "\u{1b}]0;title\u{7}payload";
    assert_eq!(strip_ansi_codes(input.as_bytes()), "payload");
}

#[test]
fn strips_csi_sequences() {
    let input = "\u{1b}[31mwarn\u{1b}[0m";
    assert_eq!(strip_ansi_codes(input.as_bytes()), "warn");
}

#[test]
fn strips_single_byte_csi_sequences() {
    let input = [
        0x9b, b'3', b'1', b'm', b'w', b'a', b'r', b'n', 0x9b, b'0', b'm',
    ];
    assert_eq!(strip_ansi_codes(&input), "warn");
}
