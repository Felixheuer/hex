//! A document's structured content, as `hex_document_write` takes it: a
//! title, blocks (headings, paragraphs, lists, tables, images, quotes,
//! code, page breaks), and for spreadsheets and decks their sheets and
//! slides. The agent passes blocks as JSON or simply Markdown; the missing
//! parts are derived (a deck's slides from the headings, a workbook's
//! sheets from the tables).

use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    Bullets {
        items: Vec<String>,
        ordered: bool,
    },
    Table {
        rows: Vec<Vec<String>>,
        header: bool,
    },
    Image {
        path: String,
        width: Option<f64>,
    },
    Quote(String),
    Code(String),
    PageBreak,
}

/// A workbook's sheet: cells are text, numbers, booleans, or empty; text
/// starting with `=` is a formula.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Value>>,
}

/// A deck's slide.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slide {
    pub title: String,
    pub blocks: Vec<Block>,
    pub notes: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub title: String,
    pub blocks: Vec<Block>,
    pub sheets: Vec<Sheet>,
    pub slides: Vec<Slide>,
}

fn cell_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| items.iter().map(cell_text).collect())
        .unwrap_or_default()
}

fn rows(value: &Value) -> Vec<Vec<String>> {
    value
        .as_array()
        .map(|rows| rows.iter().map(strings).collect())
        .unwrap_or_default()
}

/// One block from its JSON (`{"type": "heading", "text": …, "level": 2}`, …).
fn block(value: &Value) -> Option<Block> {
    if let Some(text) = value.as_str() {
        return Some(Block::Paragraph(text.to_owned()));
    }
    let text = || value["text"].as_str().unwrap_or_default().to_owned();
    Some(match value["type"].as_str().unwrap_or("paragraph") {
        "heading" | "title" => Block::Heading {
            level: value["level"].as_u64().unwrap_or(1).clamp(1, 6) as u8,
            text: text(),
        },
        "bullets" | "list" => Block::Bullets {
            items: strings(&value["items"]),
            ordered: value["ordered"].as_bool().unwrap_or(false),
        },
        "table" => Block::Table {
            rows: rows(&value["rows"]),
            header: value["header"].as_bool().unwrap_or(true),
        },
        "image" => Block::Image {
            path: value["path"].as_str()?.to_owned(),
            width: value["width"].as_f64(),
        },
        "quote" => Block::Quote(text()),
        "code" => Block::Code(text()),
        "page_break" | "pagebreak" => Block::PageBreak,
        _ => Block::Paragraph(text()),
    })
}

/// The blocks in `value`: a JSON array of blocks, or Markdown text.
pub fn blocks(value: &Value) -> Vec<Block> {
    match value {
        Value::String(markdown) => from_markdown(markdown),
        Value::Array(items) => items.iter().filter_map(block).collect(),
        _ => Vec::new(),
    }
}

impl Document {
    /// The document `arguments` describe (`title`, `blocks` or `markdown`,
    /// `sheets`, `slides`).
    pub fn from_arguments(arguments: &Value) -> Self {
        let mut blocks = blocks(&arguments["blocks"]);
        if let Some(markdown) = arguments["markdown"].as_str() {
            blocks.extend(from_markdown(markdown));
        }
        let sheets = arguments["sheets"]
            .as_array()
            .map(|sheets| {
                sheets
                    .iter()
                    .enumerate()
                    .map(|(index, sheet)| Sheet {
                        name: sheet["name"]
                            .as_str()
                            .map_or_else(|| format!("Sheet{}", index + 1), str::to_owned),
                        rows: sheet["rows"]
                            .as_array()
                            .map(|rows| {
                                rows.iter()
                                    .map(|row| row.as_array().cloned().unwrap_or_default())
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let slides = arguments["slides"]
            .as_array()
            .map(|slides| {
                slides
                    .iter()
                    .map(|slide| {
                        let mut content = blocks_of(&slide["blocks"]);
                        let bullets = strings(&slide["bullets"]);
                        if !bullets.is_empty() {
                            content.push(Block::Bullets {
                                items: bullets,
                                ordered: false,
                            });
                        }
                        if let Some(text) = slide["text"].as_str() {
                            content.push(Block::Paragraph(text.to_owned()));
                        }
                        if !slide["table"].is_null() {
                            content.push(Block::Table {
                                rows: rows(&slide["table"]),
                                header: true,
                            });
                        }
                        if let Some(path) = slide["image"].as_str() {
                            content.push(Block::Image {
                                path: path.to_owned(),
                                width: None,
                            });
                        }
                        Slide {
                            title: slide["title"].as_str().unwrap_or_default().to_owned(),
                            blocks: content,
                            notes: slide["notes"].as_str().unwrap_or_default().to_owned(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            title: arguments["title"].as_str().unwrap_or_default().to_owned(),
            blocks,
            sheets,
            slides,
        }
    }

    /// Its sheets, else one per table in its blocks.
    pub fn sheets(&self) -> Vec<Sheet> {
        if !self.sheets.is_empty() {
            return self.sheets.clone();
        }
        let mut heading = String::new();
        let mut sheets = Vec::new();
        for block in &self.blocks {
            match block {
                Block::Heading { text, .. } => heading.clone_from(text),
                Block::Table { rows, .. } => sheets.push(Sheet {
                    name: if heading.is_empty() {
                        format!("Sheet{}", sheets.len() + 1)
                    } else {
                        heading.clone()
                    },
                    rows: rows
                        .iter()
                        .map(|row| row.iter().map(|cell| typed(cell)).collect())
                        .collect(),
                }),
                _ => {}
            }
        }
        sheets
    }

    /// Its slides, else one per top heading in its blocks (a title slide
    /// first when it has a title).
    pub fn slides(&self) -> Vec<Slide> {
        if !self.slides.is_empty() {
            return self.slides.clone();
        }
        let mut slides: Vec<Slide> = Vec::new();
        if !self.title.is_empty() {
            slides.push(Slide {
                title: self.title.clone(),
                ..Slide::default()
            });
        }
        let mut levels: Vec<u8> = self
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::Heading { level, .. } => Some(*level),
                _ => None,
            })
            .collect();
        levels.sort_unstable();
        // A single top heading is the document's own title: the level
        // below it makes the slides (and it a title slide).
        let top = match levels.as_slice() {
            [first, second, ..] if first != second => levels
                .iter()
                .copied()
                .find(|level| level > first)
                .unwrap_or(*first),
            [first, ..] => *first,
            [] => 1,
        };
        for block in &self.blocks {
            match block {
                Block::Heading { level, text } if *level <= top => {
                    slides.push(Slide {
                        title: text.clone(),
                        ..Slide::default()
                    });
                }
                Block::PageBreak => slides.push(Slide::default()),
                other => {
                    if slides.is_empty() {
                        slides.push(Slide::default());
                    }
                    if let Some(slide) = slides.last_mut() {
                        slide.blocks.push(other.clone());
                    }
                }
            }
        }
        slides
    }

    /// Its blocks as JSON, for the PDF bridge.
    pub fn blocks_json(&self) -> Value {
        Value::Array(self.blocks.iter().map(to_json).collect())
    }
}

fn blocks_of(value: &Value) -> Vec<Block> {
    if value.is_null() {
        Vec::new()
    } else {
        blocks(value)
    }
}

/// A cell's text as the value it means: a number when it reads as one.
pub fn typed(cell: &str) -> Value {
    let trimmed = cell.trim();
    if !trimmed.is_empty()
        && !(trimmed.len() > 1 && trimmed.starts_with('0') && !trimmed.starts_with("0."))
        && let Ok(number) = trimmed.parse::<f64>()
        && number.is_finite()
    {
        return match trimmed.parse::<i64>() {
            Ok(whole) => json!(whole),
            Err(_) => json!(number),
        };
    }
    Value::String(cell.to_owned())
}

/// `block` as JSON, with Markdown emphasis left out.
pub fn to_json(block: &Block) -> Value {
    match block {
        Block::Heading { level, text } => {
            json!({"type": "heading", "level": level, "text": plain(text)})
        }
        Block::Paragraph(text) => json!({"type": "paragraph", "text": plain(text)}),
        Block::Bullets { items, ordered } => json!({"type": "bullets", "ordered": ordered,
            "items": items.iter().map(|item| plain(item)).collect::<Vec<_>>()}),
        Block::Table { rows, header } => json!({"type": "table", "header": header,
            "rows": rows.iter().map(|row| row.iter().map(|cell| plain(cell)).collect::<Vec<_>>()).collect::<Vec<_>>()}),
        Block::Image { path, width } => json!({"type": "image", "path": path, "width": width}),
        Block::Quote(text) => json!({"type": "quote", "text": plain(text)}),
        Block::Code(text) => json!({"type": "code", "text": text}),
        Block::PageBreak => json!({"type": "page_break"}),
    }
}

/// `text` split into runs at `**bold**` markers: (text, bold).
pub fn runs(text: &str) -> Vec<(String, bool)> {
    text.split("**")
        .enumerate()
        .filter(|(_, part)| !part.is_empty())
        .map(|(index, part)| (part.to_owned(), index % 2 == 1))
        .collect()
}

/// `text` without `**` markers.
pub fn plain(text: &str) -> String {
    text.replace("**", "")
}

/// Markdown as blocks: headings, paragraphs, lists, pipe tables, images
/// on their own line, quotes, fenced code, and `\pagebreak`.
pub fn from_markdown(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut lines = markdown.lines().peekable();
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(paragraph.join(" ")));
            paragraph.clear();
        }
    };
    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if line.is_empty() {
            flush(&mut paragraph, &mut blocks);
            continue;
        }
        if line.starts_with("```") {
            flush(&mut paragraph, &mut blocks);
            let mut code = Vec::new();
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                code.push(inner);
            }
            blocks.push(Block::Code(code.join("\n")));
            continue;
        }
        if line == "\\pagebreak" || line == "<!-- pagebreak -->" {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::PageBreak);
            continue;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading {
                level: hashes as u8,
                text: line[hashes..].trim().to_owned(),
            });
            continue;
        }
        if let Some(item) = bullet(line) {
            flush(&mut paragraph, &mut blocks);
            let ordered = item.1;
            let mut items = vec![item.0.to_owned()];
            while let Some(next) = lines.peek().map(|next| next.trim())
                && let Some((text, same)) = bullet(next)
                && same == ordered
            {
                items.push(text.to_owned());
                lines.next();
            }
            blocks.push(Block::Bullets { items, ordered });
            continue;
        }
        if line.starts_with('|') {
            flush(&mut paragraph, &mut blocks);
            let mut table = vec![line];
            while let Some(next) = lines.peek().map(|next| next.trim())
                && next.starts_with('|')
            {
                table.push(next);
                lines.next();
            }
            let separator = |row: &str| row.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
            let header = table.get(1).is_some_and(|row| separator(row));
            let rows = table
                .iter()
                .filter(|row| !separator(row))
                .map(|row| {
                    row.trim_matches('|')
                        .split('|')
                        .map(|cell| cell.trim().to_owned())
                        .collect()
                })
                .collect();
            blocks.push(Block::Table { rows, header });
            continue;
        }
        if let Some(quote) = line.strip_prefix('>') {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Quote(quote.trim().to_owned()));
            continue;
        }
        if line.starts_with("![")
            && line.ends_with(')')
            && let Some(open) = line.find("](")
        {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Image {
                path: line[open + 2..line.len() - 1].trim().to_owned(),
                width: None,
            });
            continue;
        }
        if line.chars().all(|c| matches!(c, '-' | '*' | '_')) && line.len() >= 3 {
            flush(&mut paragraph, &mut blocks);
            continue;
        }
        paragraph.push(line);
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

/// A list item's text and whether it is numbered.
fn bullet(line: &str) -> Option<(&str, bool)> {
    for mark in ["- ", "* ", "+ ", "• "] {
        if let Some(rest) = line.strip_prefix(mark) {
            return Some((rest.trim(), false));
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &line[digits..];
        if let Some(text) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some((text.trim(), true));
        }
    }
    None
}

/// The blocks as Markdown.
pub fn to_markdown(document: &Document) -> String {
    let mut out = String::new();
    if !document.title.is_empty() {
        out.push_str(&format!("# {}\n\n", document.title));
    }
    let mut all = document.blocks.clone();
    if all.is_empty() {
        for slide in document.slides() {
            all.push(Block::Heading {
                level: 2,
                text: slide.title,
            });
            all.extend(slide.blocks);
        }
    }
    for block in &all {
        match block {
            Block::Heading { level, text } => {
                out.push_str(&format!("{} {text}\n\n", "#".repeat(*level as usize)));
            }
            Block::Paragraph(text) => out.push_str(&format!("{text}\n\n")),
            Block::Bullets { items, ordered } => {
                for (index, item) in items.iter().enumerate() {
                    if *ordered {
                        out.push_str(&format!("{}. {item}\n", index + 1));
                    } else {
                        out.push_str(&format!("- {item}\n"));
                    }
                }
                out.push('\n');
            }
            Block::Table { rows, header } => {
                for (index, row) in rows.iter().enumerate() {
                    let cells: Vec<String> =
                        row.iter().map(|cell| cell.replace('|', "\\|")).collect();
                    out.push_str(&format!("| {} |\n", cells.join(" | ")));
                    if index == 0 && *header {
                        out.push_str(&format!("|{}\n", " --- |".repeat(row.len().max(1))));
                    }
                }
                out.push('\n');
            }
            Block::Image { path, .. } => out.push_str(&format!("![]({path})\n\n")),
            Block::Quote(text) => out.push_str(&format!("> {text}\n\n")),
            Block::Code(text) => out.push_str(&format!("```\n{text}\n```\n\n")),
            Block::PageBreak => out.push_str("\\pagebreak\n\n"),
        }
    }
    for sheet in &document.sheets {
        out.push_str(&format!("## {}\n\n", sheet.name));
        for (index, row) in sheet.rows.iter().enumerate() {
            let cells: Vec<String> = row.iter().map(cell_text).collect();
            out.push_str(&format!("| {} |\n", cells.join(" | ")));
            if index == 0 {
                out.push_str(&format!("|{}\n", " --- |".repeat(row.len().max(1))));
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKDOWN: &str = "# Bericht\n\nErster Absatz,\nzweite Zeile.\n\n## Zahlen\n\n| Monat | Umsatz |\n|---|---:|\n| Jan | 120 |\n| Feb | 0815 |\n\n- eins\n- **zwei**\n\n1. a\n2. b\n\n> Zitat\n\n![Logo](logo.png)\n\n```\nlet x = 1;\n```\n\\pagebreak\n";

    #[test]
    fn markdown_becomes_blocks_and_back() {
        let blocks = from_markdown(MARKDOWN);
        assert_eq!(
            blocks[0],
            Block::Heading {
                level: 1,
                text: "Bericht".into()
            }
        );
        assert_eq!(
            blocks[1],
            Block::Paragraph("Erster Absatz, zweite Zeile.".into())
        );
        assert_eq!(
            blocks[3],
            Block::Table {
                rows: vec![
                    vec!["Monat".into(), "Umsatz".into()],
                    vec!["Jan".into(), "120".into()],
                    vec!["Feb".into(), "0815".into()]
                ],
                header: true
            }
        );
        assert_eq!(
            blocks[4],
            Block::Bullets {
                items: vec!["eins".into(), "**zwei**".into()],
                ordered: false
            }
        );
        assert!(matches!(&blocks[5], Block::Bullets { ordered: true, items } if items.len() == 2));
        assert_eq!(blocks[6], Block::Quote("Zitat".into()));
        assert!(matches!(&blocks[7], Block::Image { path, .. } if path == "logo.png"));
        assert_eq!(blocks[8], Block::Code("let x = 1;".into()));
        assert_eq!(blocks[9], Block::PageBreak);
        assert_eq!(
            runs("a **b** c"),
            [
                ("a ".into(), false),
                ("b".into(), true),
                (" c".into(), false)
            ]
        );
        let document = Document {
            blocks: blocks.clone(),
            ..Document::default()
        };
        assert_eq!(from_markdown(&to_markdown(&document)), blocks);
    }

    #[test]
    fn sheets_and_slides_are_derived_when_not_given() {
        let document = Document::from_arguments(&json!({"title": "Deck", "markdown": MARKDOWN}));
        let sheets = document.sheets();
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].name, "Zahlen");
        assert_eq!(sheets[0].rows[1], [json!("Jan"), json!(120)]);
        assert_eq!(
            sheets[0].rows[2][1],
            json!("0815"),
            "a leading zero stays text"
        );
        let slides = document.slides();
        assert_eq!(slides[0].title, "Deck");
        assert_eq!(slides[1].title, "Bericht");
        let given = Document::from_arguments(&json!({"slides": [
            {"title": "A", "bullets": ["x", "y"], "notes": "say this"}
        ], "sheets": [{"rows": [["a", 1, true, null]]}]}));
        assert_eq!(given.slides()[0].notes, "say this");
        assert!(
            matches!(&given.slides()[0].blocks[0], Block::Bullets { items, .. } if items.len() == 2)
        );
        assert_eq!(given.sheets()[0].name, "Sheet1");
        assert_eq!(given.sheets()[0].rows[0].len(), 4);
        assert_eq!(blocks(&json!(["plain", {"type": "page_break"}])).len(), 2);
    }
}
