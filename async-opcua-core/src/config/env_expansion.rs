//! Environment variable expansion for config values, following POSIX shell parameter
//! expansion.
//!
//! Supported forms: `$NAME`, `${NAME}`, `${NAME-word}`, `${NAME:-word}`, `${NAME+word}`,
//! `${NAME:+word}`, `${NAME?word}`, `${NAME:?word}`, and `$$` for a literal `$`. `word` is
//! expanded recursively. A `$` that does not start a reference is kept as written.
//!
//! A top-level `$NAME` or `${NAME}` naming an unset variable turns the whole value into YAML
//! null, as earlier versions did. Inside `word`, an unset variable expands to `""`.

use std::borrow::Cow;
use std::env;

use serde_saphyr::FormatterError;

/// YAML null, produced when a top-level reference names an unset variable.
const NULL: &str = "~";

/// How a plain reference to an unset variable is expanded.
#[derive(Clone, Copy)]
enum Unset {
    /// The whole value becomes null.
    Null,
    /// The reference expands to `""`.
    Empty,
}

/// Expands environment references in one config value.
pub(super) fn expand_env(input: Cow<'_, str>) -> Result<Cow<'_, str>, FormatterError> {
    if !input.contains('$') {
        return Ok(input);
    }
    Ok(match expand(&input, Unset::Null)? {
        Some(expanded) => Cow::Owned(expanded),
        None => Cow::Borrowed(NULL),
    })
}

/// Expands `input`, returning `None` when `unset` is [`Unset::Null`] and a plain reference
/// names an unset variable.
fn expand(input: &str, unset: Unset) -> Result<Option<String>, FormatterError> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let reference = &rest[i..];
        let after = &reference[1..];

        if let Some(tail) = after.strip_prefix('$') {
            out.push('$');
            rest = tail;
        } else if let Some(braced) = after.strip_prefix('{') {
            let close = find_close(braced).ok_or_else(|| {
                FormatterError::new(format!("unclosed variable reference `{reference}`"))
            })?;
            match expand_braced(&braced[..close], unset)? {
                Some(value) => out.push_str(&value),
                None => return Ok(None),
            }
            rest = &braced[close + 1..];
        } else {
            let len = name_len(after);
            if len == 0 {
                out.push('$');
            } else {
                match (lookup(&after[..len]), unset) {
                    (Some(value), _) => out.push_str(&value),
                    (None, Unset::Null) => return Ok(None),
                    (None, Unset::Empty) => {}
                }
            }
            rest = &after[len..];
        }
    }

    out.push_str(rest);
    Ok(Some(out))
}

/// Expands the body of a `${...}` reference (without the braces).
fn expand_braced(body: &str, unset: Unset) -> Result<Option<String>, FormatterError> {
    let len = name_len(body);
    if len == 0 {
        return Err(invalid(body));
    }
    let (name, op) = body.split_at(len);
    let value = lookup(name);

    let (colon, op) = match op.strip_prefix(':') {
        Some(op) => (true, op),
        None => (false, op),
    };
    let mut chars = op.chars();
    let Some(kind) = chars.next() else {
        if colon {
            return Err(invalid(body));
        }
        return Ok(match (value, unset) {
            (Some(value), _) => Some(value),
            (None, Unset::Null) => None,
            (None, Unset::Empty) => Some(String::new()),
        });
    };
    let word = chars.as_str();
    // With a colon, an empty variable is treated like an unset one.
    let is_set = value.as_ref().is_some_and(|v| !(colon && v.is_empty()));

    match kind {
        '-' if is_set => Ok(value),
        '-' => expand(word, Unset::Empty),
        '+' if is_set => expand(word, Unset::Empty),
        '+' => Ok(Some(String::new())),
        '?' if is_set => Ok(value),
        '?' => {
            let message = expand(word, Unset::Empty)?.unwrap_or_default();
            Err(FormatterError::new(if !message.is_empty() {
                format!("{name}: {message}")
            } else if colon {
                format!("{name}: variable is unset or empty")
            } else {
                format!("{name}: variable is unset")
            }))
        }
        _ => Err(invalid(body)),
    }
}

/// Finds the `}` closing a reference body, skipping nested `${...}` references.
fn find_close(body: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                depth += 1;
                i += 1;
            }
            b'}' if depth == 0 => return Some(i),
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Length of the variable name (`[A-Za-z_][A-Za-z0-9_]*`) at the start of `s`.
fn name_len(s: &str) -> usize {
    let mut bytes = s.bytes();
    match bytes.next() {
        Some(b) if b == b'_' || b.is_ascii_alphabetic() => {
            1 + bytes
                .take_while(|b| *b == b'_' || b.is_ascii_alphanumeric())
                .count()
        }
        _ => 0,
    }
}

fn lookup(name: &str) -> Option<String> {
    env::var(name).ok()
}

fn invalid(body: &str) -> FormatterError {
    FormatterError::new(format!("invalid variable reference `${{{body}}}`"))
}

#[cfg(test)]
mod tests {
    use super::expand_env;
    use std::borrow::Cow;

    fn run(input: &str) -> Result<String, String> {
        expand_env(Cow::Borrowed(input))
            .map(Cow::into_owned)
            .map_err(|e| e.to_string())
    }

    #[test]
    fn values_without_dollar_are_borrowed() {
        let out = expand_env(Cow::Borrowed("plain")).unwrap();
        assert!(matches!(out, Cow::Borrowed("plain")));
    }

    #[test]
    fn expands_references_inside_text() {
        std::env::set_var("EXP_UNIT_HOST", "db");
        assert_eq!(
            run("tcp://$EXP_UNIT_HOST:${EXP_UNIT_HOST}/x").unwrap(),
            "tcp://db:db/x"
        );
        assert_eq!(run("cost $5 and $").unwrap(), "cost $5 and $");
        assert_eq!(run("$$EXP_UNIT_HOST").unwrap(), "$EXP_UNIT_HOST");
        std::env::remove_var("EXP_UNIT_HOST");
    }

    #[test]
    fn unset_top_level_reference_is_null() {
        std::env::remove_var("EXP_UNIT_UNSET");
        assert_eq!(run("$EXP_UNIT_UNSET").unwrap(), "~");
        assert_eq!(run("prefix-${EXP_UNIT_UNSET}").unwrap(), "~");
        assert_eq!(run("${EXP_UNIT_UNSET:-a${EXP_UNIT_UNSET}b}").unwrap(), "ab");
    }

    #[test]
    fn nested_defaults() {
        std::env::remove_var("EXP_UNIT_A");
        std::env::set_var("EXP_UNIT_B", "b");
        assert_eq!(run("${EXP_UNIT_A:-${EXP_UNIT_B:-x}}").unwrap(), "b");
        // As in a shell, the first unmatched `}` closes the reference.
        assert_eq!(run("${EXP_UNIT_A:-{literal}}").unwrap(), "{literal}");
        std::env::remove_var("EXP_UNIT_B");
    }

    #[test]
    fn malformed_references_are_errors() {
        assert!(run("${")
            .unwrap_err()
            .starts_with("unclosed variable reference"));
        assert!(run("${1A}")
            .unwrap_err()
            .starts_with("invalid variable reference"));
        assert!(run("${A:}")
            .unwrap_err()
            .starts_with("invalid variable reference"));
        assert!(run("${A*b}")
            .unwrap_err()
            .starts_with("invalid variable reference"));
    }

    #[test]
    fn required_messages() {
        std::env::remove_var("EXP_UNIT_REQ");
        assert_eq!(
            run("${EXP_UNIT_REQ?}").unwrap_err(),
            "EXP_UNIT_REQ: variable is unset"
        );
        assert_eq!(
            run("${EXP_UNIT_REQ:?}").unwrap_err(),
            "EXP_UNIT_REQ: variable is unset or empty"
        );
        assert_eq!(
            run("${EXP_UNIT_REQ:?set it}").unwrap_err(),
            "EXP_UNIT_REQ: set it"
        );
    }
}
