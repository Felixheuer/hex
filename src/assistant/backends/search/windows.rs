//! Windows' PATH as a newly signed-in session would get it: the machine's
//! and the person's own `Path` from the registry, read afresh, so a tool an
//! installer just added is found without restarting Hex.

use std::path::PathBuf;

/// `%NAME%` replaced by `var(NAME)` (names are case-insensitive on
/// Windows; `var` gets them as written); an unknown one stays as it is.
#[cfg_attr(
    not(any(windows, test)),
    expect(dead_code, reason = "only Windows reads its PATH from the registry")
)]
pub fn expand(text: &str, var: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match var(name) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A `Path` value's directories, in order: `;`-separated, quotes and
/// blanks dropped.
#[cfg_attr(
    not(any(windows, test)),
    expect(dead_code, reason = "only Windows reads its PATH from the registry")
)]
pub fn split(value: &str) -> Vec<PathBuf> {
    value
        .split(';')
        .map(|dir| dir.trim().trim_matches('"').trim())
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The machine's `Path`, then the person's, from the registry, expanded.
#[cfg(windows)]
pub fn registry_path() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

    let read = |root, key: &str| -> Option<String> {
        RegKey::predef(root)
            .open_subkey_with_flags(key, KEY_READ)
            .ok()?
            .get_value::<String, _>("Path")
            .ok()
    };
    let machine = read(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
    );
    let user = read(HKEY_CURRENT_USER, "Environment");
    [machine, user]
        .into_iter()
        .flatten()
        .flat_map(|value| split(&expand(&value, |name| std::env::var(name).ok())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_in_a_registry_path_are_expanded() {
        let var = |name: &str| match name {
            "USERPROFILE" => Some(r"C:\Users\f".to_owned()),
            "SystemRoot" => Some(r"C:\Windows".to_owned()),
            _ => None,
        };
        assert_eq!(
            expand(r"%SystemRoot%\system32;%USERPROFILE%\.local\bin", var),
            r"C:\Windows\system32;C:\Users\f\.local\bin"
        );
        assert_eq!(expand("%NOPE%\\bin;50%", var), "%NOPE%\\bin;50%");
    }

    #[test]
    fn a_path_value_splits_into_its_directories() {
        assert_eq!(
            split(r#"C:\Windows; "C:\Program Files\nodejs" ;;C:\Users\f\AppData\Roaming\npm"#),
            [
                r"C:\Windows",
                r"C:\Program Files\nodejs",
                r"C:\Users\f\AppData\Roaming\npm"
            ]
            .map(PathBuf::from)
        );
    }
}
