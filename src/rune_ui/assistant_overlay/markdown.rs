//! Just enough Markdown for a quick answer in the bubble: paragraphs,
//! headings (as bold lines), bulleted and numbered lists, fenced code,
//! **bold**, *italic*, `inline code`, and links (their text). Anything else
//! reads as the text it is. The kit's full Markdown isn't in Agent-Desk's build;
//! answers to a spoken question are short and plain anyway.

use rune::{El, Span, col, el, rich, row, text};

use super::super::controls::Paint;

/// One block of an answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Block {
    Paragraph(Inline),
    Heading(Inline),
    /// A list item: its marker ("•", "1.") and text.
    Item(String, Inline),
    Code(String),
}

/// Text with its marks: byte ranges into `text`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Inline {
    pub(crate) text: String,
    pub(crate) marks: Vec<(usize, usize, Mark)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mark {
    Bold,
    Italic,
    Code,
}

/// `source`'s blocks.
pub(crate) fn blocks(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(inline(&paragraph.join(" "))));
            paragraph.clear();
        }
    };
    for raw in source.lines() {
        let line = raw.trim();
        if let Some(lines) = code.as_mut() {
            if line.starts_with("```") {
                blocks.push(Block::Code(lines.join("\n")));
                code = None;
            } else {
                lines.push(raw.trim_end());
            }
            continue;
        }
        if line.starts_with("```") {
            flush(&mut paragraph, &mut blocks);
            code = Some(Vec::new());
            continue;
        }
        if line.is_empty() {
            flush(&mut paragraph, &mut blocks);
            continue;
        }
        if line.starts_with('#') {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading(inline(line.trim_start_matches('#').trim())));
            continue;
        }
        if let Some(rest) = ["- ", "* ", "+ "]
            .iter()
            .find_map(|bullet| line.strip_prefix(bullet))
        {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Item("•".into(), inline(rest.trim())));
            continue;
        }
        if let Some((number, rest)) = line.split_once(". ")
            && !number.is_empty()
            && number.len() <= 3
            && number.chars().all(|c| c.is_ascii_digit())
        {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Item(format!("{number}."), inline(rest.trim())));
            continue;
        }
        paragraph.push(line.trim_start_matches('>').trim());
    }
    // A fence still open (a streaming answer) shows what it has.
    if let Some(lines) = code {
        blocks.push(Block::Code(lines.join("\n")));
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

/// `source`'s inline marks taken out of its text.
pub(crate) fn inline(source: &str) -> Inline {
    let mut out = Inline::default();
    let mut open: Vec<(Mark, usize, &str)> = Vec::new();
    let mut rest = source;
    while !rest.is_empty() {
        // Inside code, only its closing backtick counts.
        let in_code = open.last().is_some_and(|(mark, ..)| *mark == Mark::Code);
        let markers: &[(&str, Mark)] = if in_code {
            &[("`", Mark::Code)]
        } else {
            &[
                ("**", Mark::Bold),
                ("__", Mark::Bold),
                ("`", Mark::Code),
                ("*", Mark::Italic),
                ("_", Mark::Italic),
            ]
        };
        if !in_code
            && rest.starts_with('[')
            && let Some(close) = rest.find("](")
            && let Some(end) = rest[close..].find(')').map(|at| close + at)
        {
            out.text.push_str(&rest[1..close]);
            rest = &rest[end + 1..];
            continue;
        }
        if let Some((marker, mark)) = markers.iter().find(|(m, _)| rest.starts_with(m)) {
            let closes = open.iter().rposition(|(_, _, m)| m == marker);
            match closes {
                Some(index) => {
                    let (mark, start, _) = open.remove(index);
                    if out.text.len() > start {
                        out.marks.push((start, out.text.len(), mark));
                    }
                }
                // `_` inside a word (snake_case) is a letter, not a mark.
                None if *marker == "_"
                    && out
                        .text
                        .chars()
                        .next_back()
                        .is_some_and(char::is_alphanumeric) =>
                {
                    out.text.push('_');
                }
                None => open.push((*mark, out.text.len(), marker)),
            }
            rest = &rest[marker.len()..];
            continue;
        }
        let next = rest.chars().next().expect("not empty");
        out.text.push(next);
        rest = &rest[next.len_utf8()..];
    }
    out.marks.sort_by_key(|(start, ..)| *start);
    out
}

/// The answer drawn: its blocks one under another.
pub(crate) fn view(p: Paint, source: &str) -> El {
    col().key("markdown").w_full().gap(6.0).children(
        blocks(source)
            .into_iter()
            .enumerate()
            .map(|(index, block)| block_view(p, block, index)),
    )
}

fn block_view(p: Paint, block: Block, index: usize) -> El {
    let key = format!("b{index}");
    match block {
        Block::Paragraph(text) => line(p, text, false).key(key),
        Block::Heading(text) => line(p, text, true).key(key),
        Block::Item(marker, text) => row()
            .key(key)
            .w_full()
            .gap(6.0)
            .items_start()
            .child(text_marker(p, marker))
            .child(line(p, text, false).grow(1.0).min_w(0.0)),
        Block::Code(code) => el()
            .key(key)
            .w_full()
            .px(8.0)
            .py(6.0)
            .radius(p.k.radius.sm)
            .fill(p.k.c.surface_raised_base)
            .child(text(code).font(p.k.ty.mono).color(p.text()).wrap()),
    }
}

fn text_marker(p: Paint, marker: String) -> El {
    text(marker)
        .key("marker")
        .min_w(12.0)
        .font(p.k.ty.ui)
        .color(p.muted())
}

/// A wrapping line in the answer's type, with its marks; all bold when
/// `strong` (a heading).
fn line(p: Paint, inline: Inline, strong: bool) -> El {
    let mut spans: Vec<Span> = Vec::new();
    if strong {
        spans.push(Span::new(0, inline.text.len()).bold());
    }
    for (start, end, mark) in &inline.marks {
        let span = Span::new(*start, *end);
        spans.push(match mark {
            Mark::Bold => span.bold(),
            Mark::Italic => Span {
                italic: true,
                ..span
            },
            Mark::Code => Span {
                mono: true,
                bg: Some(p.k.c.surface_raised_base),
                bg_pad: 2.0,
                ..span
            },
        });
    }
    rich(inline.text, spans)
        .font(p.k.ty.ui)
        .color(p.text())
        .w_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_marks_come_out_of_the_text() {
        let line = inline("Run **cargo** with `--locked`, *now*. See [docs](https://x.y).");
        assert_eq!(line.text, "Run cargo with --locked, now. See docs.");
        assert_eq!(
            line.marks,
            [
                (4, 9, Mark::Bold),
                (15, 23, Mark::Code),
                (25, 28, Mark::Italic)
            ]
        );
        let snake = inline("call my_function_name and `a*b*c`");
        assert_eq!(snake.text, "call my_function_name and a*b*c");
        assert_eq!(snake.marks, [(26, 31, Mark::Code)]);
    }

    #[test]
    fn an_unclosed_mark_marks_nothing() {
        let line = inline("2 * 3 is **six");
        assert_eq!(line.text, "2  3 is six");
        assert!(line.marks.is_empty());
    }

    #[test]
    fn blocks_are_paragraphs_headings_items_and_code() {
        let answer =
            "## Plan\nFirst we\ncheck.\n\n- one\n* two\n3. three\n\n```\nlet x = 1;\n```\nDone.";
        let blocks = blocks(answer);
        assert_eq!(blocks.len(), 7);
        assert_eq!(blocks[0], Block::Heading(inline("Plan")));
        assert_eq!(blocks[1], Block::Paragraph(inline("First we check.")));
        assert_eq!(blocks[2], Block::Item("•".into(), inline("one")));
        assert_eq!(blocks[4], Block::Item("3.".into(), inline("three")));
        assert_eq!(blocks[5], Block::Code("let x = 1;".into()));
        assert_eq!(blocks[6], Block::Paragraph(inline("Done.")));
        // A fence still streaming shows what it has.
        assert_eq!(blocks_of("```\nfn"), [Block::Code("fn".into())]);
    }

    fn blocks_of(source: &str) -> Vec<Block> {
        blocks(source)
    }
}
