//! Bytes the PTY gets for pasted text.

/// `text` as the PTY input of a paste. Bracketed paste wraps it in `ESC[200~ … ESC[201~`
/// with ESC and ^C removed (so the text can't end the bracket early); otherwise line
/// breaks (`\r\n`, `\n`) become `\r`, what Enter sends. Empty text writes nothing.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if text.is_empty() {
        return Vec::new();
    }
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend(text.bytes().filter(|&b| b != 0x1b && b != 0x03));
        out.extend_from_slice(b"\x1b[201~");
        return out;
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_bracketed_strips_escapes() {
        assert_eq!(
            paste_bytes("a\x1b[201~b\x03", true),
            b"\x1b[200~a[201~b\x1b[201~"
        );
    }

    #[test]
    fn paste_plain_converts_newlines() {
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc");
    }

    #[test]
    fn paste_empty_writes_nothing() {
        assert!(paste_bytes("", true).is_empty());
        assert!(paste_bytes("", false).is_empty());
    }
}
