//! Accepted file types, magic bytes, PDF page counts, and token estimates.
//!
//! The media type comes from the extension plus a magic-byte check, never from the client's
//! `Content-Type` alone (`docs/design/copilot.md`, Security, Files).

use std::fmt;
use std::str::FromStr;

use crate::shared::constants::{BYTES_PER_TOKEN, TOKENS_PER_IMAGE, TOKENS_PER_PDF_PAGE};
use crate::shared::errors::{DomainError, DomainResult};

/// The type served for anything not on the allowlist.
pub const OCTET_STREAM: &str = "application/octet-stream";

/// A type on the upload allowlist.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MediaType {
    Png,
    Jpeg,
    Webp,
    Gif,
    Pdf,
    Wav,
    Mpeg,
    Text,
    Markdown,
}

/// How a file reaches a model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    Image,
    Pdf,
    Audio,
    Text,
}

impl MediaType {
    pub const ALL: [Self; 9] = [
        Self::Png,
        Self::Jpeg,
        Self::Webp,
        Self::Gif,
        Self::Pdf,
        Self::Wav,
        Self::Mpeg,
        Self::Text,
        Self::Markdown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
            Self::Pdf => "application/pdf",
            Self::Wav => "audio/wav",
            Self::Mpeg => "audio/mpeg",
            Self::Text => "text/plain",
            Self::Markdown => "text/markdown",
        }
    }

    pub fn kind(self) -> FileKind {
        match self {
            Self::Png | Self::Jpeg | Self::Webp | Self::Gif => FileKind::Image,
            Self::Pdf => FileKind::Pdf,
            Self::Wav | Self::Mpeg => FileKind::Audio,
            Self::Text | Self::Markdown => FileKind::Text,
        }
    }

    /// The accepted type of an upload, from its name's extension confirmed by its content.
    /// Anything else fails with "This file type is not supported."
    pub fn detect(name: &str, bytes: &[u8]) -> DomainResult<Self> {
        let extension = extension(name).ok_or_else(unsupported)?;
        let (media, matches) = match extension.as_str() {
            "png" => (Self::Png, bytes.starts_with(b"\x89PNG\r\n\x1a\n")),
            "jpg" | "jpeg" => (Self::Jpeg, bytes.starts_with(&[0xff, 0xd8, 0xff])),
            "webp" => (Self::Webp, riff(bytes, b"WEBP")),
            "gif" => (
                Self::Gif,
                bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
            ),
            "pdf" => (Self::Pdf, bytes.starts_with(b"%PDF-")),
            "wav" => (Self::Wav, riff(bytes, b"WAVE")),
            "mp3" => (Self::Mpeg, mp3(bytes)),
            "md" | "markdown" => (Self::Markdown, utf8(bytes)),
            other if TEXT_EXTENSIONS.contains(&other) => (Self::Text, utf8(bytes)),
            _ => return Err(unsupported()),
        };
        if matches {
            Ok(media)
        } else {
            Err(unsupported())
        }
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for MediaType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|media| media.as_str() == value)
            .ok_or_else(|| value.to_owned())
    }
}

/// The `Content-Type` to serve a stored file with: the stored type when it is on the allowlist,
/// otherwise `application/octet-stream`. SVG and HTML are never served as their own types.
pub fn served_type(stored: &str) -> &'static str {
    stored
        .parse::<MediaType>()
        .map_or(OCTET_STREAM, MediaType::as_str)
}

/// Extensions of plain-text and code files (stored as `text/plain`). Markdown is separate.
/// SVG is not text here: it is an image format that can carry scripts.
const TEXT_EXTENSIONS: &[&str] = &[
    "adoc",
    "bash",
    "bat",
    "bib",
    "c",
    "cc",
    "cfg",
    "cjs",
    "clj",
    "cmake",
    "conf",
    "cpp",
    "cs",
    "css",
    "csv",
    "cxx",
    "dart",
    "diff",
    "env",
    "erl",
    "ex",
    "exs",
    "fish",
    "go",
    "gql",
    "gradle",
    "graphql",
    "h",
    "hcl",
    "hh",
    "hpp",
    "hs",
    "htm",
    "html",
    "ini",
    "java",
    "jl",
    "js",
    "json",
    "jsonl",
    "jsx",
    "kt",
    "kts",
    "less",
    "log",
    "lua",
    "m",
    "mjs",
    "mk",
    "ml",
    "mm",
    "ndjson",
    "org",
    "patch",
    "php",
    "pl",
    "pm",
    "properties",
    "proto",
    "ps1",
    "py",
    "r",
    "rb",
    "rs",
    "rst",
    "sass",
    "scala",
    "scss",
    "sh",
    "sql",
    "srt",
    "svelte",
    "swift",
    "tex",
    "text",
    "tf",
    "toml",
    "ts",
    "tsv",
    "tsx",
    "txt",
    "vtt",
    "vue",
    "xml",
    "yaml",
    "yml",
    "zsh",
];

/// The lowercase extension after the last dot, if any.
fn extension(name: &str) -> Option<String> {
    let (_, extension) = name.rsplit_once('.')?;
    (!extension.is_empty()).then(|| extension.to_ascii_lowercase())
}

fn unsupported() -> DomainError {
    DomainError::validation("This file type is not supported.").with_field("file")
}

/// A RIFF container of the given form type (`WEBP`, `WAVE`).
fn riff(bytes: &[u8], form: &[u8; 4]) -> bool {
    bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == form
}

/// An ID3 tag, or an MPEG audio Layer III frame header.
fn mp3(bytes: &[u8]) -> bool {
    if bytes.starts_with(b"ID3") {
        return true;
    }
    match bytes {
        [0xff, second, ..] => (second & 0xe0) == 0xe0 && ((second >> 1) & 0x03) == 0x01,
        _ => false,
    }
}

fn utf8(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok()
}

/// Inflated object stream bytes read per PDF at most, so a compression bomb costs little.
const PDF_INFLATE_MAX_BYTES: usize = 32 * 1024 * 1024;
/// The size of a page when no page object can be read (an encrypted or unusual PDF), so such a
/// file never counts as one page whatever its length.
const PDF_BYTES_PER_PAGE: usize = 50 * 1024;

/// Pages of a PDF, counted heuristically: page objects (`/Type /Page`, not `/Pages`), both in the
/// file and inside its Flate-compressed object streams (`/Type /ObjStm`, PDF 1.5 and later,
/// where most LaTeX and modern producers put every page dictionary). When no page object can be
/// read at all, one page per 50 KiB. At least 1.
pub fn pdf_pages(bytes: &[u8]) -> u32 {
    let mut pages = page_objects(bytes);
    let mut budget = PDF_INFLATE_MAX_BYTES;
    for stream in object_streams(bytes) {
        if budget == 0 {
            break;
        }
        let Ok(inflated) = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(stream, budget)
        else {
            continue;
        };
        budget = budget.saturating_sub(inflated.len());
        pages = pages.saturating_add(page_objects(&inflated));
    }
    if pages == 0 {
        pages = u32::try_from(bytes.len() / PDF_BYTES_PER_PAGE).unwrap_or(u32::MAX);
    }
    pages.max(1)
}

/// Object streams examined per PDF at most. A real PDF has a few dozen; the cap bounds the work
/// a crafted file of nothing but `/ObjStm` marks can cause.
const PDF_OBJECT_STREAMS_MAX: usize = 4_096;

/// The raw data of every Flate-compressed object stream: the bytes between `stream` and
/// `endstream` of an object whose dictionary (from its `obj` keyword to `stream`) holds
/// `/ObjStm` and `/FlateDecode`. One forward pass: each byte is searched a bounded number of
/// times, so the cost is linear in the file's length whatever it holds.
fn object_streams(bytes: &[u8]) -> Vec<&[u8]> {
    const MARK: &[u8] = b"/ObjStm";
    const OBJ: &[u8] = b"obj";
    const STREAM: &[u8] = b"stream";
    const END: &[u8] = b"endstream";
    let mut streams = Vec::new();
    // Where the current object's dictionary starts: its last `obj` keyword, never before the
    // end of what an earlier step already read.
    let mut object = 0;
    // Bytes before `scanned` were already searched for `obj`.
    let mut scanned = 0;
    let mut index = 0;
    for _ in 0..PDF_OBJECT_STREAMS_MAX {
        let Some(offset) = find(&bytes[index..], MARK) else {
            break;
        };
        let mark = index + offset;
        if let Some(found) = rfind(&bytes[scanned..mark], OBJ) {
            object = scanned + found;
        }
        let Some(keyword) = find(&bytes[mark..], STREAM).map(|found| mark + found) else {
            break;
        };
        // Every mark before `stream` belongs to this dictionary: the next search starts after it.
        index = keyword + STREAM.len();
        scanned = index;
        let dictionary = &bytes[object..keyword];
        object = index;
        if find(dictionary, b"/FlateDecode").is_none() || find(dictionary, b"endobj").is_some() {
            continue;
        }
        let mut data = index;
        if bytes.get(data) == Some(&b'\r') {
            data += 1;
        }
        if bytes.get(data) == Some(&b'\n') {
            data += 1;
        }
        let Some(end) = find(&bytes[data..], END).map(|found| data + found) else {
            break;
        };
        streams.push(&bytes[data..end]);
        index = end + END.len();
        scanned = index;
        object = index;
    }
    streams
}

/// Page objects (`/Type /Page`, not `/Pages` or `/PageLabel`) in `bytes`.
fn page_objects(bytes: &[u8]) -> u32 {
    const TYPE: &[u8] = b"/Type";
    const PAGE: &[u8] = b"/Page";
    let mut pages: u32 = 0;
    let mut index = 0;
    while let Some(offset) = find(&bytes[index..], TYPE) {
        let mut cursor = index + offset + TYPE.len();
        while bytes.get(cursor).is_some_and(|byte| is_pdf_space(*byte)) {
            cursor += 1;
        }
        let rest = &bytes[cursor..];
        if rest.starts_with(PAGE)
            && !rest
                .get(PAGE.len())
                .is_some_and(|next| next.is_ascii_alphanumeric())
        {
            pages = pages.saturating_add(1);
        }
        index = cursor;
    }
    pages
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// PDF white-space characters.
fn is_pdf_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

/// The rough token cost of a file in a model's context: text bytes / 4, PDF pages x 3,000,
/// 1,600 per image. Audio has no estimate in v1 (it is a chat attachment for `audio_in` models
/// and never part of the project file preamble), so it counts 0. Capped at `i32::MAX`.
pub fn estimate(media: MediaType, size_bytes: u64, pages: Option<u32>) -> u32 {
    let tokens = match media.kind() {
        FileKind::Text => size_bytes.div_ceil(BYTES_PER_TOKEN),
        FileKind::Pdf => u64::from(pages.unwrap_or(1)) * u64::from(TOKENS_PER_PDF_PAGE),
        FileKind::Image => u64::from(TOKENS_PER_IMAGE),
        FileKind::Audio => 0,
    };
    u32::try_from(tokens.min(i32::MAX as u64)).unwrap_or(i32::MAX as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";
    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 0];
    const WEBP: &[u8] = b"RIFF\x10\0\0\0WEBPVP8 ";
    const WAV: &[u8] = b"RIFF\x10\0\0\0WAVEfmt ";
    const GIF: &[u8] = b"GIF89a\x01\0";
    const PDF: &[u8] = b"%PDF-1.7\n";

    #[test]
    fn detects_every_allowed_type() {
        let cases: [(&str, &[u8], MediaType); 12] = [
            ("photo.png", PNG, MediaType::Png),
            ("photo.JPG", JPEG, MediaType::Jpeg),
            ("photo.jpeg", JPEG, MediaType::Jpeg),
            ("sticker.webp", WEBP, MediaType::Webp),
            ("anim.gif", GIF, MediaType::Gif),
            ("paper.pdf", PDF, MediaType::Pdf),
            ("clip.wav", WAV, MediaType::Wav),
            ("song.mp3", b"ID3\x04\0", MediaType::Mpeg),
            ("frame.mp3", &[0xff, 0xfb, 0x90, 0x00], MediaType::Mpeg),
            ("notes.md", "# Hello\n".as_bytes(), MediaType::Markdown),
            ("main.rs", b"fn main() {}\n", MediaType::Text),
            ("data.csv", "a,b\n\u{e9},2\n".as_bytes(), MediaType::Text),
        ];
        for (name, bytes, expected) in cases {
            assert_eq!(MediaType::detect(name, bytes).unwrap(), expected, "{name}");
        }
    }

    #[test]
    fn rejects_content_that_does_not_match_the_extension() {
        let cases: [(&str, &[u8]); 9] = [
            ("photo.png", JPEG),
            ("photo.jpg", PNG),
            ("sticker.webp", WAV),
            ("clip.wav", WEBP),
            ("paper.pdf", b"<html>"),
            ("song.mp3", JPEG),
            ("notes.txt", &[0xff, 0xfe, 0x00]),
            ("empty.png", b""),
            ("anim.gif", b"GIF90a"),
        ];
        for (name, bytes) in cases {
            let error = MediaType::detect(name, bytes).unwrap_err();
            assert_eq!(
                error.to_string(),
                "This file type is not supported.",
                "{name}"
            );
        }
    }

    #[test]
    fn rejects_unknown_and_missing_extensions() {
        for name in [
            "logo.svg",
            "setup.exe",
            "archive.zip",
            "README",
            "trailing.",
            "doc.docx",
        ] {
            assert!(MediaType::detect(name, b"plain text").is_err(), "{name}");
        }
    }

    #[test]
    fn serves_only_allowlisted_types() {
        assert_eq!(served_type("image/png"), "image/png");
        assert_eq!(served_type("text/markdown"), "text/markdown");
        assert_eq!(served_type("image/svg+xml"), OCTET_STREAM);
        assert_eq!(served_type("text/html"), OCTET_STREAM);
        assert_eq!(served_type(""), OCTET_STREAM);
    }

    #[test]
    fn round_trips_media_types() {
        for media in MediaType::ALL {
            assert_eq!(media.as_str().parse::<MediaType>(), Ok(media));
        }
    }

    #[test]
    fn counts_pdf_page_objects() {
        let pdf = b"%PDF-1.7\n1 0 obj << /Type /Pages /Kids [2 0 R 3 0 R] /Count 2 >>\n\
                    2 0 obj << /Type /Page /Parent 1 0 R >>\n\
                    3 0 obj <</Type/Page/Parent 1 0 R>>\n\
                    4 0 obj << /Type\n/Page >>\n\
                    5 0 obj << /Type /PageLabel >>";
        assert_eq!(pdf_pages(pdf), 3);
        assert_eq!(pdf_pages(b"%PDF-1.7\n"), 1, "at least one page");
        assert_eq!(
            pdf_pages(b"%PDF /Type"),
            1,
            "a trailing /Type does not overrun"
        );
    }

    /// A PDF 1.5 whose page dictionaries all sit in one compressed object stream.
    fn compressed_pdf(pages: usize) -> Vec<u8> {
        let mut objects = String::from("<< /Type /Catalog /Pages 2 0 R >> ");
        objects.push_str(&format!("<< /Type /Pages /Count {pages} >> "));
        for _ in 0..pages {
            objects.push_str("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >> ");
        }
        let data = miniz_oxide::deflate::compress_to_vec_zlib(objects.as_bytes(), 6);
        let mut pdf = b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n5 0 obj\n<< /Length ".to_vec();
        pdf.extend_from_slice(data.len().to_string().as_bytes());
        pdf.extend_from_slice(b" /Filter /FlateDecode /Type /ObjStm /N 9 /First 40 >>\nstream\r\n");
        pdf.extend_from_slice(&data);
        pdf.extend_from_slice(b"\r\nendstream\nendobj\n");
        pdf.extend_from_slice(b"6 0 obj\n<< /Type /XRef /Filter /FlateDecode /Length 3 >>\nstream\nabc\nendstream\nendobj\n%%EOF\n");
        pdf
    }

    #[test]
    fn counts_pages_inside_compressed_object_streams() {
        let pdf = compressed_pdf(30);
        assert_eq!(page_objects(&pdf), 0, "nothing visible without inflating");
        assert_eq!(pdf_pages(&pdf), 30);
        assert_eq!(
            estimate(MediaType::Pdf, pdf.len() as u64, Some(pdf_pages(&pdf))),
            90_000
        );
    }

    #[test]
    fn an_unreadable_pdf_counts_one_page_per_50_kib() {
        let mut pdf =
            b"%PDF-1.7\n5 0 obj << /Type /ObjStm /Filter /FlateDecode >>\nstream\n".to_vec();
        pdf.extend(std::iter::repeat_n(0xa5, 500 * 1024));
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        assert_eq!(pdf_pages(&pdf), 10);
    }

    #[test]
    fn counts_every_object_stream_of_a_file() {
        let mut pdf = compressed_pdf(3);
        let second = compressed_pdf(4);
        pdf.extend_from_slice(&second[b"%PDF-1.5\n".len()..]);
        assert_eq!(pdf_pages(&pdf), 7);
    }

    /// Crafted files of nothing but object stream marks cost time linear in their length (the
    /// old backwards search took minutes for these sizes).
    #[test]
    fn crafted_object_stream_marks_take_linear_time() {
        let patterns: [&[u8]; 3] = [
            b"/ObjStm stream ",
            b"obj /FlateDecode /ObjStm stream x endstream ",
            b"/FlateDecode /ObjStm /ObjStm obj ",
        ];
        for pattern in patterns {
            let mut pdf = b"%PDF-1.5\n".to_vec();
            while pdf.len() < 2 * 1024 * 1024 {
                pdf.extend_from_slice(pattern);
            }
            let started = std::time::Instant::now();
            let pages = pdf_pages(&pdf);
            let took = started.elapsed();
            assert_eq!(
                pages as usize,
                pdf.len() / PDF_BYTES_PER_PAGE,
                "one page per 50 KiB"
            );
            assert!(
                took < std::time::Duration::from_secs(3),
                "{took:?} for {:?}",
                String::from_utf8_lossy(pattern)
            );
        }
    }

    #[test]
    fn estimates_tokens_by_kind() {
        assert_eq!(
            estimate(MediaType::Text, 10, None),
            3,
            "bytes / 4, rounded up"
        );
        assert_eq!(estimate(MediaType::Markdown, 4_000, None), 1_000);
        assert_eq!(estimate(MediaType::Pdf, 1_000_000, Some(12)), 36_000);
        assert_eq!(estimate(MediaType::Png, 9_000_000, None), 1_600);
        assert_eq!(estimate(MediaType::Wav, 1_000_000, None), 0);
        assert_eq!(estimate(MediaType::Pdf, 1, Some(u32::MAX)), i32::MAX as u32);
    }
}
