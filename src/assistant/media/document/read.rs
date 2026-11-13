//! `hex_document_read`: any document's text, tables, and (on request) its
//! pages as images. PDF through PDFKit with Vision's OCR for scanned pages;
//! Word, Excel, and PowerPoint files read here; CSV and Markdown as text;
//! HTML, RTF, DOC, and ODT through macOS's textutil; Pages, Numbers, and
//! Keynote exported by their own app to the Office format first; a picture
//! of a page through OCR.

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{Value, json};

use super::{apple, office};
use crate::assistant::media::out;
use crate::assistant::media::zip::Archive;

/// Characters of text and table cells returned at most, by default.
const DEFAULT_CHARS: usize = 60_000;
/// Page images rendered at most, and shown inline at most.
const MAX_PAGE_IMAGES: usize = 20;
const INLINE_IMAGES: usize = 6;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub n: usize,
    pub text: String,
    pub ocr: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

/// What reading found.
#[derive(Debug, Default)]
pub struct Read {
    pub format: &'static str,
    pub pages: Vec<Page>,
    pub tables: Vec<Table>,
    /// A workbook's sheets: name and rows of values.
    pub sheets: Vec<(String, Vec<Vec<Value>>)>,
    pub images: Vec<PathBuf>,
    pub meta: Value,
    pub cut: bool,
}

/// What the agent asked for.
pub struct Ask {
    pub pages: Option<Vec<(usize, usize)>>,
    pub sheet: Option<String>,
    pub ocr: String,
    pub page_images: bool,
    pub max_chars: usize,
}

impl Ask {
    pub fn from(arguments: &Value) -> Self {
        Self {
            pages: arguments["pages"].as_str().and_then(ranges).or_else(|| {
                arguments["pages"]
                    .as_u64()
                    .map(|page| vec![(page as usize, page as usize)])
            }),
            sheet: arguments["sheet"].as_str().map(str::to_owned),
            ocr: arguments["ocr"].as_str().unwrap_or("auto").to_owned(),
            page_images: arguments["page_images"].as_bool().unwrap_or(false),
            max_chars: arguments["max_chars"]
                .as_u64()
                .map_or(DEFAULT_CHARS, |max| (max as usize).clamp(2_000, 400_000)),
        }
    }

    fn wants(&self, page: usize) -> bool {
        self.pages.as_ref().is_none_or(|ranges| {
            ranges
                .iter()
                .any(|(first, last)| (*first..=*last).contains(&page))
        })
    }

    /// The first and last page asked for (all when none were).
    fn span(&self) -> (usize, usize) {
        match &self.pages {
            Some(ranges) => (
                ranges.iter().map(|range| range.0).min().unwrap_or(1),
                ranges
                    .iter()
                    .map(|range| range.1)
                    .max()
                    .unwrap_or(usize::MAX),
            ),
            None => (1, usize::MAX),
        }
    }
}

/// `"1-3, 5"` as page ranges.
pub fn ranges(text: &str) -> Option<Vec<(usize, usize)>> {
    let mut ranges = Vec::new();
    for part in text
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (first, last) = part.split_once('-').unwrap_or((part, part));
        let first = first.trim().parse::<usize>().ok()?.max(1);
        let last = if last.trim().is_empty() {
            usize::MAX
        } else {
            last.trim().parse::<usize>().ok()?
        };
        ranges.push((first, last.max(first)));
    }
    (!ranges.is_empty()).then_some(ranges)
}

/// Reads `path`; page images and pictures go into `folder`.
pub fn read(path: &Path, ask: &Ask, folder: &Path) -> Result<Read, String> {
    let extension = out::extension(path);
    let stem = path.file_stem().map_or_else(
        || "document".into(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let pictures = folder.join(format!("{}-pages", safe(&stem)));
    let zipped = || -> Result<Archive, String> {
        let bytes =
            std::fs::read(path).map_err(|error| format!("Couldn't read the file: {error}"))?;
        Archive::new(bytes)
    };
    let mut read = match extension.as_str() {
        "pdf" => apple::pdf(path, ask, &pictures)?,
        "docx" | "docm" | "dotx" => office::docx(&zipped()?)?,
        "xlsx" | "xlsm" => office::xlsx(&zipped()?, ask.sheet.as_deref())?,
        "pptx" | "pptm" => office::pptx(&zipped()?, &|page| ask.wants(page))?,
        "csv" | "tsv" => csv_file(path)?,
        "md" | "markdown" | "txt" | "text" | "json" | "xml" | "yaml" | "yml" | "log" => Read {
            format: "text",
            pages: vec![Page {
                n: 1,
                text: std::fs::read_to_string(path)
                    .map_err(|error| format!("Couldn't read the file as text: {error}"))?,
                ocr: false,
            }],
            ..Read::default()
        },
        "html" | "htm" => html(path)?,
        "rtf" | "rtfd" | "doc" | "odt" | "wordml" | "webarchive" => Read {
            format: "text",
            pages: vec![Page {
                n: 1,
                text: apple::textutil_text(path)?,
                ocr: false,
            }],
            ..Read::default()
        },
        "pages" | "numbers" | "key" => {
            let exported = apple::iwork_export(path, folder)?;
            let mut read = read(&exported, ask, folder)?;
            read.meta["exported_to"] = json!(exported);
            read
        }
        "png" | "jpg" | "jpeg" | "heic" | "tif" | "tiff" | "gif" | "bmp" | "webp" => {
            let text = apple::ocr(path)?;
            Read {
                format: "image (OCR)",
                pages: vec![Page {
                    n: 1,
                    text: text["text"].as_str().unwrap_or_default().to_owned(),
                    ocr: true,
                }],
                images: vec![path.to_owned()],
                ..Read::default()
            }
        }
        other => {
            return Err(format!(
                "“.{other}” isn't a document format hex_document_read knows (PDF, Word, Excel, PowerPoint, Pages, Numbers, Keynote, CSV, Markdown, HTML, RTF, images)."
            ));
        }
    };
    if ask.page_images && read.images.is_empty() {
        let prefix = match read.format {
            "docx" => Some("word/media/"),
            "pptx" => Some("ppt/media/"),
            "xlsx" => Some("xl/media/"),
            _ => None,
        };
        if let Some(prefix) = prefix {
            read.images = office::pictures(&zipped()?, prefix, &pictures, MAX_PAGE_IMAGES);
            read.meta["images_are"] = json!("the pictures embedded in the file");
        }
    }
    Ok(read)
}

fn safe(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect()
}

/// A CSV or TSV file as one sheet: its delimiter guessed from the first line.
fn csv_file(path: &Path) -> Result<Read, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("Couldn't read the file: {error}"))?;
    let text = String::from_utf8(bytes.clone()).unwrap_or_else(|_| {
        // Excel's CSVs are often Latin-1.
        bytes.iter().map(|&byte| byte as char).collect()
    });
    let rows = csv(&text);
    Ok(Read {
        format: "csv",
        sheets: vec![(
            "csv".into(),
            rows.into_iter()
                .map(|row| row.iter().map(|cell| super::blocks::typed(cell)).collect())
                .collect(),
        )],
        ..Read::default()
    })
}

/// CSV rows (quotes, doubled quotes, and line breaks in quotes), with the
/// delimiter (`,`, `;`, or tab) guessed from the first line.
pub fn csv(text: &str) -> Vec<Vec<String>> {
    let text = text.trim_start_matches('\u{feff}');
    let first = text.lines().next().unwrap_or_default();
    let delimiter = [',', ';', '\t']
        .into_iter()
        .max_by_key(|delimiter| first.matches(*delimiter).count())
        .unwrap_or(',');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == delimiter && !quoted => row.push(std::mem::take(&mut cell)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// An HTML file's text (textutil on macOS, else its tags dropped) and its
/// tables.
fn html(path: &Path) -> Result<Read, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|error| format!("Couldn't read the file: {error}"))?;
    let text = apple::textutil_text(path).unwrap_or_else(|_| strip_tags(&source));
    Ok(Read {
        format: "html",
        pages: vec![Page {
            n: 1,
            text,
            ocr: false,
        }],
        tables: html_tables(&source),
        ..Read::default()
    })
}

fn strip_tags(html: &str) -> String {
    let without = regex::Regex::new(r"(?is)<(script|style)[^>]*>.*?</(script|style)>")
        .map(|pattern| pattern.replace_all(html, " ").into_owned())
        .unwrap_or_else(|_| html.to_owned());
    let breaks = regex::Regex::new(r"(?i)<(br|/p|/div|/h[1-6]|/li|/tr)[^>]*>")
        .map(|pattern| pattern.replace_all(&without, "\n").into_owned())
        .unwrap_or(without);
    let text = regex::Regex::new(r"<[^>]*>")
        .map(|pattern| pattern.replace_all(&breaks, "").into_owned())
        .unwrap_or(breaks);
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The HTML's `<table>`s as rows of cell text.
fn html_tables(html: &str) -> Vec<Table> {
    let (Ok(tables), Ok(rows), Ok(cells)) = (
        regex::Regex::new(r"(?is)<table[^>]*>(.*?)</table>"),
        regex::Regex::new(r"(?is)<tr[^>]*>(.*?)</tr>"),
        regex::Regex::new(r"(?is)<t[dh][^>]*>(.*?)</t[dh]>"),
    ) else {
        return Vec::new();
    };
    tables
        .captures_iter(html)
        .enumerate()
        .map(|(index, table)| Table {
            name: format!("Table {}", index + 1),
            rows: rows
                .captures_iter(&table[1])
                .map(|row| {
                    cells
                        .captures_iter(&row[1])
                        .map(|cell| strip_tags(&cell[1]).replace('\n', " "))
                        .collect()
                })
                .filter(|row: &Vec<String>| !row.is_empty())
                .collect(),
        })
        .collect()
}

/// The tool's report: what was read, its text and tables (cut to
/// `max_chars`), and the page images.
pub fn report(path: &Path, read: &Read, ask: &Ask, started: Instant) -> Value {
    let mut budget = ask.max_chars;
    let mut cut = read.cut;
    let mut take = |text: &str| -> String {
        let count = text.chars().count();
        if count <= budget {
            budget -= count;
            text.to_owned()
        } else {
            cut = true;
            let kept: String = text.chars().take(budget).collect();
            budget = 0;
            kept
        }
    };
    let pages: Vec<Value> = read
        .pages
        .iter()
        .map(|page| {
            let mut value = json!({"n": page.n, "text": take(&page.text)});
            if page.ocr {
                value["ocr"] = json!(true);
            }
            value
        })
        .collect();
    let tables: Vec<Value> = read
        .tables
        .iter()
        .map(|table| {
            let rows: Vec<Vec<String>> = table
                .rows
                .iter()
                .map(|row| row.iter().map(|cell| take(cell)).collect())
                .collect();
            json!({"name": table.name, "rows": rows})
        })
        .collect();
    let sheets: Vec<Value> = read
        .sheets
        .iter()
        .map(|(name, rows)| {
            let total = rows.len();
            let mut kept = Vec::new();
            for row in rows {
                let size: usize = row.iter().map(|cell| cell.to_string().len() + 1).sum();
                if size > budget {
                    cut = true;
                    break;
                }
                budget -= size;
                kept.push(row.clone());
            }
            json!({"name": name, "rows_total": total, "rows": kept})
        })
        .collect();
    let mut report = json!({
        "file": path,
        "format": read.format,
        "took_s": out::took(started),
    });
    if !pages.is_empty() {
        report["pages"] = json!(pages);
    }
    if !tables.is_empty() {
        report["tables"] = json!(tables);
    }
    if !sheets.is_empty() {
        report["sheets"] = json!(sheets);
    }
    if !read.meta.is_null() {
        report["about"] = read.meta.clone();
    }
    if !read.images.is_empty() {
        report["images"] = json!(read.images);
        report["images_shown"] = json!(read.images.len().min(INLINE_IMAGES));
    }
    if cut {
        report["cut"] = json!(
            "Not everything fit: ask for fewer pages (\"pages\": \"5-9\"), one sheet, or a larger max_chars."
        );
    }
    report
}

/// How many images go inline.
pub const fn inline_images() -> usize {
    INLINE_IMAGES
}

/// Page images rendered at most.
pub const fn max_page_images() -> usize {
    MAX_PAGE_IMAGES
}

impl Ask {
    /// The pages to render as images: those asked for, at most
    /// [`MAX_PAGE_IMAGES`] from the first.
    pub fn image_pages(&self) -> Vec<usize> {
        if !self.page_images {
            return Vec::new();
        }
        let (first, last) = self.span();
        (first..=last.min(first + 400))
            .filter(|page| self.wants(*page))
            .take(MAX_PAGE_IMAGES)
            .collect()
    }

    /// The page span for the PDF bridge.
    pub fn bridge_span(&self) -> (usize, usize) {
        self.span()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ranges_csv_and_html_are_read() {
        assert_eq!(ranges("1-3, 5"), Some(vec![(1, 3), (5, 5)]));
        assert_eq!(ranges("4-"), Some(vec![(4, usize::MAX)]));
        assert_eq!(ranges("x"), None);
        let ask = Ask::from(&json!({"pages": "2-3", "page_images": true}));
        assert!(!ask.wants(1) && ask.wants(3));
        assert_eq!(ask.image_pages(), [2, 3]);
        assert_eq!(Ask::from(&json!({"pages": 4})).bridge_span(), (4, 4));
        assert_eq!(
            csv("a;b\n\"x;\"\"y\"\"\";2\r\n"),
            [vec!["a", "b"], vec!["x;\"y\"", "2"]]
        );
        assert_eq!(csv("a\tb\nc\td"), [vec!["a", "b"], vec!["c", "d"]]);
        let html = "<html><style>p{}</style><h1>Hi</h1><p>A&amp;B</p><table><tr><th>k</th><th>v</th></tr><tr><td>1</td><td><b>2</b></td></tr></table></html>";
        assert_eq!(strip_tags(html), "Hi\nA&B\nkv\n12");
        assert_eq!(html_tables(html)[0].rows, [vec!["k", "v"], vec!["1", "2"]]);
    }

    #[test]
    fn the_report_keeps_within_its_budget() {
        let read = Read {
            format: "text",
            pages: vec![
                Page {
                    n: 1,
                    text: "a".repeat(1500),
                    ocr: false,
                },
                Page {
                    n: 2,
                    text: "b".repeat(1500),
                    ocr: true,
                },
            ],
            ..Read::default()
        };
        let ask = Ask::from(&json!({"max_chars": 2000}));
        let report = report(Path::new("/x.txt"), &read, &ask, Instant::now());
        assert_eq!(report["pages"][1]["text"].as_str().unwrap().len(), 500);
        assert_eq!(report["pages"][1]["ocr"], true);
        assert!(report["cut"].is_string());
    }
}
