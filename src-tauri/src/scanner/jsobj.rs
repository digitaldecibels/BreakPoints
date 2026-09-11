//! Just enough JavaScript to read an object literal, and no more.
//!
//! A Tailwind config is JavaScript, so it can import, spread and compute. We
//! parse it statically and never execute it: a responsive tester must not be a
//! way to run arbitrary code from a repo you just opened. The cost is that some
//! configs will not resolve, and the answer to that is to say exactly why (see
//! `ScanLog::parse_failure`) rather than to start evaluating.

/// Replace every comment with spaces, keeping byte offsets and line numbers
/// intact so a failure can still be reported at the right line.
pub fn blank_comments(src: &str) -> String {
    let bytes: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut quote: Option<char> = None;

    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1]);
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '"' | '\'' | '`' => {
                quote = Some(c);
                out.push(c);
                i += 1;
            }
            '/' if i + 1 < bytes.len() && bytes[i + 1] == '/' => {
                while i < bytes.len() && bytes[i] != '\n' {
                    out.push(' ');
                    i += 1;
                }
            }
            '/' if i + 1 < bytes.len() && bytes[i + 1] == '*' => {
                let mut j = i;
                while j < bytes.len() {
                    if bytes[j] == '\n' {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                    if j > i && bytes[j - 1] == '*' && bytes[j] == '/' {
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                i = j;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// The index just past the `{` that opens an object, given the index of a `:`.
/// Returns None when the value is not an object literal, which is the normal
/// way a spread or an identifier gets rejected.
pub fn object_after_colon(src: &[char], colon: usize) -> Option<usize> {
    let mut i = colon + 1;
    while i < src.len() && src[i].is_whitespace() {
        i += 1;
    }
    if i < src.len() && src[i] == '{' {
        Some(i)
    } else {
        None
    }
}

/// Index of the `}` that closes the `{` at `open`.
pub fn match_braces(src: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    let mut quote: Option<char> = None;
    while i < src.len() {
        let c = src[i];
        if let Some(q) = quote {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Index of the `:` following a key with this name, searching from `from`.
/// Matches `screens:`, `"screens":` and `'screens':`.
pub fn find_key(src: &[char], key: &str, from: usize) -> Option<usize> {
    let key: Vec<char> = key.chars().collect();
    let mut i = from;
    while i + key.len() < src.len() {
        if src[i..].starts_with(&key[..]) {
            let before_ok = i == 0 || {
                let b = src[i - 1];
                !b.is_alphanumeric() && b != '_' && b != '$' && b != '.'
            };
            let mut j = i + key.len();
            // Allow a closing quote when the key was written as a string.
            if before_ok && j < src.len() && (src[j] == '"' || src[j] == '\'') {
                j += 1;
            }
            while j < src.len() && src[j].is_whitespace() {
                j += 1;
            }
            if before_ok && j < src.len() && src[j] == ':' {
                return Some(j);
            }
        }
        i += 1;
    }
    None
}

/// The `:` after a key that is a *direct* member of the object opened at
/// `open`, rather than one buried in a nested object.
///
/// "The first `screens` in the file" is the wrong question to ask.
/// `theme.container.screens` is the container plugin's max-widths and has
/// nothing to do with breakpoints, but it is spelled the same and in a real
/// config it usually comes first.
pub fn child_key(src: &[char], open: usize, key: &str) -> Option<usize> {
    let close = match_braces(src, open)?;
    let mut from = open + 1;
    while let Some(colon) = find_key(src, key, from) {
        if colon >= close {
            return None;
        }
        if depth_between(src, open, colon) == 1 {
            return Some(colon);
        }
        from = colon + 1;
    }
    None
}

/// How many objects deep `pos` is, counting from the `{` at `open`.
fn depth_between(src: &[char], open: usize, pos: usize) -> usize {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut i = open;
    while i < pos && i < src.len() {
        let c = src[i];
        if let Some(q) = quote {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    depth
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    /// The value exactly as written, so an unresolvable one can be logged.
    pub value: String,
    pub offset: usize,
}

/// Read the top-level entries of the object opened at `open`.
pub fn entries(src: &[char], open: usize) -> Vec<Entry> {
    let Some(close) = match_braces(src, open) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut i = open + 1;

    while i < close {
        while i < close && (src[i].is_whitespace() || src[i] == ',') {
            i += 1;
        }
        if i >= close {
            break;
        }
        let key_start = i;
        let key = if src[i] == '"' || src[i] == '\'' {
            let quote = src[i];
            i += 1;
            let start = i;
            while i < close && src[i] != quote {
                i += 1;
            }
            let key: String = src[start..i].iter().collect();
            i += 1;
            key
        } else {
            let start = i;
            // A comma ends the run too, or a spread swallows the separator and
            // comes back as `...rest,`.
            while i < close && !src[i].is_whitespace() && src[i] != ':' && src[i] != ',' {
                i += 1;
            }
            src[start..i].iter().collect()
        };
        while i < close && src[i].is_whitespace() {
            i += 1;
        }
        if i >= close || src[i] != ':' {
            // A spread, a method, or something else that is not `key: value`.
            // Record it so the log can say what it was.
            let raw: String = src[key_start..i.min(close)].iter().collect();
            if !raw.trim().is_empty() {
                out.push(Entry {
                    key: String::new(),
                    value: raw.trim().to_string(),
                    offset: key_start,
                });
            }
            while i < close && src[i] != ',' {
                i += 1;
            }
            continue;
        }
        i += 1; // past the colon
        while i < close && src[i].is_whitespace() {
            i += 1;
        }
        let value_start = i;
        let mut depth = 0usize;
        let mut quote: Option<char> = None;
        while i < close {
            let c = src[i];
            if let Some(q) = quote {
                if c == '\\' {
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
                i += 1;
                continue;
            }
            match c {
                '"' | '\'' | '`' => quote = Some(c),
                '{' | '[' | '(' => depth += 1,
                '}' | ']' | ')' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => break,
                _ => {}
            }
            i += 1;
        }
        let value: String = src[value_start..i.min(close)].iter().collect();
        out.push(Entry {
            key,
            value: value.trim().to_string(),
            offset: value_start,
        });
    }
    out
}

pub fn line_of(src: &[char], offset: usize) -> usize {
    src[..offset.min(src.len())]
        .iter()
        .filter(|c| **c == '\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        blank_comments(s).chars().collect()
    }

    #[test]
    fn comments_vanish_without_moving_anything_after_them() {
        let src = "a // note\nb /* x */ c";
        let out = blank_comments(src);
        assert_eq!(out.len(), src.len());
        assert_eq!(out.lines().count(), src.lines().count());
        assert!(!out.contains("note"));
    }

    #[test]
    fn a_brace_inside_a_string_does_not_close_the_object() {
        let src = chars(r#"{ a: "}", b: 1 }"#);
        assert_eq!(match_braces(&src, 0), Some(src.len() - 1));
    }

    #[test]
    fn screens_is_found_however_the_key_is_quoted() {
        for src in [r#"{ screens: {} }"#, r#"{ "screens": {} }"#, r#"{ 'screens' : {} }"#] {
            let c = chars(src);
            assert!(find_key(&c, "screens", 0).is_some(), "{src}");
        }
    }

    #[test]
    fn a_key_that_is_only_a_substring_is_not_a_match() {
        let c = chars(r#"{ myscreens: {}, theme: { screens: {} } }"#);
        let colon = find_key(&c, "screens", 0).unwrap();
        // The first hit must be the real key, not the tail of `myscreens`.
        let open = object_after_colon(&c, colon).unwrap();
        assert!(open > 20);
    }

    #[test]
    fn entries_come_back_with_their_values_as_written() {
        let c = chars(r#"{ sm: '640px', md: { min: '768px' }, ...rest, lg: base.lg }"#);
        let open = 0;
        let found = entries(&c, open);
        assert_eq!(found[0], Entry { key: "sm".into(), value: "'640px'".into(), offset: found[0].offset });
        assert_eq!(found[1].key, "md");
        assert!(found[1].value.starts_with('{'));
        assert_eq!(found[2].key, "");
        assert_eq!(found[2].value, "...rest");
        assert_eq!(found[3].value, "base.lg");
    }
}
