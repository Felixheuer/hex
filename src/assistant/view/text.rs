//! A task's words as a surface shows them on one line: titles, steps, and
//! the first line of an answer or error.

/// The longest a title, step, or outcome line runs before "…".
pub const LINE_CHARS: usize = 72;

/// `text` as one line of at most [`LINE_CHARS`] characters: its first
/// non-empty line, whitespace collapsed, cut at a word, with "…" when
/// anything was left out.
pub fn shorten(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let line = lines.next().unwrap_or_default();
    clip(line, lines.next().is_some(), LINE_CHARS)
}

/// The first non-empty line of an answer or error as plain text (its
/// Markdown marks gone), shortened.
pub fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(plain)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let more = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .nth(1)
        .is_some();
    clip(&line, more, LINE_CHARS)
}

/// One Markdown line as a person reads it: no heading, list, or quote
/// marker, no bold, italic, or code marks, and a link's text without its
/// address.
pub fn plain(line: &str) -> String {
    let mut line = line.trim();
    line = line.trim_start_matches('#').trim_start();
    line = line.trim_start_matches('>').trim_start();
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            line = rest;
        }
    }
    if let Some((number, rest)) = line.split_once(". ")
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
    {
        line = rest;
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    // Links: `[text](address)` reads as `text`.
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|at| open + at) else {
            break;
        };
        let Some(end) = rest[close..].find(')').map(|at| close + at) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out.replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .replace(['*'], "")
        .trim()
        .to_owned()
}

fn clip(line: &str, more: bool, max: usize) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    let joined = words.join(" ");
    if joined.chars().count() <= max && !more {
        return joined;
    }
    let mut out = String::new();
    for word in &words {
        let next = if out.is_empty() {
            word.chars().count()
        } else {
            out.chars().count() + 1 + word.chars().count()
        };
        if next > max - 1 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() {
        // One word longer than the line.
        out = joined.chars().take(max - 1).collect();
    }
    let out = out.trim_end_matches([',', ';', ':', '.', ' ']);
    format!("{out}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_line_reads_without_its_markdown() {
        assert_eq!(first_line("## **Booked** a table"), "Booked a table");
        assert_eq!(
            first_line("- Run `cargo build`\n- then test"),
            "Run cargo build…"
        );
        assert_eq!(
            first_line("See [the menu](https://nopa.com) *tonight*"),
            "See the menu tonight"
        );
        assert_eq!(first_line("1. First step"), "First step");
        assert_eq!(plain("> quoted"), "quoted");
    }

    #[test]
    fn titles_and_lines_fit_one_line() {
        assert_eq!(shorten("  Open   Mail  "), "Open Mail");
        assert_eq!(shorten("First line\nsecond"), "First line…");
        let long = "Find the cheapest flight from Berlin to Lisbon next weekend and put it in my calendar please";
        let short = shorten(long);
        assert!(short.chars().count() <= LINE_CHARS, "{short}");
        assert!(short.ends_with('…'));
        assert!(long.starts_with(short.trim_end_matches('…')));
        assert_eq!(first_line("\n\n  Done.  \nMore"), "Done…");
        assert_eq!(first_line("All set"), "All set");
        let word = "x".repeat(200);
        assert_eq!(shorten(&word).chars().count(), LINE_CHARS);
    }
}
