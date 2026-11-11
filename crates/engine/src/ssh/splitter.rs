//! Splits a byte stream into lines, for progress and logs.

/// Splits a byte stream into lines.
#[derive(Default)]
pub(super) struct Splitter {
    partial: Vec<u8>,
}

impl Splitter {
    pub(super) fn feed(&mut self, bytes: &[u8], mut emit: impl FnMut(&str)) {
        for &byte in bytes {
            if byte == b'\n' {
                emit(String::from_utf8_lossy(&self.partial).trim_end());
                self.partial.clear();
            } else {
                self.partial.push(byte);
            }
        }
    }

    pub(super) fn flush(&mut self, mut emit: impl FnMut(&str)) {
        if !self.partial.is_empty() {
            emit(String::from_utf8_lossy(&self.partial).trim_end());
            self.partial.clear();
        }
    }
}

/// Splits a byte stream into lines, keeping the whole text too.
#[derive(Default)]
pub(super) struct Lines {
    split: Splitter,
    /// Every byte, decoded once at the end: a packet can end inside a
    /// character.
    all: Vec<u8>,
}

impl Lines {
    pub(super) fn feed(&mut self, bytes: &[u8], emit: impl FnMut(&str)) {
        self.all.extend_from_slice(bytes);
        self.split.feed(bytes, emit);
    }

    pub(super) fn flush(&mut self, emit: impl FnMut(&str)) {
        self.split.flush(emit);
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8_lossy(&self.all).into_owned()
    }
}
