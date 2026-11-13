//! Contacts: names, company, phones (in international form, as Messages and
//! WhatsApp need them), and emails. Through the Contacts framework when
//! Hex has Contacts access, else from the address book's own database
//! (Full Disk Access), read from a copy.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::exec;
use super::native::{self, Access};
use super::rank::{self, Hit};

/// One person or company card.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Contact {
    pub name: String,
    pub nickname: String,
    pub organization: String,
    pub job_title: String,
    pub phones: Vec<Phone>,
    pub emails: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Phone {
    pub label: String,
    pub number: String,
}

/// `number` as E.164 ("+491621234567") for this Mac's `region`: spaces,
/// dashes, dots, slashes, brackets, and a German "(0)" dropped, a national
/// or "00" prefix made international. Something that isn't a phone number
/// stays as it was.
pub fn e164(number: &str, region: &str) -> String {
    let trimmed = number.trim();
    let without_trunk = trimmed.replace("(0)", "");
    let mut digits = String::new();
    for c in without_trunk.chars() {
        if c.is_ascii_digit() || (c == '+' && digits.is_empty()) {
            digits.push(c);
        } else if !matches!(c, ' ' | '-' | '.' | '/' | '(' | ')' | '\u{a0}') {
            return trimmed.to_owned();
        }
    }
    let count = digits.trim_start_matches('+').len();
    if !(6..=16).contains(&count) {
        return trimmed.to_owned();
    }
    crate::assistant::phone::incoming::international(&digits, region)
}

/// An address book label ("_$!<Mobile>!$_") as a word ("mobile").
fn label(raw: &str) -> String {
    raw.trim_start_matches("_$!<")
        .trim_end_matches(">!$_")
        .to_lowercase()
}

/// How well `contact` matches: its names and company, or a number or
/// address (a number's digits compare without spacing or prefix).
pub fn score(contact: &Contact, tokens: &[String]) -> f64 {
    let names = format!(
        "{} {} {} {}",
        contact.name, contact.nickname, contact.organization, contact.job_title
    );
    let by_name = rank::score(&names, tokens);
    let reachable: Vec<String> = contact
        .phones
        .iter()
        .flat_map(|phone| {
            let digits: String = phone.number.chars().filter(char::is_ascii_digit).collect();
            [
                phone.number.clone(),
                digits.trim_start_matches('0').to_owned(),
            ]
        })
        .chain(contact.emails.iter().cloned())
        .collect();
    let tokens_digits: Vec<String> = tokens
        .iter()
        .map(|token| {
            let digits = token.trim_start_matches('+');
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                digits.trim_start_matches('0').to_owned()
            } else {
                token.clone()
            }
        })
        .collect();
    let by_address = rank::score(&format!("{names} {}", reachable.join(" ")), &tokens_digits) * 0.9;
    by_name.max(by_address)
}

/// `contact` as a result line: phones in E.164 with their labels.
pub fn hit(contact: &Contact, score: f64, region: &str) -> Hit {
    let mut title = if contact.name.is_empty() {
        contact.organization.clone()
    } else {
        contact.name.clone()
    };
    if !contact.nickname.is_empty() {
        title.push_str(&format!(" ({})", contact.nickname));
    }
    let mut parts = Vec::new();
    if !contact.name.is_empty() && !contact.organization.is_empty() {
        parts.push(if contact.job_title.is_empty() {
            contact.organization.clone()
        } else {
            format!("{}, {}", contact.job_title, contact.organization)
        });
    }
    for phone in contact.phones.iter().take(4) {
        let number = e164(&phone.number, region);
        parts.push(if phone.label.is_empty() {
            number
        } else {
            format!("{} {number}", label(&phone.label))
        });
    }
    parts.extend(contact.emails.iter().take(3).cloned());
    Hit {
        title,
        detail: parts.join(" · "),
        when_ms: None,
        score,
    }
}

/// The address book's databases: the local one and each account's.
fn databases() -> Vec<PathBuf> {
    let Some(home) = crate::app_paths::home_dir() else {
        return Vec::new();
    };
    let root = home.join("Library/Application Support/AddressBook");
    let mut found = vec![root.join("AddressBook-v22.abcddb")];
    if let Ok(sources) = std::fs::read_dir(root.join("Sources")) {
        found.extend(
            sources
                .flatten()
                .map(|source| source.path().join("AddressBook-v22.abcddb")),
        );
    }
    found.retain(|path| path.exists());
    found
}

const ADDRESS_BOOK_SQL: &str = "SELECT r.ZFIRSTNAME AS first, r.ZLASTNAME AS last, \
     r.ZNICKNAME AS nick, r.ZORGANIZATION AS org, r.ZJOBTITLE AS job, \
     (SELECT group_concat(IFNULL(p.ZLABEL, '') || char(31) || p.ZFULLNUMBER, char(30)) \
        FROM ZABCDPHONENUMBER p WHERE p.ZOWNER = r.Z_PK) AS phones, \
     (SELECT group_concat(e.ZADDRESS, char(30)) FROM ZABCDEMAILADDRESS e \
        WHERE e.ZOWNER = r.Z_PK) AS emails \
     FROM ZABCDRECORD r WHERE r.ZFIRSTNAME IS NOT NULL OR r.ZLASTNAME IS NOT NULL \
        OR r.ZORGANIZATION IS NOT NULL LIMIT 20000;";

/// Address book rows as contacts.
pub fn from_rows(rows: &[Value]) -> Vec<Contact> {
    let text = |row: &Value, key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    rows.iter()
        .map(|row| {
            let name = [text(row, "first"), text(row, "last")]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let phones = text(row, "phones")
                .split('\u{1e}')
                .filter(|entry| !entry.is_empty())
                .map(|entry| {
                    let (label, number) = entry.split_once('\u{1f}').unwrap_or(("", entry));
                    Phone {
                        label: label.to_owned(),
                        number: number.to_owned(),
                    }
                })
                .collect();
            let emails = text(row, "emails")
                .split('\u{1e}')
                .filter(|email| !email.is_empty())
                .map(str::to_owned)
                .collect();
            Contact {
                name,
                nickname: text(row, "nick"),
                organization: text(row, "org"),
                job_title: text(row, "job"),
                phones,
                emails,
            }
        })
        .collect()
}

fn all_from_address_book() -> Result<Vec<Contact>, String> {
    let databases = databases();
    if databases.is_empty() {
        return Err("no address book found".into());
    }
    let mut contacts = Vec::new();
    let mut error = None;
    for database in databases {
        match exec::query_copy(&database, ADDRESS_BOOK_SQL, Duration::from_secs(3)) {
            Ok(rows) => contacts.extend(from_rows(&rows)),
            Err(why) => error = Some(why),
        }
    }
    match (contacts.is_empty(), error) {
        (true, Some(error)) => Err(error),
        _ => Ok(contacts),
    }
}

/// People and companies matching the query.
pub fn contacts(query: &str, tokens: &[String], limit: usize) -> Result<Vec<Hit>, String> {
    let candidates: Vec<Contact> = match native::contacts_access() {
        Access::Granted => native::contacts(query, 300)
            .and_then(|json| serde_json::from_str(&json).ok())
            .ok_or("Contacts didn't answer")?,
        access => all_from_address_book().map_err(|error| {
            let asked = if access == Access::NotAsked {
                "Hex hasn't asked for Contacts access yet"
            } else {
                "Contacts access is off for Hex"
            };
            format!("{asked} (System Settings › Privacy & Security › Contacts), and the address book couldn't be read: {error}")
        })?,
    };
    let region = crate::assistant::phone::incoming::region();
    let hits = candidates
        .iter()
        .map(|contact| hit(contact, score(contact, tokens), &region))
        .collect();
    Ok(rank::top(hits, limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_become_international() {
        assert_eq!(e164("0162 / 123 45-67", "DE"), "+491621234567");
        assert_eq!(e164("+49 (0) 162 1234567", "DE"), "+491621234567");
        assert_eq!(e164("0049 162 1234567", "AT"), "+491621234567");
        assert_eq!(e164("0664 1234567", "AT"), "+436641234567");
        assert_eq!(e164("+1 (415) 555-0100", "DE"), "+14155550100");
        assert_eq!(e164("call me", "DE"), "call me");
        assert_eq!(e164("112", "DE"), "112");
    }

    #[test]
    fn address_book_rows_parse_with_labels() {
        let rows = [json!({
            "first": "Michael", "last": "Schwarz", "org": "Kalibermarkt GmbH",
            "job": null, "nick": null,
            "phones": "_$!<Mobile>!$_\u{1f}0162 1234567\u{1e}\u{1f}+49 30 123456",
            "emails": "m@kalibermarkt.de"
        })];
        let contacts = from_rows(&rows);
        assert_eq!(contacts[0].name, "Michael Schwarz");
        assert_eq!(contacts[0].phones.len(), 2);
        let line = hit(&contacts[0], 1.0, "DE");
        assert_eq!(line.title, "Michael Schwarz");
        assert_eq!(
            line.detail,
            "Kalibermarkt GmbH · mobile +491621234567 · +4930123456 · m@kalibermarkt.de"
        );
    }

    #[test]
    fn people_match_by_name_company_or_number() {
        let contact = Contact {
            name: "Michael Schwarz".into(),
            organization: "Kalibermarkt".into(),
            phones: vec![Phone {
                label: String::new(),
                number: "+49 162 1234567".into(),
            }],
            emails: vec!["m@kalibermarkt.de".into()],
            ..Default::default()
        };
        assert_eq!(score(&contact, &rank::tokens("michael schwarz")), 1.0);
        assert!(score(&contact, &rank::tokens("kalibermarkt")) > 0.9);
        assert!(score(&contact, &rank::tokens("0162 1234567")) > 0.0);
        assert!(score(&contact, &rank::tokens("m@kalibermarkt.de")) > 0.0);
        assert_eq!(score(&contact, &rank::tokens("michael weber")), 0.0);
    }
}
