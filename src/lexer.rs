/// Minimal Python tokenizer that preserves source exactly.
/// It splits source into tokens so transforms can safely skip strings/comments.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokKind {
    Ident,
    Number,
    Str,
    Comment,
    Newline,
    Whitespace,
    Op,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokKind,
    pub text: String,
}

impl Token {
    fn new(kind: TokKind, text: String) -> Self {
        Self { kind, text }
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

/// Try to parse a string starting at `start` (which points at a possible
/// prefix or quote). Returns (token_text, end_byte_index) on success.
pub(crate) fn parse_string(src: &str, start: usize) -> Option<(String, usize)> {
    let b = src.as_bytes();
    let n = b.len();
    let mut i = start;

    // Collect prefix letters (r,b,u,f and combos, case-insensitive).
    let mut prefix = String::new();
    while i < n {
        let c = b[i] as char;
        if c == '\'' || c == '"' {
            break;
        }
        if "rRbBuUfF".contains(c) && (i - start) < 3 {
            prefix.push(c);
            i += 1;
        } else {
            break;
        }
    }
    if i >= n {
        return None;
    }
    let q = b[i] as char;
    if q != '\'' && q != '"' {
        return None;
    }
    // Prefix must be a valid string prefix to count as a string.
    if !prefix.is_empty() && !is_valid_prefix(&prefix) {
        return None;
    }
    // If prefix contains identifier chars beyond a valid prefix, this isn't
    // a string (e.g. `foo"bar"` is not a string start). Caller only calls
    // this when at ident boundary, so a non-empty invalid prefix means bail.
    // Determine triple vs single quote.
    let triple = i + 2 < n && b[i + 1] as char == q && b[i + 2] as char == q;
    let is_raw = prefix.to_lowercase().contains('r');
    let is_fstr = prefix.to_lowercase().contains('f');
    let mut j = if triple { i + 3 } else { i + 1 };
    let mut brace_depth: i32 = 0;

    while j < n {
        let c = b[j] as char;
        if !is_raw && c == '\\' {
            // Skip escaped char (also handles line continuation).
            j += 2;
            continue;
        }
        if is_fstr && !triple {
            if c == '{' {
                // `{{` is an escaped brace.
                if j + 1 < n && b[j + 1] as char == '{' {
                    j += 2;
                    continue;
                }
                brace_depth += 1;
                j += 1;
                continue;
            }
            if c == '}' {
                if brace_depth > 0 {
                    brace_depth -= 1;
                } else if j + 1 < n && b[j + 1] as char == '}' {
                    j += 2;
                    continue;
                }
                j += 1;
                continue;
            }
        }
        if is_fstr && triple && (c == '{' || c == '}') {
            // Rough handling inside triple f-strings; braces don't end string.
            j += 1;
            continue;
        }
        if c == q {
            if triple {
                if j + 2 < n && b[j + 1] as char == q && b[j + 2] as char == q {
                    // In non-raw strings a backslash right before quotes was
                    // already skipped above, so this closes the string.
                    j += 3;
                    return Some((src[start..j].to_string(), j));
                }
                j += 1;
                continue;
            } else if brace_depth == 0 {
                j += 1;
                return Some((src[start..j].to_string(), j));
            } else {
                j += 1;
                continue;
            }
        }
        if !triple && (c == '\n' || c == '\r') {
            // Single-quoted strings can't span lines (unless line continuation,
            // handled via backslash skip above).
            return None;
        }
        j += 1;
    }
    // Unterminated: treat triple-quoted as running to EOF (tolerant),
    // single-quoted as not-a-string.
    if triple {
        return Some((src[start..j].to_string(), j));
    }
    None
}

fn is_valid_prefix(p: &str) -> bool {
    matches!(
        p.to_lowercase().as_str(),
        "r" | "b" | "u" | "f" | "br" | "rb" | "fr" | "rf"
    )
}

/// Parse a number starting at `start`. Returns end index.
fn parse_number(src: &str, start: usize) -> usize {
    let b = src.as_bytes();
    let n = b.len();
    let mut j = start;
    // Hex / oct / bin.
    if b[start] as char == '0' && j + 1 < n {
        let c1 = b[j + 1] as char;
        if matches!(c1, 'x' | 'X' | 'o' | 'O' | 'b' | 'B') {
            j += 2;
            while j < n {
                let c = b[j] as char;
                if c.is_ascii_hexdigit() || c == '_' {
                    j += 1;
                } else {
                    break;
                }
            }
            return j;
        }
    }
    // Decimal / float.
    while j < n && (is_digit(b[j] as char) || b[j] as char == '_') {
        j += 1;
    }
    // Fractional part.
    if j < n && b[j] as char == '.' {
        // `x.5` handled by caller; `5.` is a float; avoid consuming `..`
        // (never valid python anyway) — simple approach is fine.
        let mut k = j + 1;
        let mut frac = 0;
        while k < n && (is_digit(b[k] as char) || b[k] as char == '_') {
            k += 1;
            frac += 1;
        }
        if frac > 0 || (j > start) {
            j = k;
        }
    }
    // Exponent.
    if j < n && (b[j] as char == 'e' || b[j] as char == 'E') {
        let mut k = j + 1;
        if k < n && (b[k] as char == '+' || b[k] as char == '-') {
            k += 1;
        }
        let exp_start = k;
        while k < n && (is_digit(b[k] as char) || b[k] as char == '_') {
            k += 1;
        }
        if k > exp_start {
            j = k;
        }
    }
    // Complex suffix.
    if j < n && (b[j] as char == 'j' || b[j] as char == 'J') {
        j += 1;
    }
    j
}

const THREE_CHAR_OPS: [&str; 4] = ["**=", "//=", ">>=", "<<="];
const TWO_CHAR_OPS: [&str; 19] = [
    "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "->", ":=", "**", "//",
    ">>", "<<", "...",
];

pub fn tokenize(src: &str) -> Vec<Token> {
    let mut toks: Vec<Token> = Vec::new();
    let b = src.as_bytes();
    let n = b.len();
    let mut i = 0;
    // At a position where an identifier/keyword could start, a quote or a
    // valid string prefix + quote begins a string.
    while i < n {
        let c = b[i] as char;
        // Newlines (handle \r\n, \r).
        if c == '\r' || c == '\n' {
            if c == '\r' && i + 1 < n && b[i + 1] as char == '\n' {
                toks.push(Token::new(TokKind::Newline, "\n".to_string()));
                i += 2;
            } else {
                toks.push(Token::new(TokKind::Newline, "\n".to_string()));
                i += 1;
            }
            continue;
        }
        // Horizontal whitespace.
        if c == ' ' || c == '\t' || c == '\x0C' {
            let mut j = i;
            while j < n && {
                let d = b[j] as char;
                d == ' ' || d == '\t' || d == '\x0C'
            } {
                j += 1;
            }
            toks.push(Token::new(TokKind::Whitespace, src[i..j].to_string()));
            i = j;
            continue;
        }
        // Comment (only when not inside string; strings handled first at
        // token starts, and `#` can't appear inside other tokens).
        if c == '#' {
            let mut j = i;
            while j < n && b[j] as char != '\n' && b[j] as char != '\r' {
                j += 1;
            }
            toks.push(Token::new(TokKind::Comment, src[i..j].to_string()));
            i = j;
            continue;
        }
        // String: quote, or prefix+quote at token boundary.
        if c == '\'' || c == '"' || "rRbBuUfF".contains(c) {
            // Only attempt string parse if this could be a string start:
            // current char is a quote, or scanning a prefix leads to a quote.
            let mut k = i;
            while k < n && "rRbBuUfF".contains(b[k] as char) && (k - i) < 3 {
                k += 1;
            }
            let is_quote_start = (b[i] as char == '\'' || b[i] as char == '"')
                || (k < n && (b[k] as char == '\'' || b[k] as char == '"'));
            if is_quote_start {
                if let Some((text, end)) = parse_string(src, i) {
                    toks.push(Token::new(TokKind::Str, text));
                    i = end;
                    continue;
                }
            }
        }
        // Identifier (ASCII fast path + non-ASCII alphabetic fallback so
        // unicode identifiers stay single tokens).
        let ch_now = src[i..].chars().next().unwrap();
        let ident_start = is_ident_start(c) || (!ch_now.is_ascii() && ch_now.is_alphabetic());
        if ident_start {
            let mut j = i + ch_now.len_utf8();
            while j < n {
                let ch = src[j..].chars().next().unwrap();
                let cont = if ch.is_ascii() {
                    ch.is_ascii_alphanumeric() || ch == '_'
                } else {
                    ch.is_alphabetic()
                };
                if !cont {
                    break;
                }
                j += ch.len_utf8();
            }
            toks.push(Token::new(TokKind::Ident, src[i..j].to_string()));
            i = j;
            continue;
        }
        // Number (also `.5` float leading-dot).
        if is_digit(c) || (c == '.' && i + 1 < n && is_digit(b[i + 1] as char)) {
            let end = parse_number(src, i);
            toks.push(Token::new(TokKind::Number, src[i..end].to_string()));
            i = end;
            continue;
        }
        // Line continuation backslash outside strings.
        if c == '\\' {
            // Backslash-newline joins lines; keep as Op to preserve it.
            if i + 1 < n && (b[i + 1] as char == '\n' || b[i + 1] as char == '\r') {
                if b[i + 1] as char == '\r' && i + 2 < n && b[i + 2] as char == '\n' {
                    toks.push(Token::new(TokKind::Op, src[i..i + 3].to_string()));
                    i += 3;
                } else {
                    toks.push(Token::new(TokKind::Op, src[i..i + 2].to_string()));
                    i += 2;
                }
                continue;
            }
            toks.push(Token::new(TokKind::Op, "\\".to_string()));
            i += 1;
            continue;
        }
        // Multi-char operators first (ASCII-only; guarded for unicode safety).
        if b[i] < 128 && i + 3 <= n && THREE_CHAR_OPS.contains(&&src[i..i + 3]) {
            toks.push(Token::new(TokKind::Op, src[i..i + 3].to_string()));
            i += 3;
            continue;
        }
        if b[i] < 128 && i + 2 <= n && TWO_CHAR_OPS.contains(&&src[i..i + 2]) {
            toks.push(Token::new(TokKind::Op, src[i..i + 2].to_string()));
            i += 2;
            continue;
        }
        // Single char op (unicode-safe: never splits a char boundary).
        let ch = src[i..].chars().next().unwrap();
        toks.push(Token::new(TokKind::Op, ch.to_string()));
        i += ch.len_utf8();
    }
    toks
}

/// Split a string token into (prefix, quote, body).
/// Returns None if the token text is not shaped like a string.
pub fn split_string(text: &str) -> Option<(String, String, String)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n && "rRbBuUfF".contains(b[i] as char) {
        i += 1;
    }
    let prefix = text[..i].to_string();
    if i >= n {
        return None;
    }
    let q = b[i] as char;
    if q != '\'' && q != '"' {
        return None;
    }
    let triple = i + 2 < n && b[i + 1] as char == q && b[i + 2] as char == q;
    if triple {
        let open = i + 3;
        let close = text.len() - 3;
        if text.len() < 6 || !text.ends_with(&q.to_string().repeat(3)) {
            return None;
        }
        let quote = q.to_string().repeat(3);
        let body = text[open..close].to_string();
        Some((prefix, quote, body))
    } else {
        let open = i + 1;
        if text.len() < 2 || !text.ends_with(q) {
            return None;
        }
        let close = text.len() - 1;
        let body = text[open..close].to_string();
        Some((prefix, q.to_string(), body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_tokens() {
        let toks = tokenize("x = 1 # hi\n");
        let kinds: Vec<&str> = toks
            .iter()
            .map(|t| match t.kind {
                TokKind::Ident => "ident",
                TokKind::Number => "num",
                TokKind::Str => "str",
                TokKind::Comment => "com",
                TokKind::Newline => "nl",
                TokKind::Whitespace => "ws",
                TokKind::Op => "op",
            })
            .collect();
        assert!(kinds.contains(&"ident"));
        assert!(kinds.contains(&"com"));
        assert!(kinds.contains(&"nl"));
    }

    #[test]
    fn strings_with_hash_and_escapes() {
        let toks = tokenize("s = \"a # not comment\" + 'it\\'s'\n");
        assert!(toks.iter().any(|t| t.kind == TokKind::Str));
        assert!(!toks.iter().any(|t| t.kind == TokKind::Comment));
    }

    #[test]
    fn triple_string_multiline() {
        let toks = tokenize("s = '''a\nb''' \n");
        let s: Vec<_> = toks.iter().filter(|t| t.kind == TokKind::Str).collect();
        assert_eq!(s.len(), 1);
        assert!(s[0].text.contains('\n'));
    }

    #[test]
    fn fstring_not_confused() {
        let toks = tokenize("s = f\"hi {name}!\" \n");
        assert!(toks.iter().any(|t| t.kind == TokKind::Str));
    }

    #[test]
    fn split_roundtrip() {
        let (p, q, body) = split_string("\"hello\"").unwrap();
        assert_eq!((p.as_str(), q.as_str(), body.as_str()), ("", "\"", "hello"));
        let (p, q, _) = split_string("r'''x'''").unwrap();
        assert_eq!((p.as_str(), q.as_str()), ("r", "'''"));
    }
}
