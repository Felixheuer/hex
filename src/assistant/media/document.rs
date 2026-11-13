//! `hex_document_read` and `hex_document_write`: documents in and out,
//! without opening an app for them (only Pages, Numbers, and Keynote files
//! go through their own app). Results land in the task's folder.

mod apple;
pub mod blocks;
mod office;
pub mod read;
mod slides;
pub mod write;

use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};

pub use apple::ocr;

pub const READ: &str = "hex_document_read";
pub const WRITE: &str = "hex_document_write";

pub fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": READ,
            "description": "Read a document's content: PDF (scanned ones too, through OCR), Word (.docx), Excel (.xlsx), PowerPoint (.pptx), Pages, Numbers, Keynote, CSV, Markdown, HTML, RTF, or a photo of a page. Returns the text page by page (slides as pages; headings as #, list items as -), tables as rows, a workbook's sheets as rows of values, and with page_images the PDF's pages (or the file's pictures) as images.",
            "inputSchema": {"type": "object", "properties": {
                "path": {"type": "string", "description": "The document's absolute path (or relative to your folder)."},
                "pages": {"type": "string", "description": "Pages or slides to read, like \"1-3,7\" (default all)."},
                "sheet": {"type": "string", "description": "A workbook's sheet to read (default all)."},
                "page_images": {"type": "boolean", "description": "Also render those pages as PNG images into your folder and show them (at most 20, 6 shown)."},
                "ocr": {"type": "string", "enum": ["auto", "always", "never"], "description": "OCR for PDF pages: auto reads pages without text (scans) by OCR."},
                "max_chars": {"type": "integer", "description": "Most characters returned (default 60000)."}
            }, "required": ["path"]},
        }),
        json!({
            "name": WRITE,
            "description": "Write a document from structured content; the file name's extension picks the format: .docx, .xlsx, .pptx, .pdf, .md, .html, .csv, .txt, .rtf, .odt, or .pages/.numbers/.key (through that app, when installed). Give the content as Markdown (headings, paragraphs, **bold**, lists, | tables |, ![](image.png), \\pagebreak) or as blocks; a spreadsheet takes sheets (cells: text, numbers, booleans, \"=SUM(B2:B9)\" formulas); a deck takes slides (title, bullets, text, table, image, notes), else each top heading becomes a slide. Writes into your folder unless the path is absolute.",
            "inputSchema": {"type": "object", "properties": {
                "path": {"type": "string", "description": "Where to write, e.g. \"Bericht.docx\" (into your folder) or an absolute path."},
                "title": {"type": "string"},
                "markdown": {"type": "string", "description": "The content as Markdown."},
                "blocks": {"type": "array", "description": "The content as blocks: {\"type\": \"heading\"|\"paragraph\"|\"bullets\"|\"table\"|\"image\"|\"quote\"|\"code\"|\"page_break\", \"text\", \"level\", \"items\", \"ordered\", \"rows\", \"header\", \"path\", \"width\"}.", "items": {"type": "object"}},
                "sheets": {"type": "array", "description": "For .xlsx/.csv/.numbers: [{\"name\", \"rows\": [[cell, …], …]}]; a first row of text is the header.", "items": {"type": "object"}},
                "slides": {"type": "array", "description": "For .pptx/.key: [{\"title\", \"bullets\": [..], \"text\", \"table\": [[..]], \"image\": path, \"notes\"}].", "items": {"type": "object"}},
                "page": {"type": "string", "enum": ["a4", "letter"], "description": "A PDF's page size (default a4)."},
                "landscape": {"type": "boolean"},
                "overwrite": {"type": "boolean", "description": "Replace an existing file (default false: a new name is chosen)."}
            }, "required": ["path"]},
        }),
    ]
}

/// `hex_document_read`: its text report, then page images.
pub fn call_read(arguments: &Value, folder: &Path) -> Result<Vec<Value>, String> {
    let started = Instant::now();
    let path = super::out::input(arguments, "path", folder, READ)?;
    let ask = read::Ask::from(arguments);
    let found = read::read(&path, &ask, folder)?;
    let report = read::report(&path, &found, &ask, started);
    Ok(super::out::content(
        &report,
        &found.images,
        read::inline_images(),
    ))
}

/// `hex_document_write`: where it wrote what.
pub fn call_write(arguments: &Value, folder: &Path) -> Result<Vec<Value>, String> {
    let started = Instant::now();
    let mut path = super::out::output(arguments, "path", folder, "document.md")?;
    if path.exists() && !arguments["overwrite"].as_bool().unwrap_or(false) {
        let parent = path.parent().unwrap_or(folder).to_owned();
        let stem = path.file_stem().map_or_else(
            || "document".into(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        path = parent.join(super::out::fresh_name(
            &parent,
            &stem,
            &super::out::extension(&path),
        ));
    }
    let document = blocks::Document::from_arguments(arguments);
    if document.blocks.is_empty()
        && document.sheets.is_empty()
        && document.slides.is_empty()
        && document.title.is_empty()
    {
        return Err(format!(
            "{WRITE} needs content: markdown, blocks, sheets, or slides."
        ));
    }
    let embeds = matches!(
        super::out::extension(&path).as_str(),
        "docx" | "pptx" | "pdf" | "pages" | "key"
    );
    let missing = write::missing_images(&document, folder);
    if embeds && !missing.is_empty() {
        return Err(format!("These images don't exist: {missing:?}"));
    }
    let mut report = write::write(&document, &path, folder, arguments)?;
    report["took_s"] = json!(super::out::took(started));
    Ok(vec![json!({"type": "text", "text": report.to_string()})])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("hex-doc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn text_of(content: &[Value]) -> Value {
        serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap()
    }

    /// A small PNG to put in documents.
    fn picture(folder: &Path) -> std::path::PathBuf {
        let path = folder.join("chart.png");
        let image = image::RgbaImage::from_fn(120, 80, |x, y| {
            image::Rgba([(x * 2) as u8, (y * 3) as u8, 160, 255])
        });
        image.save(&path).unwrap();
        path
    }

    const REPORT: &str = "## Umsatz 2026\n\nDer **Umsatz** stieg deutlich.\n\n| Monat | Umsatz | Kosten |\n|---|---|---|\n| Januar | 120 | 80 |\n| Februar | 140.5 | 90 |\n\n- Vertrieb\n- Marketing\n\n![](chart.png)\n\n## Ausblick\n\n1. Wachsen\n2. Sparen\n";

    #[test]
    fn office_documents_written_here_read_back() {
        let folder = folder("office");
        picture(&folder);
        for (name, wanted) in [
            (
                "Bericht.docx",
                vec![
                    "# Bericht",
                    "## Umsatz 2026",
                    "Der Umsatz stieg deutlich.",
                    "- Vertrieb",
                    "[Table 1]",
                ],
            ),
            (
                "Bericht.pptx",
                vec!["# Umsatz 2026", "Der Umsatz stieg deutlich.", "Vertrieb"],
            ),
        ] {
            let wrote = call_write(
                &json!({"path": name, "title": "Bericht", "markdown": REPORT}),
                &folder,
            )
            .unwrap();
            let wrote = text_of(&wrote);
            assert!(wrote["bytes"].as_u64().unwrap() > 1000, "{wrote}");
            let read = text_of(&call_read(&json!({"path": name}), &folder).unwrap());
            let text: String = read["pages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|page| page["text"].as_str().unwrap().to_owned() + "\n")
                .collect();
            for line in wanted {
                assert!(text.contains(line), "{name}: “{line}” missing in {text}");
            }
            assert_eq!(
                read["tables"][0]["rows"][1],
                json!(["Januar", "120", "80"]),
                "{name}"
            );
            // The system's unzip agrees the archive is sound.
            if Path::new("/usr/bin/unzip").exists() {
                let tested = std::process::Command::new("/usr/bin/unzip")
                    .args(["-tqq"])
                    .arg(folder.join(name))
                    .status()
                    .unwrap();
                assert!(tested.success(), "{name}");
            }
        }
        // Each top heading is a slide, after the title slide; the picture
        // is in the deck.
        let deck = text_of(
            &call_read(
                &json!({"path": "Bericht.pptx", "page_images": true}),
                &folder,
            )
            .unwrap(),
        );
        assert_eq!(deck["about"]["slides"], 3);
        assert_eq!(deck["images"].as_array().unwrap().len(), 1);
        let slide =
            text_of(&call_read(&json!({"path": "Bericht.pptx", "pages": "3"}), &folder).unwrap());
        assert_eq!(slide["pages"].as_array().unwrap().len(), 1);
        assert!(
            slide["pages"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Wachsen")
        );
        // Speaker notes go to notes pages and come back.
        call_write(
            &json!({"path": "talk.pptx", "slides": [
                {"title": "Hallo", "bullets": ["eins"], "notes": "Langsam sprechen."},
                {"title": "Daten", "table": [["a", "b"], ["1", "2"]]}
            ]}),
            &folder,
        )
        .unwrap();
        let talk = text_of(&call_read(&json!({"path": "talk.pptx"}), &folder).unwrap());
        assert!(
            talk["pages"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Notes: Langsam sprechen.")
        );
        assert_eq!(talk["tables"][0]["rows"], json!([["a", "b"], ["1", "2"]]));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn workbooks_and_csv_keep_their_values() {
        let folder = folder("sheets");
        call_write(
            &json!({"path": "Zahlen.xlsx", "sheets": [
                {"name": "Umsatz", "rows": [["Monat", "Betrag", "Bezahlt"], ["Jan", 120, true], ["Feb", 140.5, false], ["Summe", "=SUM(B2:B3)", null]]},
                {"name": "Notizen", "rows": [["frei"]]}
            ]}),
            &folder,
        )
        .unwrap();
        let read = text_of(&call_read(&json!({"path": "Zahlen.xlsx"}), &folder).unwrap());
        assert_eq!(read["about"]["sheets"], json!(["Umsatz", "Notizen"]));
        assert_eq!(read["sheets"][0]["rows"][1], json!(["Jan", 120, true]));
        assert_eq!(read["sheets"][0]["rows"][2][1], json!(140.5));
        let one = text_of(
            &call_read(&json!({"path": "Zahlen.xlsx", "sheet": "notizen"}), &folder).unwrap(),
        );
        assert_eq!(one["sheets"].as_array().unwrap().len(), 1);
        assert!(call_read(&json!({"path": "Zahlen.xlsx", "sheet": "Fehlt"}), &folder).is_err());
        // Tables in Markdown become sheets; CSV round-trips.
        call_write(&json!({"path": "t.csv", "markdown": REPORT}), &folder).unwrap();
        let csv = text_of(&call_read(&json!({"path": "t.csv"}), &folder).unwrap());
        assert_eq!(csv["sheets"][0]["rows"][2], json!(["Februar", 140.5, 90]));
        // An existing file is kept: the new one gets a fresh name.
        let again =
            text_of(&call_write(&json!({"path": "t.csv", "markdown": REPORT}), &folder).unwrap());
        assert!(again["file"].as_str().unwrap().ends_with("t-2.csv"));
        assert!(
            call_write(&json!({"path": "x.docx"}), &folder)
                .unwrap_err()
                .contains("needs content")
        );
        assert!(
            call_write(
                &json!({"path": "x.docx", "markdown": "![](none.png)"}),
                &folder
            )
            .unwrap_err()
            .contains("don't exist")
        );
        assert!(
            call_write(&json!({"path": "x.zzz", "markdown": "a"}), &folder)
                .unwrap_err()
                .contains("can't write")
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn markdown_html_and_text_documents_read_back() {
        let folder = folder("text");
        picture(&folder);
        call_write(
            &json!({"path": "r.md", "title": "Bericht", "markdown": REPORT}),
            &folder,
        )
        .unwrap();
        call_write(
            &json!({"path": "r.html", "title": "Bericht", "markdown": REPORT}),
            &folder,
        )
        .unwrap();
        let md = text_of(&call_read(&json!({"path": "r.md"}), &folder).unwrap());
        assert!(
            md["pages"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("# Bericht\n\n## Umsatz 2026")
        );
        let html = text_of(&call_read(&json!({"path": "r.html"}), &folder).unwrap());
        assert!(
            html["pages"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Umsatz 2026")
        );
        assert_eq!(
            html["tables"][0]["rows"][2],
            json!(["Februar", "140.5", "90"])
        );
        assert!(call_read(&json!({"path": "missing.pdf"}), &folder).is_err());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// PDF through PDFKit and Core Text, scanned pages through Vision, RTF
    /// and DOC through textutil (macOS only).
    #[cfg(target_os = "macos")]
    #[test]
    fn pdfs_and_scans_are_written_and_read_on_macos() {
        let folder = folder("pdf");
        picture(&folder);
        let started = Instant::now();
        let long: String = (1..=120)
            .map(|n| format!("Zeile {n} des langen Absatzes. "))
            .collect();
        let wrote = text_of(
            &call_write(
                &json!({"path": "Bericht.pdf", "title": "Bericht Ä", "markdown": format!("{REPORT}\n\\pagebreak\n\n## Anhang\n\n{long}")}),
                &folder,
            )
            .unwrap(),
        );
        assert!(wrote["pages"].as_u64().unwrap() >= 2, "{wrote}");
        let read = call_read(
            &json!({"path": "Bericht.pdf", "page_images": true, "pages": "1"}),
            &folder,
        )
        .unwrap();
        let report = text_of(&read);
        let text = report["pages"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("Bericht Ä") && text.contains("Umsatz 2026") && text.contains("Februar"),
            "{text}"
        );
        assert_eq!(report["pages"].as_array().unwrap().len(), 1);
        assert_eq!(report["about"]["page_count"], wrote["pages"]);
        assert_eq!(
            read.iter().filter(|item| item["type"] == "image").count(),
            1
        );
        // A scan: the page image as a picture-only PDF reads through OCR.
        let page = report["images"][0].as_str().unwrap().to_owned();
        let scan = text_of(
            &call_write(
                &json!({"path": "scan.pdf", "markdown": format!("![]({page})")}),
                &folder,
            )
            .unwrap(),
        );
        assert_eq!(scan["pages"], 1);
        let scanned = text_of(&call_read(&json!({"path": "scan.pdf"}), &folder).unwrap());
        assert_eq!(scanned["pages"][0]["ocr"], true, "{scanned}");
        let words = scanned["pages"][0]["text"].as_str().unwrap();
        assert!(
            words.contains("Umsatz") && words.contains("Januar"),
            "{words}"
        );
        // A photo of the page reads the same way.
        let photo = text_of(&call_read(&json!({"path": page}), &folder).unwrap());
        assert!(
            photo["pages"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Vertrieb")
        );
        // RTF and DOC through textutil.
        call_write(&json!({"path": "r.rtf", "markdown": REPORT}), &folder).unwrap();
        let rtf = text_of(&call_read(&json!({"path": "r.rtf"}), &folder).unwrap());
        assert!(
            rtf["pages"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Marketing")
        );
        println!(
            "PDF write, read, OCR, RTF: {:.1} s",
            started.elapsed().as_secs_f64()
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}
