//! Bounded line reads of `.tally/ledger.jsonl` and `.tally/captures.jsonl` (#103, F-12).
//!
//! These files are untrusted input: a hostile or corrupted checkout can hold a single enormous
//! line. They are read through [`read_bounded`], which never holds more than [`MAX_LINE_BYTES`]
//! of any one line. A longer line is not buffered: its bytes are skipped to the next newline and
//! the line is replaced by [`OVERLONG_MARKER`], which no parser accepts, so the line is counted
//! as malformed under its own 1-based line number like any other unreadable line. Line numbers
//! of everything after it are unchanged and reading continues.

use std::io::{self, BufRead, BufReader, Read};

/// The longest line, in bytes and excluding its newline, that the ledger and captures readers
/// accept. The longest real line measured is 804 bytes; 1 MiB leaves a wide margin.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// The most bytes of one `.tally/` file that `ledger check` or [`TallyDir::read`] will take in
/// (#103). A larger file is refused with a typed error rather than buffered. Real ledgers are
/// kilobytes to a few megabytes; 256 MiB is far above any honest file.
pub const MAX_TALLY_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// What stands in for an over-long line in the bytes [`read_bounded`] returns. It is not valid
/// JSON, so every line parser counts it as malformed.
pub(super) const OVERLONG_MARKER: &[u8] = b"\x00";

/// The bytes of a file with over-long lines replaced by [`OVERLONG_MARKER`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Bounded {
    /// The file's bytes, one marker per over-long line.
    pub bytes: Vec<u8>,
    /// How many lines were over-long.
    pub overlong: u64,
}

/// Reads `reader` to the end holding at most `cap` bytes of any one line and taking in at most
/// `max_total` bytes in all.
///
/// # Errors
/// An error of kind [`io::ErrorKind::FileTooLarge`] as soon as more than `max_total` bytes have
/// been read; nothing past the buffer that crossed the limit is consumed. Other read errors pass
/// through.
pub(super) fn read_bounded<R: Read>(reader: R, cap: usize, max_total: u64) -> io::Result<Bounded> {
    let mut reader = BufReader::new(reader);
    let mut out = Bounded::default();
    // Length of the current line kept so far, whether it already exceeded the cap, and where it
    // starts in the output.
    let mut line_len = 0usize;
    let mut skipping = false;
    let mut line_start = 0usize;
    let mut total = 0u64;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        total = total.saturating_add(chunk.len() as u64);
        if total > max_total {
            return Err(io::Error::from(io::ErrorKind::FileTooLarge));
        }
        let used = chunk.len();
        let mut rest = chunk;
        while !rest.is_empty() {
            let newline = rest.iter().position(|b| *b == b'\n');
            let piece = &rest[..newline.unwrap_or(rest.len())];
            if !skipping {
                if line_len.saturating_add(piece.len()) > cap {
                    skipping = true;
                    out.bytes.truncate(line_start);
                    out.bytes.extend_from_slice(OVERLONG_MARKER);
                    out.overlong = out.overlong.saturating_add(1);
                } else {
                    out.bytes.extend_from_slice(piece);
                    line_len += piece.len();
                }
            }
            match newline {
                Some(at) => {
                    out.bytes.push(b'\n');
                    line_len = 0;
                    skipping = false;
                    line_start = out.bytes.len();
                    rest = &rest[at + 1..];
                }
                None => break,
            }
        }
        reader.consume(used);
    }
    Ok(out)
}

/// Reads the line file at `path` with [`MAX_LINE_BYTES`] and [`MAX_TALLY_FILE_BYTES`]. A missing
/// file is an `io::Error` of kind `NotFound`, as with `std::fs::read`; a file over the size limit
/// is one of kind `FileTooLarge`, refused on its length alone before any read.
pub(super) fn read_file(path: &std::path::Path) -> io::Result<Bounded> {
    let file = std::fs::File::open(path)?;
    read_open_file(file)
}

/// [`read_file`] for a file already open.
pub(super) fn read_open_file(file: std::fs::File) -> io::Result<Bounded> {
    if file.metadata()?.len() > MAX_TALLY_FILE_BYTES {
        return Err(io::Error::from(io::ErrorKind::FileTooLarge));
    }
    read_bounded(file, MAX_LINE_BYTES, MAX_TALLY_FILE_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(input: &[u8], cap: usize) -> Bounded {
        read_bounded(input, cap, u64::MAX).unwrap()
    }

    #[test]
    fn short_lines_pass_through_untouched() {
        let got = read(b"ab\ncd\r\n\nef", 8);
        assert_eq!(got.bytes, b"ab\ncd\r\n\nef");
        assert_eq!(got.overlong, 0);
    }

    #[test]
    fn a_line_of_exactly_the_cap_is_kept_and_one_more_is_not() {
        assert_eq!(read(b"abcd\nx\n", 4).overlong, 0);
        let got = read(b"abcde\nx\n", 4);
        assert_eq!(got.overlong, 1);
        assert_eq!(got.bytes, b"\x00\nx\n");
    }

    #[test]
    fn an_overlong_line_is_replaced_and_numbering_after_it_is_unchanged() {
        let got = read(b"ok\nTOOLONGTOOLONG\nok2\nTOOLONGTOOLONGTOOLONG", 4);
        assert_eq!(got.overlong, 2);
        assert_eq!(got.bytes, b"ok\n\x00\nok2\n\x00");
    }

    #[test]
    fn an_overlong_line_is_never_buffered() {
        // A reader that would hand out far more than the cap; the output stays tiny.
        let big = std::io::repeat(b'a').take(50_000_000);
        let got = read_bounded(big, 16, u64::MAX).unwrap();
        assert_eq!(got.bytes, OVERLONG_MARKER);
        assert_eq!(got.overlong, 1);
        assert!(got.bytes.capacity() < 1024);
    }

    #[test]
    fn a_line_spanning_many_buffer_fills_is_measured_whole() {
        let mut input = vec![b'a'; 100_000];
        input.push(b'\n');
        assert_eq!(read(&input, 100_000).overlong, 0);
        assert_eq!(read(&input, 99_999).overlong, 1);
    }

    #[test]
    fn many_short_lines_past_the_total_limit_are_refused() {
        let input = b"a\n".repeat(10);
        let err = read_bounded(&input[..], 8, 19).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::FileTooLarge);
        // Exactly at the limit is fine.
        assert_eq!(read_bounded(&input[..], 8, 20).unwrap().bytes, input);
    }

    #[test]
    fn the_total_limit_stops_the_read_without_draining_the_reader() {
        let mut source = std::io::repeat(b'a').take(50_000_000);
        let err = read_bounded(&mut source, 16, 1000).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::FileTooLarge);
        // Only a buffer or so past the limit was consumed, not the whole source.
        assert!(source.limit() > 49_000_000, "{}", source.limit());
    }

    #[test]
    fn read_file_refuses_a_file_over_the_production_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_TALLY_FILE_BYTES + 1).unwrap();
        drop(file);
        let err = read_file(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::FileTooLarge);
    }
}
