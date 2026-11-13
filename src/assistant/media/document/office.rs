//! Office Open XML read without Office: a Word document's paragraphs (its
//! headings and list items marked as in Markdown) and tables, a workbook's
//! sheets as rows of values, a deck's slides (title, text, tables, speaker
//! notes) in their order. Embedded pictures can be copied out.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use roxmltree::{Document as Xml, Node};
use serde_json::{Value, json};

use super::read::{Page, Read, Table};
use crate::assistant::media::zip::Archive;

/// The most rows read of one sheet, and columns of one row.
const MAX_ROWS: usize = 5000;
const MAX_COLUMNS: usize = 200;

fn parse(text: &str) -> Result<Xml<'_>, String> {
    Xml::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..roxmltree::ParsingOptions::default()
        },
    )
    .map_err(|error| format!("unreadable XML: {error}"))
}

fn named<'a, 'input>(node: Node<'a, 'input>, name: &str) -> impl Iterator<Item = Node<'a, 'input>> {
    node.children()
        .filter(move |child| child.is_element() && child.tag_name().name() == name)
}

fn attribute<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|attribute| attribute.name() == name)
        .map(|attribute| attribute.value())
}

/// A paragraph's text (`w:p` or `a:p`): its runs, tabs, and breaks.
fn paragraph_text(node: Node) -> String {
    let mut text = String::new();
    for part in node.descendants().filter(Node::is_element) {
        match part.tag_name().name() {
            "t" => text.push_str(part.text().unwrap_or_default()),
            "tab" => text.push('\t'),
            "br" | "cr" => text.push('\n'),
            _ => {}
        }
    }
    text
}

/// A part's relationships: id → (type, target part path).
fn relationships(archive: &Archive, part: &str) -> HashMap<String, (String, String)> {
    let (folder, file) = part.rsplit_once('/').unwrap_or(("", part));
    let rels = if folder.is_empty() {
        format!("_rels/{file}.rels")
    } else {
        format!("{folder}/_rels/{file}.rels")
    };
    let Some(text) = archive.text(&rels) else {
        return HashMap::new();
    };
    let Ok(xml) = parse(&text) else {
        return HashMap::new();
    };
    xml.root_element()
        .children()
        .filter(Node::is_element)
        .filter_map(|rel| {
            let id = attribute(rel, "Id")?;
            let kind = attribute(rel, "Type").unwrap_or_default();
            let target = attribute(rel, "Target")?;
            Some((id.to_owned(), (kind.to_owned(), join_part(folder, target))))
        })
        .collect()
}

/// `target` relative to the part folder `folder` (or absolute from `/`).
pub fn join_part(folder: &str, target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        return absolute.to_owned();
    }
    let mut parts: Vec<&str> = folder.split('/').filter(|part| !part.is_empty()).collect();
    for piece in target.split('/') {
        match piece {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            piece => parts.push(piece),
        }
    }
    parts.join("/")
}

/// Copies the archive's pictures under `prefix` (`word/media/`) into
/// `into`, at most `max`.
pub fn pictures(archive: &Archive, prefix: &str, into: &Path, max: usize) -> Vec<PathBuf> {
    let names: Vec<String> = archive
        .names()
        .filter(|name| name.starts_with(prefix))
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            [
                ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".tif", ".tiff",
            ]
            .iter()
            .any(|extension| lower.ends_with(extension))
        })
        .map(str::to_owned)
        .take(max)
        .collect();
    let _ = std::fs::create_dir_all(into);
    names
        .iter()
        .filter_map(|name| {
            let bytes = archive.read(name)?;
            let file = into.join(name.rsplit('/').next()?);
            std::fs::write(&file, bytes).ok()?;
            Some(file)
        })
        .collect()
}

/// A Word document: its text (headings as `#`, list items as `- `, each
/// table as `[Table n]`) and tables.
pub fn docx(archive: &Archive) -> Result<Read, String> {
    let text = archive
        .text("word/document.xml")
        .ok_or("not a Word document (no word/document.xml)")?;
    let xml = parse(&text)?;
    let body = xml
        .descendants()
        .find(|node| node.tag_name().name() == "body")
        .ok_or("the Word document has no body")?;
    let mut lines = Vec::new();
    let mut tables = Vec::new();
    for node in body.children().filter(Node::is_element) {
        match node.tag_name().name() {
            "p" => lines.push(word_paragraph(node)),
            "tbl" => {
                tables.push(Table {
                    name: format!("Table {}", tables.len() + 1),
                    rows: word_rows(node),
                });
                lines.push(format!("[Table {}]", tables.len()));
            }
            "sdt" => {
                for paragraph in node
                    .descendants()
                    .filter(|inner| inner.tag_name().name() == "p")
                {
                    lines.push(word_paragraph(paragraph));
                }
            }
            _ => {}
        }
    }
    let text = squeeze(&lines.join("\n"));
    Ok(Read {
        format: "docx",
        pages: vec![Page {
            n: 1,
            text,
            ocr: false,
        }],
        tables,
        ..Read::default()
    })
}

fn word_paragraph(node: Node) -> String {
    let text = paragraph_text(node);
    let properties = named(node, "pPr").next();
    let style = properties
        .and_then(|properties| named(properties, "pStyle").next())
        .and_then(|style| attribute(style, "val"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let listed = properties.is_some_and(|properties| named(properties, "numPr").next().is_some());
    if text.trim().is_empty() {
        return String::new();
    }
    if style == "title" {
        return format!("# {text}");
    }
    // "Heading2", and localized ids such as German "berschrift2".
    if style.contains("heading") || style.contains("berschrift") || style.contains("titre") {
        let level = style
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse::<usize>()
            .unwrap_or(1)
            .clamp(1, 6);
        return format!("{} {text}", "#".repeat(level));
    }
    if listed || style.contains("list") {
        return format!("- {text}");
    }
    text
}

fn word_rows(table: Node) -> Vec<Vec<String>> {
    named(table, "tr")
        .map(|row| {
            named(row, "tc")
                .map(|cell| {
                    cell.descendants()
                        .filter(|node| node.tag_name().name() == "p")
                        .map(paragraph_text)
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim()
                        .to_owned()
                })
                .collect()
        })
        .collect()
}

/// Blank lines collapsed to one.
fn squeeze(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_owned()
}

/// A deck: one page per slide (`pages` picks them), its title as `#`, its
/// text, tables, and speaker notes.
pub fn pptx(archive: &Archive, wanted: &dyn Fn(usize) -> bool) -> Result<Read, String> {
    let presentation = archive
        .text("ppt/presentation.xml")
        .ok_or("not a PowerPoint deck (no ppt/presentation.xml)")?;
    let xml = parse(&presentation)?;
    let rels = relationships(archive, "ppt/presentation.xml");
    let slides: Vec<String> = xml
        .descendants()
        .filter(|node| node.tag_name().name() == "sldId")
        .filter_map(|node| {
            let id = node
                .attributes()
                .find(|attribute| attribute.name() == "id" && attribute.namespace().is_some())?
                .value();
            rels.get(id).map(|(_, target)| target.clone())
        })
        .collect();
    let mut read = Read {
        format: "pptx",
        meta: json!({"slides": slides.len()}),
        ..Read::default()
    };
    for (index, part) in slides.iter().enumerate() {
        let n = index + 1;
        if !wanted(n) {
            continue;
        }
        let Some(text) = archive.text(part) else {
            continue;
        };
        let xml = parse(&text)?;
        let mut lines = Vec::new();
        for shape in xml
            .descendants()
            .filter(|node| matches!(node.tag_name().name(), "sp" | "graphicFrame"))
        {
            let title = shape
                .descendants()
                .find(|node| node.tag_name().name() == "ph")
                .and_then(|ph| attribute(ph, "type"))
                .is_some_and(|kind| kind == "title" || kind == "ctrTitle");
            if shape.tag_name().name() == "graphicFrame" {
                if let Some(table) = shape
                    .descendants()
                    .find(|node| node.tag_name().name() == "tbl")
                {
                    let rows = named(table, "tr")
                        .map(|row| {
                            named(row, "tc")
                                .map(|cell| {
                                    cell.descendants()
                                        .filter(|node| node.tag_name().name() == "p")
                                        .map(paragraph_text)
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                        .trim()
                                        .to_owned()
                                })
                                .collect()
                        })
                        .collect();
                    read.tables.push(Table {
                        name: format!("Slide {n} table"),
                        rows,
                    });
                    lines.push(format!("[Table {}]", read.tables.len()));
                }
                continue;
            }
            for paragraph in shape
                .descendants()
                .filter(|node| node.tag_name().name() == "p")
            {
                let text = paragraph_text(paragraph);
                if text.trim().is_empty() {
                    continue;
                }
                lines.push(if title { format!("# {text}") } else { text });
            }
        }
        let notes = relationships(archive, part)
            .values()
            .find(|(kind, _)| kind.ends_with("/notesSlide"))
            .and_then(|(_, target)| archive.text(target))
            .and_then(|text| {
                let xml = parse(&text).ok()?;
                let said: Vec<String> = xml
                    .descendants()
                    .filter(|node| node.tag_name().name() == "sp")
                    .filter(|shape| {
                        shape
                            .descendants()
                            .find(|node| node.tag_name().name() == "ph")
                            .and_then(|ph| attribute(ph, "type"))
                            == Some("body")
                    })
                    .flat_map(|shape| {
                        shape
                            .descendants()
                            .filter(|node| node.tag_name().name() == "p")
                            .map(paragraph_text)
                            .collect::<Vec<_>>()
                    })
                    .filter(|line| !line.trim().is_empty())
                    .collect();
                (!said.is_empty()).then(|| said.join("\n"))
            });
        if let Some(notes) = notes {
            lines.push(format!("Notes: {notes}"));
        }
        read.pages.push(Page {
            n,
            text: lines.join("\n"),
            ocr: false,
        });
    }
    Ok(read)
}

/// A workbook: each sheet (or the one `sheet` names) as rows of values,
/// empty cells as null, at most [`MAX_ROWS`] rows.
pub fn xlsx(archive: &Archive, sheet: Option<&str>) -> Result<Read, String> {
    let workbook = archive
        .text("xl/workbook.xml")
        .ok_or("not an Excel workbook (no xl/workbook.xml)")?;
    let xml = parse(&workbook)?;
    let rels = relationships(archive, "xl/workbook.xml");
    let shared: Vec<String> = archive
        .text("xl/sharedStrings.xml")
        .and_then(|text| {
            let xml = parse(&text).ok()?;
            Some(
                xml.root_element()
                    .children()
                    .filter(|node| node.tag_name().name() == "si")
                    .map(|item| {
                        item.descendants()
                            .filter(|node| {
                                node.tag_name().name() == "t"
                                    && node.parent().is_some_and(|parent| {
                                        matches!(parent.tag_name().name(), "si" | "r")
                                    })
                            })
                            .filter_map(|node| node.text())
                            .collect::<String>()
                    })
                    .collect(),
            )
        })
        .unwrap_or_default();
    let mut read = Read {
        format: "xlsx",
        ..Read::default()
    };
    let mut names = Vec::new();
    for entry in xml
        .descendants()
        .filter(|node| node.tag_name().name() == "sheet")
    {
        let name = attribute(entry, "name").unwrap_or("Sheet").to_owned();
        names.push(name.clone());
        if sheet.is_some_and(|wanted| !wanted.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let Some(part) = entry
            .attributes()
            .find(|attribute| attribute.name() == "id" && attribute.namespace().is_some())
            .and_then(|id| rels.get(id.value()))
            .map(|(_, target)| target.clone())
        else {
            continue;
        };
        let Some(text) = archive.text(&part) else {
            continue;
        };
        let (rows, cut) = sheet_rows(&parse(&text)?, &shared);
        read.cut |= cut;
        read.sheets.push((name, rows));
    }
    read.meta = json!({"sheets": names});
    if let Some(wanted) = sheet
        && read.sheets.is_empty()
    {
        return Err(format!(
            "The workbook has no sheet “{wanted}”; it has {names:?}."
        ));
    }
    Ok(read)
}

fn sheet_rows(xml: &Xml, shared: &[String]) -> (Vec<Vec<Value>>, bool) {
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut cut = false;
    for row in xml
        .descendants()
        .filter(|node| node.tag_name().name() == "row")
    {
        let index = attribute(row, "r")
            .and_then(|r| r.parse::<usize>().ok())
            .map_or(rows.len(), |r| r.saturating_sub(1));
        if index >= MAX_ROWS {
            cut = true;
            break;
        }
        let mut cells: Vec<Value> = Vec::new();
        for cell in named(row, "c") {
            let column = attribute(cell, "r").map_or(cells.len(), column_index);
            if column >= MAX_COLUMNS {
                continue;
            }
            let raw = named(cell, "v").next().and_then(|v| v.text());
            let value = match attribute(cell, "t") {
                Some("s") => raw
                    .and_then(|index| index.parse::<usize>().ok())
                    .and_then(|index| shared.get(index))
                    .map_or(Value::Null, |text| json!(text)),
                Some("inlineStr") => json!(
                    cell.descendants()
                        .filter(|node| node.tag_name().name() == "t")
                        .filter_map(|node| node.text())
                        .collect::<String>()
                ),
                Some("b") => json!(raw == Some("1")),
                Some("str") | Some("e") => raw.map_or(Value::Null, |text| json!(text)),
                _ => raw.map_or(Value::Null, |text| {
                    text.parse::<f64>()
                        .ok()
                        .and_then(|number| {
                            if number.fract() == 0.0 && number.abs() < 1e15 {
                                Some(json!(number as i64))
                            } else {
                                serde_json::Number::from_f64(number).map(Value::Number)
                            }
                        })
                        .unwrap_or_else(|| json!(text))
                }),
            };
            if cells.len() <= column {
                cells.resize(column + 1, Value::Null);
            }
            cells[column] = value;
        }
        if rows.len() <= index {
            rows.resize(index + 1, Vec::new());
        }
        rows[index] = cells;
    }
    while rows.last().is_some_and(Vec::is_empty) {
        rows.pop();
    }
    (rows, cut)
}

/// `B12` → 1 (the column, from 0).
pub fn column_index(reference: &str) -> usize {
    reference
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .fold(0, |sum, c| {
            sum * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1)
        })
        .saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_and_columns_are_found() {
        assert_eq!(
            join_part("ppt", "slides/slide1.xml"),
            "ppt/slides/slide1.xml"
        );
        assert_eq!(
            join_part("ppt/slides", "../notesSlides/n1.xml"),
            "ppt/notesSlides/n1.xml"
        );
        assert_eq!(
            join_part("xl", "/xl/worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(column_index("A1"), 0);
        assert_eq!(column_index("Z9"), 25);
        assert_eq!(column_index("AA10"), 26);
        assert_eq!(squeeze("a\n\n\n\nb\n"), "a\n\nb");
    }
}
