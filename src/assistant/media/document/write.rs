//! `hex_document_write`: a document from structured content, its format
//! from the file name. Word (DOCX) and Excel (XLSX) are written here as
//! Office Open XML, PowerPoint in [`super::slides`]; PDF is laid out by
//! Core Text (`apple`); RTF, DOC, and ODT go through textutil from HTML;
//! Pages, Numbers, and Keynote files are their app's import of the Office
//! twin (when the app is installed).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::apple;
use super::blocks::{Block, Document, Sheet, plain, runs, to_markdown};
use crate::assistant::media::out;
use crate::assistant::media::zip::Writer;

/// The formats written, by extension.
pub const FORMATS: &str =
    "docx, xlsx, pptx, pdf, md, html, csv, txt, rtf, doc, odt, pages, numbers, key";

/// Text escaped for XML.
pub fn xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // XML 1.0 forbids most control characters.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// Writes `document` to `path` (images named relative to `base`); what
/// was written.
pub fn write(
    document: &Document,
    path: &Path,
    base: &Path,
    arguments: &Value,
) -> Result<Value, String> {
    let extension = out::extension(path);
    let mut report = json!({"file": path, "format": extension});
    match extension.as_str() {
        "docx" => std::fs::write(path, docx(document, base)?),
        "xlsx" => {
            let sheets = document.sheets();
            if sheets.is_empty() {
                return Err(
                    "An Excel file needs “sheets” (or tables in the blocks or Markdown).".into(),
                );
            }
            report["sheets"] = json!(sheets.iter().map(|sheet| &sheet.name).collect::<Vec<_>>());
            std::fs::write(path, xlsx(&sheets))
        }
        "pptx" => {
            let slides = document.slides();
            if slides.is_empty() {
                return Err(
                    "A deck needs “slides” (or headings in the blocks or Markdown).".into(),
                );
            }
            report["slides"] = json!(slides.len());
            std::fs::write(path, super::slides::pptx(&document.title, &slides, base)?)
        }
        "pdf" => {
            let mut document = document.clone();
            if document.blocks.is_empty() {
                document.blocks = super::blocks::from_markdown(&to_markdown(&document));
                document.title.clear();
            }
            resolve_images(&mut document.blocks, base);
            let pages = apple::write_pdf(
                path,
                &document.title,
                &document.blocks_json(),
                arguments["page"].as_str().unwrap_or("a4"),
                arguments["landscape"].as_bool().unwrap_or(false),
            )?;
            report["pages"] = json!(pages);
            Ok(())
        }
        "md" | "markdown" => std::fs::write(path, to_markdown(document)),
        "txt" => std::fs::write(path, plain(&to_markdown(document))),
        "html" | "htm" => std::fs::write(path, html(document, base)),
        "csv" => {
            let sheets = document.sheets();
            let sheet = sheets
                .first()
                .ok_or("A CSV file needs a sheet or a table.")?;
            std::fs::write(path, csv(sheet))
        }
        "rtf" | "doc" | "odt" => {
            let html_file = path.with_extension("source.html");
            std::fs::write(&html_file, html(document, base)).map_err(|error| error.to_string())?;
            let converted = apple::textutil_convert(&html_file, path);
            let _ = std::fs::remove_file(&html_file);
            converted?;
            Ok(())
        }
        "pages" | "numbers" | "key" => {
            let office = apple::iwork_office(&extension).ok_or("not an iWork format")?;
            let twin = path.with_extension(office);
            write(document, &twin, base, arguments)?;
            apple::iwork_import(&twin, path)?;
            report["office_copy"] = json!(twin);
            Ok(())
        }
        other => {
            return Err(format!(
                "hex_document_write can't write “.{other}”; it writes {FORMATS}."
            ));
        }
    }
    .map_err(|error| format!("Couldn't write {}: {error}", path.display()))?;
    let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    report["bytes"] = json!(size);
    Ok(report)
}

/// Image blocks' paths made absolute against `base`.
fn resolve_images(blocks: &mut [Block], base: &Path) {
    for block in blocks {
        if let Block::Image { path, .. } = block {
            *path = out::resolve(path, base).to_string_lossy().into_owned();
        }
    }
}

/// A sheet as CSV.
pub fn csv(sheet: &Sheet) -> String {
    let mut out = String::new();
    for row in &sheet.rows {
        let cells: Vec<String> = row
            .iter()
            .map(|cell| {
                let text = match cell {
                    Value::String(text) => text.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                if text.contains([',', '"', '\n', ';']) {
                    format!("\"{}\"", text.replace('"', "\"\""))
                } else {
                    text
                }
            })
            .collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

/// The document as one HTML page.
pub fn html(document: &Document, base: &Path) -> String {
    let inline = |text: &str| -> String {
        runs(text)
            .into_iter()
            .map(|(part, bold)| {
                if bold {
                    format!("<b>{}</b>", xml(&part))
                } else {
                    xml(&part)
                }
            })
            .collect()
    };
    let mut body = String::new();
    if !document.title.is_empty() {
        let _ = writeln!(body, "<h1>{}</h1>", xml(&document.title));
    }
    for block in &document.blocks {
        match block {
            Block::Heading { level, text } => {
                let _ = writeln!(body, "<h{level}>{}</h{level}>", inline(text));
            }
            Block::Paragraph(text) => {
                let _ = writeln!(body, "<p>{}</p>", inline(text));
            }
            Block::Bullets { items, ordered } => {
                let tag = if *ordered { "ol" } else { "ul" };
                body.push_str(&format!("<{tag}>"));
                for item in items {
                    let _ = write!(body, "<li>{}</li>", inline(item));
                }
                body.push_str(&format!("</{tag}>\n"));
            }
            Block::Table { rows, header } => {
                body.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"4\">");
                for (index, row) in rows.iter().enumerate() {
                    let cell = if *header && index == 0 { "th" } else { "td" };
                    body.push_str("<tr>");
                    for text in row {
                        let _ = write!(body, "<{cell}>{}</{cell}>", inline(text));
                    }
                    body.push_str("</tr>");
                }
                body.push_str("</table>\n");
            }
            Block::Image { path, width } => {
                let source = out::resolve(path, base);
                let width = width
                    .map(|width| format!(" width=\"{width}\""))
                    .unwrap_or_default();
                let _ = writeln!(
                    body,
                    "<p><img src=\"{}\"{width}></p>",
                    xml(&source.to_string_lossy())
                );
            }
            Block::Quote(text) => {
                let _ = writeln!(body, "<blockquote>{}</blockquote>", inline(text));
            }
            Block::Code(text) => {
                let _ = writeln!(body, "<pre>{}</pre>", xml(text));
            }
            Block::PageBreak => body.push_str("<div style=\"page-break-after: always\"></div>\n"),
        }
    }
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{}</title>\
         <style>body{{font-family:-apple-system,Helvetica,Arial,sans-serif;max-width:46em;margin:2em auto;line-height:1.45}}\
         table{{border-collapse:collapse}}th{{background:#eee}}</style></head><body>\n{body}</body></html>\n",
        xml(&document.title)
    )
}

const CONTENT_TYPES_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/><Default Extension=\"gif\" ContentType=\"image/gif\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>";

/// `[Content_Types].xml` with `overrides` (part, type).
pub fn content_types(overrides: &[(String, &str)]) -> String {
    let mut out = CONTENT_TYPES_HEAD.to_owned();
    for (part, kind) in overrides {
        let _ = write!(
            out,
            "<Override PartName=\"{part}\" ContentType=\"{kind}\"/>"
        );
    }
    out.push_str("</Types>");
    out
}

/// The package's root relationships: the main part and the core properties.
pub fn root_rels(main: &str, kind: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{main}\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>"
    )
}

/// `docProps/core.xml` with `title`.
pub fn core(title: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:creator>Agent-Desk</dc:creator></cp:coreProperties>",
        xml(title)
    )
}

/// Relationships XML of `(id, type, target)`.
pub fn rels(relations: &[(String, String, String)]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, kind, target) in relations {
        let _ = write!(
            out,
            "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{}\"/>",
            xml(target)
        );
    }
    out.push_str("</Relationships>");
    out
}

/// A picture's bytes, extension, and size in pixels.
pub type Picture = (Vec<u8>, &'static str, (u32, u32));

/// A picture for a package: PNG or JPEG bytes (anything else becomes PNG),
/// its extension, and its size in pixels.
pub fn picture(path: &Path) -> Result<Picture, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Couldn't read the image {}: {error}", path.display()))?;
    let format =
        image::guess_format(&bytes).map_err(|_| format!("{} isn't an image", path.display()))?;
    let decoded = image::load_from_memory(&bytes)
        .map_err(|error| format!("Couldn't read the image {}: {error}", path.display()))?;
    let size = (decoded.width(), decoded.height());
    match format {
        image::ImageFormat::Png => Ok((bytes, "png", size)),
        image::ImageFormat::Jpeg => Ok((bytes, "jpeg", size)),
        _ => {
            let mut png = Vec::new();
            decoded
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|error| error.to_string())?;
            Ok((png, "png", size))
        }
    }
}

/// English Metric Units per pixel at 96 dpi, and per point.
pub const EMU_PER_PX: u64 = 9525;
pub const EMU_PER_PT: u64 = 12_700;

fn word_runs(text: &str, extra: &str) -> String {
    let mut out = String::new();
    for (part, bold) in runs(text) {
        let props = match (bold, extra.is_empty()) {
            (false, true) => String::new(),
            (bold, _) => format!("<w:rPr>{}{extra}</w:rPr>", if bold { "<w:b/>" } else { "" }),
        };
        for (index, line) in part.split('\n').enumerate() {
            if index > 0 {
                let _ = write!(out, "<w:r>{props}<w:br/></w:r>");
            }
            let _ = write!(
                out,
                "<w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
                xml(line)
            );
        }
    }
    out
}

fn word_paragraph(style: &str, text: &str) -> String {
    let props = if style.is_empty() {
        String::new()
    } else {
        format!("<w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>")
    };
    format!("<w:p>{props}{}</w:p>", word_runs(text, ""))
}

const WORD_STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:eastAsia=\"Calibri\" w:cs=\"Calibri\"/><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"120\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style><w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:spacing w:after=\"240\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"48\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"360\" w:after=\"120\"/><w:outlineLvl w:val=\"0\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"36\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"240\" w:after=\"80\"/><w:outlineLvl w:val=\"1\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"30\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading3\"><w:name w:val=\"heading 3\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"200\" w:after=\"60\"/><w:outlineLvl w:val=\"2\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"26\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading4\"><w:name w:val=\"heading 4\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val=\"3\"/></w:pPr><w:rPr><w:b/><w:i/><w:sz w:val=\"24\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:spacing w:after=\"40\"/><w:ind w:left=\"720\"/></w:pPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:ind w:left=\"567\"/></w:pPr><w:rPr><w:i/><w:color w:val=\"555555\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Code\"><w:name w:val=\"Code\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:after=\"0\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Menlo\" w:hAnsi=\"Menlo\" w:cs=\"Menlo\"/><w:sz w:val=\"18\"/></w:rPr></w:style><w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:tblPr><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/></w:tblBorders><w:tblCellMar><w:left w:w=\"80\" w:type=\"dxa\"/><w:right w:w=\"80\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style></w:styles>";

/// A table's own borders, for viewers that skip the table style's.
const TABLE_BORDERS: &str = "<w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/></w:tblBorders>";

/// The numbering part: bullets (`numId` 1), and one numbered list per
/// `ordered` count (each restarting at 1).
fn word_numbering(ordered: usize) -> String {
    let level = |format: &str, text: &str| {
        format!(
            "<w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{format}\"/><w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr></w:lvl>"
        )
    };
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:abstractNum w:abstractNumId=\"0\">{}</w:abstractNum><w:abstractNum w:abstractNumId=\"1\">{}</w:abstractNum><w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>",
        level("bullet", "•"),
        level("decimal", "%1.")
    );
    for index in 0..ordered {
        let _ = write!(
            out,
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>",
            index + 2
        );
    }
    out.push_str("</w:numbering>");
    out
}

/// A Word document.
pub fn docx(document: &Document, base: &Path) -> Result<Vec<u8>, String> {
    let mut body = String::new();
    let mut media: Vec<(String, Vec<u8>)> = Vec::new();
    let mut ordered = 0;
    if !document.title.is_empty() {
        body.push_str(&word_paragraph("Title", &document.title));
    }
    for block in &document.blocks {
        match block {
            Block::Heading { level, text } => {
                body.push_str(&word_paragraph(&format!("Heading{}", level.min(&4)), text));
            }
            Block::Paragraph(text) => body.push_str(&word_paragraph("", text)),
            Block::Bullets {
                items,
                ordered: numbered,
            } => {
                let id = if *numbered {
                    ordered += 1;
                    ordered + 1
                } else {
                    1
                };
                for item in items {
                    let _ = write!(
                        body,
                        "<w:p><w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"{id}\"/></w:numPr></w:pPr>{}</w:p>",
                        word_runs(item, "")
                    );
                }
            }
            Block::Table { rows, header } => {
                let columns = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
                // 9026 twips: A4's width inside 2.54 cm margins.
                let width = 9026 / columns;
                let _ = write!(
                    body,
                    "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"5000\" w:type=\"pct\"/>{TABLE_BORDERS}<w:tblLook w:val=\"04A0\" w:firstRow=\"1\"/></w:tblPr><w:tblGrid>{}</w:tblGrid>",
                    format!("<w:gridCol w:w=\"{width}\"/>").repeat(columns)
                );
                for (index, row) in rows.iter().enumerate() {
                    let head = *header && index == 0;
                    body.push_str(if head {
                        "<w:tr><w:trPr><w:tblHeader/></w:trPr>"
                    } else {
                        "<w:tr>"
                    });
                    for column in 0..columns {
                        let text = row.get(column).map_or("", String::as_str);
                        let shade = if head {
                            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"E7E6E6\"/>"
                        } else {
                            ""
                        };
                        let _ = write!(
                            body,
                            "<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/>{shade}</w:tcPr><w:p><w:pPr><w:spacing w:after=\"0\"/></w:pPr>{}</w:p></w:tc>",
                            word_runs(text, if head { "<w:b/>" } else { "" })
                        );
                    }
                    body.push_str("</w:tr>");
                }
                body.push_str("</w:tbl><w:p/>");
            }
            Block::Image { path, width } => {
                let (bytes, extension, (px_width, px_height)) = picture(&out::resolve(path, base))?;
                let n = media.len() + 1;
                media.push((format!("image{n}.{extension}"), bytes));
                // At most 16 cm wide; `width` is in points.
                let max = 5_760_000_u64;
                let cx = width
                    .map_or(u64::from(px_width) * EMU_PER_PX, |points| {
                        (points * EMU_PER_PT as f64) as u64
                    })
                    .min(max)
                    .max(1);
                let cy = cx * u64::from(px_height) / u64::from(px_width.max(1));
                let _ = write!(
                    body,
                    "<w:p><w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{n}\" name=\"Picture {n}\"/><a:graphic xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:nvPicPr><pic:cNvPr id=\"{n}\" name=\"image{n}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"rIdImage{n}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"
                );
            }
            Block::Quote(text) => body.push_str(&word_paragraph("Quote", text)),
            Block::Code(text) => {
                for line in text.lines() {
                    let _ = write!(
                        body,
                        "<w:p><w:pPr><w:pStyle w:val=\"Code\"/></w:pPr><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                        xml(line)
                    );
                }
            }
            Block::PageBreak => body.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>"),
        }
    }
    let document_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>"
    );
    let mut relations = vec![
        (
            "rIdStyles".to_owned(),
            "styles".to_owned(),
            "styles.xml".to_owned(),
        ),
        (
            "rIdNumbering".to_owned(),
            "numbering".to_owned(),
            "numbering.xml".to_owned(),
        ),
    ];
    for (index, (name, _)) in media.iter().enumerate() {
        relations.push((
            format!("rIdImage{}", index + 1),
            "image".into(),
            format!("media/{name}"),
        ));
    }
    let mut zip = Writer::default();
    zip.add(
        "[Content_Types].xml",
        content_types(&[
            (
                "/word/document.xml".into(),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            ),
            (
                "/word/styles.xml".into(),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml",
            ),
            (
                "/word/numbering.xml".into(),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
            ),
        ])
        .as_bytes(),
    );
    zip.add(
        "_rels/.rels",
        root_rels("word/document.xml", "officeDocument").as_bytes(),
    );
    zip.add("docProps/core.xml", core(&document.title).as_bytes());
    zip.add("word/document.xml", document_xml.as_bytes());
    zip.add("word/styles.xml", WORD_STYLES.as_bytes());
    zip.add("word/numbering.xml", word_numbering(ordered).as_bytes());
    zip.add("word/_rels/document.xml.rels", rels(&relations).as_bytes());
    for (name, bytes) in &media {
        zip.add(&format!("word/media/{name}"), bytes);
    }
    Ok(zip.finish())
}

/// `0` → `A`, `27` → `AB`.
pub fn column_name(mut index: usize) -> String {
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    name.reverse();
    String::from_utf8(name).unwrap_or_default()
}

/// Sheet names Excel accepts: at most 31 characters, none of `[]:*?/\`,
/// unique.
fn sheet_names(sheets: &[Sheet]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for sheet in sheets {
        let clean: String = sheet
            .name
            .chars()
            .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
            .take(31)
            .collect();
        let clean = if clean.trim().is_empty() {
            format!("Sheet{}", names.len() + 1)
        } else {
            clean
        };
        let mut name = clean.clone();
        let mut n = 2;
        while names.iter().any(|taken| taken.eq_ignore_ascii_case(&name)) {
            let suffix = format!(" ({n})");
            name = format!(
                "{}{suffix}",
                clean.chars().take(31 - suffix.len()).collect::<String>()
            );
            n += 1;
        }
        names.push(name);
    }
    names
}

const XLSX_STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts><fills count=\"3\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill><fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFE7E6E6\"/><bgColor indexed=\"64\"/></patternFill></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"2\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/><xf numFmtId=\"0\" fontId=\"1\" fillId=\"2\" borderId=\"0\" xfId=\"0\" applyFont=\"1\" applyFill=\"1\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>";

fn sheet_xml(sheet: &Sheet) -> String {
    // The first row is a header when all of it is text.
    let header = sheet.rows.len() > 1
        && sheet.rows[0].iter().all(|cell| {
            cell.is_string() && !cell.as_str().is_some_and(|text| text.starts_with('='))
        });
    let columns = sheet.rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![8_usize; columns];
    for row in &sheet.rows {
        for (index, cell) in row.iter().enumerate() {
            let length = match cell {
                Value::String(text) if !text.starts_with('=') => text.chars().count(),
                Value::Null => 0,
                other => other.to_string().len(),
            };
            widths[index] = widths[index].max(length.min(60) + 2);
        }
    }
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">",
    );
    if header {
        out.push_str("<sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews>");
    }
    if columns > 0 {
        out.push_str("<cols>");
        for (index, width) in widths.iter().enumerate() {
            let _ = write!(
                out,
                "<col min=\"{n}\" max=\"{n}\" width=\"{width}\" customWidth=\"1\"/>",
                n = index + 1
            );
        }
        out.push_str("</cols>");
    }
    out.push_str("<sheetData>");
    for (row_index, row) in sheet.rows.iter().enumerate() {
        let _ = write!(out, "<row r=\"{}\">", row_index + 1);
        let style = if header && row_index == 0 {
            " s=\"1\""
        } else {
            ""
        };
        for (column, cell) in row.iter().enumerate() {
            let reference = format!("{}{}", column_name(column), row_index + 1);
            match cell {
                Value::Null => {}
                Value::Bool(value) => {
                    let _ = write!(
                        out,
                        "<c r=\"{reference}\" t=\"b\"{style}><v>{}</v></c>",
                        u8::from(*value)
                    );
                }
                Value::Number(number) => {
                    let _ = write!(out, "<c r=\"{reference}\"{style}><v>{number}</v></c>");
                }
                Value::String(text) if text.starts_with('=') && text.len() > 1 => {
                    let _ = write!(
                        out,
                        "<c r=\"{reference}\"{style}><f>{}</f></c>",
                        xml(&text[1..])
                    );
                }
                other => {
                    let text = match other {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    let _ = write!(
                        out,
                        "<c r=\"{reference}\" t=\"inlineStr\"{style}><is><t xml:space=\"preserve\">{}</t></is></c>",
                        xml(&text)
                    );
                }
            }
        }
        out.push_str("</row>");
    }
    out.push_str("</sheetData>");
    if header && columns > 0 {
        let _ = write!(
            out,
            "<autoFilter ref=\"A1:{}{}\"/>",
            column_name(columns - 1),
            sheet.rows.len()
        );
    }
    out.push_str("</worksheet>");
    out
}

/// An Excel workbook: one worksheet per sheet; formulas are computed when
/// it is opened.
pub fn xlsx(sheets: &[Sheet]) -> Vec<u8> {
    let names = sheet_names(sheets);
    let mut workbook = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>",
    );
    let mut relations = Vec::new();
    let mut overrides = vec![
        (
            "/xl/workbook.xml".to_owned(),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
        ),
        (
            "/xl/styles.xml".to_owned(),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml",
        ),
    ];
    for (index, name) in names.iter().enumerate() {
        let n = index + 1;
        let _ = write!(
            workbook,
            "<sheet name=\"{}\" sheetId=\"{n}\" r:id=\"rId{n}\"/>",
            xml(name)
        );
        relations.push((
            format!("rId{n}"),
            "worksheet".to_owned(),
            format!("worksheets/sheet{n}.xml"),
        ));
        overrides.push((
            format!("/xl/worksheets/sheet{n}.xml"),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",
        ));
    }
    workbook.push_str("</sheets><calcPr calcId=\"191029\" fullCalcOnLoad=\"1\"/></workbook>");
    relations.push(("rIdStyles".into(), "styles".into(), "styles.xml".into()));
    let mut zip = Writer::default();
    zip.add("[Content_Types].xml", content_types(&overrides).as_bytes());
    zip.add(
        "_rels/.rels",
        root_rels("xl/workbook.xml", "officeDocument").as_bytes(),
    );
    zip.add(
        "docProps/core.xml",
        core(names.first().map_or("", String::as_str)).as_bytes(),
    );
    zip.add("xl/workbook.xml", workbook.as_bytes());
    zip.add("xl/_rels/workbook.xml.rels", rels(&relations).as_bytes());
    zip.add("xl/styles.xml", XLSX_STYLES.as_bytes());
    for (index, sheet) in sheets.iter().enumerate() {
        zip.add(
            &format!("xl/worksheets/sheet{}.xml", index + 1),
            sheet_xml(sheet).as_bytes(),
        );
    }
    zip.finish()
}

/// The images a document names, resolved, that don't exist.
pub fn missing_images(document: &Document, base: &Path) -> Vec<PathBuf> {
    document
        .blocks
        .iter()
        .chain(document.slides.iter().flat_map(|slide| slide.blocks.iter()))
        .filter_map(|block| match block {
            Block::Image { path, .. } => Some(out::resolve(path, base)),
            _ => None,
        })
        .filter(|path| !path.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_escapes_follow_the_formats_rules() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(27), "AB");
        assert_eq!(column_name(702), "AAA");
        assert_eq!(xml("a<b & \"c\"\u{1}"), "a&lt;b &amp; &quot;c&quot;");
        let sheet = |name: &str| Sheet {
            name: name.into(),
            rows: Vec::new(),
        };
        assert_eq!(
            sheet_names(&[
                sheet("Q1/Q2"),
                sheet("q1q2"),
                sheet(""),
                sheet(&"x".repeat(40))
            ]),
            ["Q1Q2", "q1q2 (2)", "Sheet3", &"x".repeat(31)]
        );
        let csv_text = csv(&Sheet {
            name: "s".into(),
            rows: vec![vec![
                json!("a,b"),
                json!(1.5),
                Value::Null,
                json!("say \"hi\""),
            ]],
        });
        assert_eq!(csv_text, "\"a,b\",1.5,,\"say \"\"hi\"\"\"\r\n");
    }
}
