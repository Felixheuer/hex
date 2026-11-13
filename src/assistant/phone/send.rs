//! Sends an iMessage through the Mac's Messages app (AppleScript; Hex asks
//! macOS once for permission to control Messages).

use color_eyre::eyre::{Result, bail};

/// The AppleScript that sends `text` to `handle` over the iMessage account.
pub fn script(handle: &str, text: &str) -> String {
    format!(
        "tell application \"Messages\"\n\
         \tset theAccount to 1st account whose service type = iMessage\n\
         \tsend {} to participant {} of theAccount\n\
         end tell",
        quoted(text),
        quoted(handle)
    )
}

/// `text` as an AppleScript string literal.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\r' => {}
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Sends `text` to `handle`; blocks until Messages took it.
pub fn send(handle: &str, text: &str) -> Result<()> {
    #[cfg(test)]
    {
        let _ = (handle, text);
        bail!("tests never send iMessages");
    }
    #[cfg(not(test))]
    {
        let output = std::process::Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script(handle, text))
            .stdin(std::process::Stdio::null())
            .output()?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            bail!("Messages didn't send: {}", error.trim());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_quotes_text_and_handle() {
        let script = script("+491701234567", "Say \"hi\" \\ now\nline two");
        assert!(script.contains("send \"Say \\\"hi\\\" \\\\ now\nline two\""));
        assert!(script.contains("participant \"+491701234567\""));
        assert!(script.contains("service type = iMessage"));
    }

    #[test]
    fn tests_never_send() {
        assert!(send("+491701234567", "x").is_err());
    }
}
