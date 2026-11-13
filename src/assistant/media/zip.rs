//! Just enough ZIP for Office files (DOCX, XLSX, PPTX are zipped XML):
//! reading an archive's entries (stored or deflated) through its central
//! directory, and writing one with deflated entries. No ZIP64, encryption,
//! or multi-disk archives; Office files never need them.

use std::io::{Read, Write};

/// The largest entry read, uncompressed: a document's XML is far smaller.
const MAX_ENTRY: u64 = 256 * 1024 * 1024;

/// An archive read into memory.
pub struct Archive {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
}

struct Entry {
    name: String,
    method: u16,
    compressed: usize,
    size: usize,
    offset: usize,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

impl Archive {
    /// The archive in `bytes`, if it is one.
    pub fn new(bytes: Vec<u8>) -> Result<Self, String> {
        let not_zip = || "not a ZIP-based document".to_owned();
        // The end of central directory record: within the last 64 KiB + 22.
        let from = bytes.len().saturating_sub(65_557);
        let end = (from..bytes.len().saturating_sub(21))
            .rev()
            .find(|&at| bytes[at..at + 4] == [0x50, 0x4b, 0x05, 0x06])
            .ok_or_else(not_zip)?;
        let count = u16_at(&bytes, end + 10).ok_or_else(not_zip)? as usize;
        let mut at = u32_at(&bytes, end + 16).ok_or_else(not_zip)? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32_at(&bytes, at) != Some(0x0201_4b50) {
                return Err(not_zip());
            }
            let field = |offset| u32_at(&bytes, at + offset).ok_or_else(not_zip);
            let method = u16_at(&bytes, at + 10).ok_or_else(not_zip)?;
            let compressed = field(20)? as usize;
            let size = field(24)? as usize;
            let name_len = u16_at(&bytes, at + 28).ok_or_else(not_zip)? as usize;
            let extra_len = u16_at(&bytes, at + 30).ok_or_else(not_zip)? as usize;
            let comment_len = u16_at(&bytes, at + 32).ok_or_else(not_zip)? as usize;
            let offset = field(42)? as usize;
            let name = bytes
                .get(at + 46..at + 46 + name_len)
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .ok_or_else(not_zip)?;
            entries.push(Entry {
                name,
                method,
                compressed,
                size,
                offset,
            });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { bytes, entries })
    }

    /// Every entry's name, in the archive's order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.name.as_str())
    }

    /// The entry `name`'s bytes, if it is there and readable.
    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let entry = self.entries.iter().find(|entry| entry.name == name)?;
        if entry.size as u64 > MAX_ENTRY {
            return None;
        }
        let header = entry.offset;
        if u32_at(&self.bytes, header)? != 0x0403_4b50 {
            return None;
        }
        let name_len = u16_at(&self.bytes, header + 26)? as usize;
        let extra_len = u16_at(&self.bytes, header + 28)? as usize;
        let start = header + 30 + name_len + extra_len;
        let data = self.bytes.get(start..start + entry.compressed)?;
        match entry.method {
            0 => Some(data.to_vec()),
            8 => {
                let mut out = Vec::with_capacity(entry.size);
                flate2::read::DeflateDecoder::new(data)
                    .take(MAX_ENTRY)
                    .read_to_end(&mut out)
                    .ok()?;
                Some(out)
            }
            _ => None,
        }
    }

    /// The entry `name` as text.
    pub fn text(&self, name: &str) -> Option<String> {
        self.read(name)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// An archive being written, in memory.
#[derive(Default)]
pub struct Writer {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl Writer {
    /// Adds `name` with `data`, deflated.
    pub fn add(&mut self, name: &str, data: &[u8]) {
        let crc = crc32fast::hash(data);
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        let compressed = encoder
            .write_all(data)
            .and_then(|()| encoder.finish())
            .unwrap_or_default();
        let offset = self.out.len() as u32;
        let name = name.as_bytes();
        // Local header: version 2.0, no flags, deflate, a fixed 1980 date.
        self.out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        self.out.extend_from_slice(&20_u16.to_le_bytes());
        self.out.extend_from_slice(&0_u16.to_le_bytes());
        self.out.extend_from_slice(&8_u16.to_le_bytes());
        self.out.extend_from_slice(&0_u16.to_le_bytes());
        self.out.extend_from_slice(&0x21_u16.to_le_bytes());
        self.out.extend_from_slice(&crc.to_le_bytes());
        self.out
            .extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.out.extend_from_slice(&0_u16.to_le_bytes());
        self.out.extend_from_slice(name);
        self.out.extend_from_slice(&compressed);
        // Its central directory record.
        let central = &mut self.central;
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&8_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0x21_u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
        self.count += 1;
    }

    /// The finished archive.
    pub fn finish(mut self) -> Vec<u8> {
        let start = self.out.len() as u32;
        let size = self.central.len() as u32;
        self.out.extend_from_slice(&self.central);
        self.out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]);
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&size.to_le_bytes());
        self.out.extend_from_slice(&start.to_le_bytes());
        self.out.extend_from_slice(&0_u16.to_le_bytes());
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_reads_back() {
        let mut writer = Writer::default();
        writer.add("a.xml", b"<a>hello</a>");
        let long = "Zeile ".repeat(2000);
        writer.add("dir/b.txt", long.as_bytes());
        writer.add("empty", b"");
        let archive = Archive::new(writer.finish()).unwrap();
        assert_eq!(
            archive.names().collect::<Vec<_>>(),
            ["a.xml", "dir/b.txt", "empty"]
        );
        assert_eq!(archive.text("a.xml").unwrap(), "<a>hello</a>");
        assert_eq!(archive.text("dir/b.txt").unwrap(), long);
        assert_eq!(archive.read("empty").unwrap(), b"");
        assert!(archive.read("missing").is_none());
        assert!(Archive::new(b"no zip here".to_vec()).is_err());
    }
}
