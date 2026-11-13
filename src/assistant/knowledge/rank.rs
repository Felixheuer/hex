//! Matching and ranking shared by every scope: the query's words, folded
//! (case, accents, umlauts), scored against a result's text, plus a
//! recency bonus; and how a result reads in the tool's answer.

/// One result of a lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// What it is: a file's name, a person, a mail's subject.
    pub title: String,
    /// Where or how to reach it: a path, numbers, sender, URL.
    pub detail: String,
    /// When it was last used, sent, visited, or when it starts (Unix ms).
    pub when_ms: Option<i64>,
    pub score: f64,
}

/// `text` lowercased with accents and umlauts folded ("Müller" →
/// "muller", "Straße" → "strasse"), so "mueller", "muller" and "Müller"
/// meet: "ue", "oe", "ae" fold too.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' | 'á' | 'à' | 'â' | 'ã' | 'å' => out.push('a'),
            'ö' | 'ó' | 'ò' | 'ô' | 'õ' | 'ø' => out.push('o'),
            'ü' | 'ú' | 'ù' | 'û' => out.push('u'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'ç' => out.push('c'),
            'ñ' => out.push('n'),
            'ß' => out.push_str("ss"),
            _ => out.push(c),
        }
    }
    out.replace("ae", "a").replace("oe", "o").replace("ue", "u")
}

/// The query's words, folded; punctuation around them dropped.
pub fn tokens(query: &str) -> Vec<String> {
    let mut words: Vec<String> = split(query).map(fold).collect();
    words.dedup();
    words
}

/// The query's words lowercased but not folded, for a database's own
/// `LIKE` (which compares accents as they are).
pub fn words(query: &str) -> Vec<String> {
    let mut words: Vec<String> = split(query).map(str::to_lowercase).collect();
    words.dedup();
    words
}

fn split(query: &str) -> impl Iterator<Item = &str> {
    query
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '"' | '“' | '”' | '„'))
        .map(|word| word.trim_matches(|c: char| matches!(c, '.' | ':' | '!' | '?' | '(' | ')')))
        .filter(|word| !word.is_empty())
}

/// How well `text` matches `tokens`: 0 when a token is missing, else for
/// each token 3 for a whole word, 2 for a word's start, 1 inside a word;
/// normalized so a full whole-word match is 1.0. No tokens match all.
pub fn score(text: &str, tokens: &[String]) -> f64 {
    if tokens.is_empty() {
        return 0.5;
    }
    let text = fold(text);
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric() && c != '+' && c != '@')
        .filter(|word| !word.is_empty())
        .collect();
    let mut total = 0.0;
    for token in tokens {
        let points = if words.iter().any(|word| word == token) {
            3.0
        } else if words.iter().any(|word| word.starts_with(token.as_str())) {
            2.0
        } else if text.contains(token.as_str()) {
            1.0
        } else {
            return 0.0;
        };
        total += points;
    }
    total / (3.0 * tokens.len() as f64)
}

/// A bonus up to `weight` for something used `now_ms - when_ms` ago:
/// halved after about a week.
pub fn recency(when_ms: Option<i64>, now_ms: i64, weight: f64) -> f64 {
    let Some(when) = when_ms else {
        return 0.0;
    };
    let days = ((now_ms - when).abs() as f64) / 86_400_000.0;
    weight / (1.0 + days / 7.0)
}

/// The best `limit` hits, best first (newest first among equals).
pub fn top(mut hits: Vec<Hit>, limit: usize) -> Vec<Hit> {
    hits.retain(|hit| hit.score > 0.0);
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.when_ms.cmp(&a.when_ms))
    });
    hits.dedup_by(|a, b| a.title == b.title && a.detail == b.detail && a.when_ms == b.when_ms);
    hits.truncate(limit);
    hits
}

/// `ms` as local "2026-11-13 14:05".
pub fn local_time(ms: i64) -> String {
    #[cfg(unix)]
    {
        let seconds = (ms / 1000) as libc::time_t;
        // SAFETY: `tm` is plain data that localtime_r fills; both pointers
        // outlive the call.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&seconds, &mut tm) }.is_null() {
            return format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                tm.tm_year + 1900,
                tm.tm_mon + 1,
                tm.tm_mday,
                tm.tm_hour,
                tm.tm_min
            );
        }
    }
    let (year, month, day) = civil_from_days(ms / 86_400_000);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Unix ms of a UTC "2026-11-13 09:58:27 +0000" (Spotlight's dates).
pub fn parse_utc(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, rest) = text.split_once(' ')?;
    let time = rest.split_whitespace().next()?;
    let mut date = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let mut time = time.split(':').map(|part| part.parse::<i64>().ok());
    let (hour, minute, second) = (time.next()??, time.next()??, time.next()??);
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000)
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn umlauts_and_case_fold_together() {
        assert_eq!(fold("Müller"), "muller");
        assert_eq!(fold("MUELLER"), "muller");
        assert_eq!(fold("Straße"), "strasse");
        assert_eq!(
            tokens("  „Rechnung“ Kalibermarkt, März! "),
            ["rechnung", "kalibermarkt", "marz"]
        );
    }

    #[test]
    fn whole_words_beat_prefixes_beat_substrings_and_all_words_must_match() {
        let query = tokens("rechnung");
        let whole = score("Rechnung 2026.pdf", &query);
        let prefix = score("Rechnungen.php", &query);
        let inside = score("elrechnungsversand.scss", &query);
        assert!(
            whole > prefix && prefix > inside && inside > 0.0,
            "{whole} {prefix} {inside}"
        );
        assert_eq!(score("Rechnung.pdf", &tokens("rechnung mai")), 0.0);
        assert_eq!(score("x", &[]), 0.5);
        assert_eq!(score("Michael Müller", &tokens("mueller")), 1.0);
    }

    #[test]
    fn recent_things_rank_first_among_equals() {
        let now = 1_000 * 86_400_000;
        assert!(recency(Some(now), now, 1.0) > recency(Some(now - 7 * 86_400_000), now, 1.0));
        assert_eq!(recency(None, now, 1.0), 0.0);
        let hit = |title: &str, score, when| Hit {
            title: title.into(),
            detail: String::new(),
            when_ms: Some(when),
            score,
        };
        let ranked = top(
            vec![
                hit("old", 0.5, 1),
                hit("none", 0.0, 9),
                hit("new", 0.5, 5),
                hit("best", 0.9, 0),
            ],
            2,
        );
        let titles: Vec<&str> = ranked.iter().map(|hit| hit.title.as_str()).collect();
        assert_eq!(titles, ["best", "new"]);
    }

    #[test]
    fn spotlight_dates_parse_as_utc() {
        assert_eq!(parse_utc("1970-01-02 00:00:01 +0000"), Some(86_401_000));
        assert_eq!(
            parse_utc("2026-11-13 09:58:27 +0000"),
            Some(1_794_563_907_000)
        );
        assert_eq!(parse_utc("(null)"), None);
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }
}
