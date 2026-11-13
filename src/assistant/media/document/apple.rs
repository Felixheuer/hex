//! What macOS does for documents: PDFKit reads a PDF's text and draws its
//! pages, Vision reads text in pictures (`src/documents.swift`), Core Text
//! lays out a PDF, textutil reads and writes RTF, DOC, ODT, and HTML, and
//! Pages, Numbers, and Keynote export their files (AppleScript, only when
//! the app is installed). Elsewhere these say what is missing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{Value, json};

use super::read::{Ask, Page, Read};
use crate::assistant::media::out;

/// How long a scanned PDF may be read and a document converted.
const PDF_SECONDS: f64 = 90.0;
const CONVERT_LIMIT: Duration = Duration::from_secs(60);
const IWORK_LIMIT: Duration = Duration::from_secs(120);

#[cfg(target_os = "macos")]
fn bridge(request: &Value) -> Result<Value, String> {
    use std::ffi::{CStr, CString, c_char};
    unsafe extern "C" {
        fn hex_documents_run(request: *const c_char) -> *mut c_char;
        fn hex_documents_free(string: *mut c_char);
    }
    let request = CString::new(request.to_string()).map_err(|error| error.to_string())?;
    // SAFETY: The request is a valid C string for the call; the reply is a
    // C string the bridge allocated, copied here and then freed by it.
    let reply = unsafe {
        let pointer = hex_documents_run(request.as_ptr());
        if pointer.is_null() {
            return Err("the documents bridge answered nothing".into());
        }
        let text = CStr::from_ptr(pointer).to_string_lossy().into_owned();
        hex_documents_free(pointer);
        text
    };
    let value: Value = serde_json::from_str(&reply).map_err(|error| error.to_string())?;
    match value["error"].as_str() {
        Some(error) => Err(error.to_owned()),
        None => Ok(value),
    }
}

#[cfg(not(target_os = "macos"))]
fn bridge(_: &Value) -> Result<Value, String> {
    Err("reading and writing PDFs and OCR need macOS".into())
}

/// A PDF's pages (those `ask` names): text, OCR where a page has none, and
/// page images into `images` when asked.
pub fn pdf(path: &Path, ask: &Ask, images: &Path) -> Result<Read, String> {
    let (first, last) = ask.bridge_span();
    let image_pages = ask.image_pages();
    if !image_pages.is_empty() {
        std::fs::create_dir_all(images).map_err(|error| error.to_string())?;
    }
    let reply = bridge(&json!({
        "op": "pdf_read",
        "path": path,
        "first": first,
        "last": last.min(i32::MAX as usize),
        "ocr": ask.ocr,
        "images_dir": images,
        "image_pages": image_pages,
        "image_width": 1200,
        "max_chars": ask.max_chars * 2,
        "seconds": PDF_SECONDS,
    }))
    .map_err(|error| format!("Couldn't read the PDF: {error}"))?;
    let pages = reply["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|page| ask_wants(ask, page["n"].as_u64().unwrap_or(0) as usize))
        .map(|page| Page {
            n: page["n"].as_u64().unwrap_or(0) as usize,
            text: page["text"].as_str().unwrap_or_default().trim().to_owned(),
            ocr: page["ocr"].as_bool().unwrap_or(false),
        })
        .collect();
    Ok(Read {
        format: "pdf",
        pages,
        images: reply["images"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .collect(),
        meta: json!({"page_count": reply["page_count"], "title": reply["title"], "author": reply["author"]}),
        cut: reply["cut"].as_bool().unwrap_or(false),
        ..Read::default()
    })
}

fn ask_wants(ask: &Ask, page: usize) -> bool {
    ask.pages.as_ref().is_none_or(|ranges| {
        ranges
            .iter()
            .any(|(first, last)| (*first..=*last).contains(&page))
    })
}

/// The text Vision reads in the picture at `path`, line by line with
/// boxes (fractions of the picture from its top left).
pub fn ocr(path: &Path) -> Result<Value, String> {
    bridge(&json!({"op": "ocr", "path": path}))
        .map_err(|error| format!("Couldn't read text in the picture: {error}"))
}

/// A PDF laid out from `blocks` (`blocks::to_json`) at `out`; its page count.
pub fn write_pdf(
    out: &Path,
    title: &str,
    blocks: &Value,
    page: &str,
    landscape: bool,
) -> Result<u64, String> {
    let reply = bridge(&json!({
        "op": "pdf_write", "out": out, "title": title, "blocks": blocks,
        "page": page, "landscape": landscape,
    }))
    .map_err(|error| format!("Couldn't write the PDF: {error}"))?;
    Ok(reply["pages"].as_u64().unwrap_or(0))
}

/// The document's text through textutil (RTF, DOC, ODT, HTML, …).
pub fn textutil_text(path: &Path) -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("textutil needs macOS".into());
    }
    let mut command = Command::new("/usr/bin/textutil");
    command
        .args(["-convert", "txt", "-stdout", "-encoding", "UTF-8"])
        .arg(path);
    Ok(out::run(command, CONVERT_LIMIT, "textutil")?.stdout)
}

/// `html` converted by textutil to `out` (its extension: rtf, doc, odt,
/// docx).
pub fn textutil_convert(html: &Path, out: &Path) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("textutil needs macOS".into());
    }
    let format = out::extension(out);
    let mut command = Command::new("/usr/bin/textutil");
    command
        .args(["-convert", &format, "-output"])
        .arg(out)
        .arg(html);
    out::run(command, CONVERT_LIMIT, "textutil").map(|_| ())
}

/// The iWork app for a file extension, and the Office format it exports to
/// (AppleScript's name, file extension).
fn iwork(extension: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match extension {
        "pages" => Some(("Pages", "Microsoft Word", "docx")),
        "numbers" => Some(("Numbers", "Microsoft Excel", "xlsx")),
        "key" => Some(("Keynote", "Microsoft PowerPoint", "pptx")),
        _ => None,
    }
}

fn installed(app: &str) -> bool {
    cfg!(target_os = "macos")
        && [
            format!("/Applications/{app}.app"),
            format!("/System/Applications/{app}.app"),
        ]
        .iter()
        .any(|path| Path::new(path).exists())
}

/// AppleScript's string literal of `text`.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The iWork file at `path` exported by its app to the Office format, into
/// `folder`.
pub fn iwork_export(path: &Path, folder: &Path) -> Result<PathBuf, String> {
    let extension = out::extension(path);
    let (app, format, office) = iwork(&extension).ok_or("not an iWork file")?;
    if !installed(app) {
        return Err(format!(
            "{app} isn't installed, so its .{extension} file can't be exported."
        ));
    }
    let stem = path.file_stem().map_or_else(
        || "document".into(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let target = folder.join(out::fresh_name(folder, &stem, office));
    let script = format!(
        "tell application {app}\n\
         set opened to open POSIX file {source}\n\
         export opened to POSIX file {target} as {format}\n\
         close opened saving no\n\
         end tell",
        app = quoted(app),
        source = quoted(&path.to_string_lossy()),
        target = quoted(&target.to_string_lossy()),
    );
    let mut command = Command::new("/usr/bin/osascript");
    command.args(["-e", &script]);
    out::run(command, IWORK_LIMIT, &format!("{app}'s export"))?;
    if target.is_file() {
        Ok(target)
    } else {
        Err(format!("{app} exported nothing"))
    }
}

/// The Office file at `office` opened in its iWork app and saved as the
/// iWork file `target`.
pub fn iwork_import(office: &Path, target: &Path) -> Result<(), String> {
    let extension = out::extension(target);
    let (app, _, _) = iwork(&extension).ok_or("not an iWork file")?;
    if !installed(app) {
        return Err(format!(
            "{app} isn't installed; write .{} instead",
            out::extension(office)
        ));
    }
    let script = format!(
        "tell application {app}\n\
         set opened to open POSIX file {source}\n\
         save opened in POSIX file {target}\n\
         close opened saving no\n\
         end tell",
        app = quoted(app),
        source = quoted(&office.to_string_lossy()),
        target = quoted(&target.to_string_lossy()),
    );
    let mut command = Command::new("/usr/bin/osascript");
    command.args(["-e", &script]);
    out::run(command, IWORK_LIMIT, &format!("{app}'s import")).map(|_| ())
}

/// The Office format an iWork extension is written through.
pub fn iwork_office(extension: &str) -> Option<&'static str> {
    iwork(extension).map(|(_, _, office)| office)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iwork_files_go_through_their_office_twin() {
        assert_eq!(iwork_office("key"), Some("pptx"));
        assert_eq!(iwork_office("numbers"), Some("xlsx"));
        assert_eq!(iwork_office("docx"), None);
        assert_eq!(quoted("a \"b\" \\c"), "\"a \\\"b\\\" \\\\c\"");
    }
}
