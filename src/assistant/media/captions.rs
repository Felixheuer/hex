//! A video's own captions (WebVTT or SubRip, as yt-dlp saves them) as a
//! timed transcript: styling and inline timings dropped, and the rolling
//! repeats of automatic captions (each cue repeats the line before it)
//! said once.

use serde::Serialize;

/// One stretch of what is said.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Line {
    pub at_s: f64,
    pub text: String,
}

/// The cues of a `.vtt` or `.srt` file, repeats said once.
pub fn parse(text: &str) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut at: Option<f64> = None;
    let mut last = String::new();
    for raw in text.lines() {
        let raw = raw.trim();
        if let Some((start, _)) = raw.split_once("-->") {
            at = timestamp(start.trim());
            continue;
        }
        // A blank line ends the cue (SubRip's next number isn't said).
        if raw.is_empty() {
            at = None;
            continue;
        }
        let Some(start) = at else {
            continue;
        };
        let clean = clean(raw);
        if clean.is_empty() || clean == last {
            continue;
        }
        // A rolling caption starts with the line before it: keep the rest.
        let new = clean
            .strip_prefix(last.as_str())
            .map(str::trim)
            .filter(|rest| !last.is_empty() && !rest.is_empty())
            .unwrap_or(&clean)
            .to_owned();
        last = clean;
        if lines.last().is_some_and(|line| line.text.ends_with(&new)) {
            continue;
        }
        lines.push(Line {
            at_s: start,
            text: new,
        });
    }
    lines
}

/// `00:01:02.500`, `01:02.500`, or SubRip's `00:01:02,500` as seconds.
fn timestamp(text: &str) -> Option<f64> {
    let text = text.split_whitespace().next()?.replace(',', ".");
    super::probe::clock_seconds(&text)
}

/// A cue line without tags (`<c>`, `<00:00:01.000>`, `<i>`) or entities.
fn clean(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_tag = false;
    for c in line.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lines joined into paragraphs of about `every` seconds, each `[m:ss]`
/// first, at most `max_chars` in all.
pub fn paragraphs(lines: &[Line], every: f64, max_chars: usize) -> (String, bool) {
    let mut out = String::new();
    let mut started: Option<f64> = None;
    for line in lines {
        if started.is_none_or(|start| line.at_s - start >= every) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[{}] ", super::frames::clock(line.at_s)));
            started = Some(line.at_s);
        } else {
            out.push(' ');
        }
        out.push_str(&line.text);
        if out.chars().count() > max_chars {
            let cut: String = out.chars().take(max_chars).collect();
            return (format!("{cut}…"), true);
        }
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_automatic_captions_are_said_once() {
        let vtt = "WEBVTT\nKind: captions\nLanguage: en\n\n\
            00:00:00.000 --> 00:00:02.000 align:start position:0%\n\
            hello<00:00:00.500><c> and</c><00:00:01.000><c> welcome</c>\n\n\
            00:00:02.000 --> 00:00:02.010 align:start position:0%\n\
            hello and welcome\n\n\
            00:00:02.010 --> 00:00:04.000 align:start position:0%\n\
            hello and welcome\n\
            to<00:00:02.500><c> the</c><c> show</c>\n\n\
            00:00:04.000 --> 00:00:06.000\n\
            to the show\n\
            today &amp; tomorrow\n";
        let lines = parse(vtt);
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            texts,
            ["hello and welcome", "to the show", "today & tomorrow"]
        );
        assert_eq!(lines[1].at_s, 2.01);
        let srt = "1\n00:00:01,500 --> 00:00:03,000\n<i>Guten Tag</i>\n\n2\n00:01:05,000 --> 00:01:07,000\nWie geht's?\n";
        let lines = parse(srt);
        assert_eq!(lines[0].at_s, 1.5);
        assert_eq!(lines[1].text, "Wie geht's?");
        let (text, cut) = paragraphs(&lines, 30.0, 1000);
        assert_eq!(text, "[0:02] Guten Tag\n[1:05] Wie geht's?");
        assert!(!cut);
        let (short, cut) = paragraphs(&lines, 30.0, 10);
        assert!(cut && short.ends_with('…'));
    }
}
