//! Copying text through the terminal (OSC 52), which works over SSH and in
//! terminals such as Windows Terminal, kitty, WezTerm and iTerm2. Terminals
//! without it ignore the request, so callers also show the text.

use std::io::Write;

pub fn copy(text: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()))?;
    out.flush()
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = match chunk {
            [a, b, c] => u32::from(*a) << 16 | u32::from(*b) << 8 | u32::from(*c),
            [a, b] => u32::from(*a) << 16 | u32::from(*b) << 8,
            [a] => u32::from(*a) << 16,
            _ => unreachable!("chunks(3) yields 1 to 3 bytes"),
        };
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn encodes_like_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"curl | bash"), "Y3VybCB8IGJhc2g=");
    }
}
