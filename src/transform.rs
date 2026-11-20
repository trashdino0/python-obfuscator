use crate::lexer::{split_string, TokKind, Token};
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashSet;

#[derive(Clone)]
pub struct ObfsOptions {
    pub seed: u64,
    pub keep: HashSet<String>,
    pub do_rename: bool,
    pub do_strings: bool,
    pub do_numbers: bool,
    pub do_bool_none: bool,
    pub do_deadcode: bool,
    pub do_flatten: bool,
    pub min_string_len: usize,
}

pub struct Stats {
    pub renamed: usize,
    pub strings_encrypted: usize,
    pub numbers_encoded: usize,
    pub bool_none_replaced: usize,
    pub dead_blocks: usize,
    pub flattened: usize,
}

pub struct Header {
    pub text: String,
    pub decoder: String,
    pub true_alias: String,
    pub false_alias: String,
    pub none_alias: String,
}

pub(crate) fn unique_name(rng: &mut ChaCha8Rng, used: &mut HashSet<String>) -> String {
    loop {
        let n: u32 = rng.random_range(0..0xFFFFFF);
        let cand = format!("_0x{n:06x}");
        if used.insert(cand.clone()) {
            return cand;
        }
    }
}

pub fn make_header(seed: u64, used: &mut HashSet<String>) -> (Header, u32) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x9E3779B97F4A7C15);
    let decoder = unique_name(&mut rng, used);
    let true_alias = unique_name(&mut rng, used);
    let false_alias = unique_name(&mut rng, used);
    let none_alias = unique_name(&mut rng, used);
    let key: u32 = rng.random_range(1..=u32::MAX);
    // Self-contained decoder: hex -> bytes, XOR with an LCG keystream.
    // No stdlib import at all (no `base64`, `bytes.fromhex` is a builtin
    // classmethod), so the injected stub is less recognisable.
    let text = format!(
        concat!(
            "def {decoder}(s,k={key}):\n",
            "    o=bytearray.fromhex(s)\n",
            "    x=k\n",
            "    for i in range(len(o)):\n",
            "        x=(x*1103515245+12345)&4294967295\n",
            "        o[i]^=(x>>16)&255\n",
            "    return o.decode()\n",
            "{true_alias}=True\n",
            "{false_alias}=False\n",
            "{none_alias}=None\n"
        ),
        decoder = decoder,
        key = key,
        true_alias = true_alias,
        false_alias = false_alias,
        none_alias = none_alias
    );
    (
        Header {
            text,
            decoder,
            true_alias,
            false_alias,
            none_alias,
        },
        key,
    )
}

/// Remove comments and collapse trailing whitespace (keep newlines).
pub fn strip_comments(toks: &mut [Token]) {
    for t in toks.iter_mut() {
        if t.kind == TokKind::Comment {
            t.text.clear();
        }
    }
}

/// Replace standalone string statements (docstrings / no-op string exprs)
/// with `pass`. Safe: both are no-ops; `pass` keeps empty bodies valid.
pub fn docstrings_to_pass(toks: &mut Vec<Token>) {
    // Split token index ranges per physical line (Newline tokens delimit).
    // A Str token containing newlines spans lines but is atomic; a line
    // "contains only this string" if the Newline-delimited slice around it
    // has no other significant token AND the Str has no siblings on its
    // first/last physical line. Approximation: check the Newline-delimited
    // slice for exactly one significant token.
    let mut line_starts: Vec<usize> = vec![0];
    for (i, t) in toks.iter().enumerate() {
        if t.kind == TokKind::Newline {
            line_starts.push(i + 1);
        }
    }
    // For each slice, count significant tokens.
    for w in 0..line_starts.len() {
        let s = line_starts[w];
        let e = if w + 1 < line_starts.len() {
            line_starts[w + 1]
        } else {
            toks.len() + 1
        };
        // indices [s, e) in token space, where e-1 may be the Newline.
        let end = e.min(toks.len() + 1).saturating_sub(1);
        let mut sig: Vec<usize> = vec![];
        for i in s..end.min(toks.len()) {
            match toks[i].kind {
                TokKind::Whitespace | TokKind::Comment | TokKind::Newline => {}
                _ => sig.push(i),
            }
        }
        if sig.len() == 1 && toks[sig[0]].kind == TokKind::Str {
            let idx = sig[0];
            // Preserve the token's leading indent (Whitespace before it stays).
            toks[idx].kind = TokKind::Ident;
            toks[idx].text = "pass".to_string();
        }
    }
}

fn prev_sig(toks: &[Token], mut idx: usize) -> Option<usize> {
    if idx == 0 {
        return None;
    }
    idx -= 1;
    loop {
        match toks[idx].kind {
            TokKind::Whitespace | TokKind::Comment | TokKind::Newline => {
                if idx == 0 {
                    return None;
                }
                idx -= 1;
            }
            _ => return Some(idx),
        }
    }
}

fn next_sig(toks: &[Token], mut idx: usize) -> Option<usize> {
    idx += 1;
    while idx < toks.len() {
        match toks[idx].kind {
            TokKind::Whitespace | TokKind::Comment | TokKind::Newline => idx += 1,
            _ => return Some(idx),
        }
    }
    None
}

fn is_dot(idx: usize, toks: &[Token]) -> bool {
    toks[idx].kind == TokKind::Op && toks[idx].text == "."
}

/// Token indices that must keep literal values: `case` patterns in match
/// statements only allow literals. A decoder call there is parsed as a class
/// pattern (TypeError at runtime), an `(0xA^0xB)` is a syntax error, and a
/// bool alias name becomes a capture pattern (silently wrong). Guard
/// expressions (`case P if ...`) are real expressions and stay transformable.
pub fn case_protected(toks: &[Token]) -> HashSet<usize> {
    let mut prot = HashSet::new();
    let mut i = 0;
    while i < toks.len() {
        if toks[i].kind == TokKind::Ident && toks[i].text == "case" && is_line_start(toks, i) {
            let mut depth = 0i32;
            let mut j = i + 1;
            while j < toks.len() {
                let t = &toks[j];
                match t.kind {
                    TokKind::Newline if depth == 0 => break,
                    TokKind::Op => match t.text.as_str() {
                        "(" | "[" | "{" => depth += 1,
                        ")" | "]" | "}" => depth -= 1,
                        ":" if depth == 0 => break,
                        _ => {}
                    },
                    TokKind::Ident if t.text == "if" && depth == 0 => break, // guard
                    TokKind::Str | TokKind::Number => {
                        prot.insert(j);
                    }
                    TokKind::Ident if t.text == "True" || t.text == "False" || t.text == "None" => {
                        prot.insert(j);
                    }
                    _ => {}
                }
                j += 1;
            }
            i = j;
            continue;
        }
        i += 1;
    }
    prot
}

fn is_line_start(toks: &[Token], idx: usize) -> bool {
    let mut k = idx;
    while k > 0 {
        k -= 1;
        match toks[k].kind {
            TokKind::Whitespace | TokKind::Comment => continue,
            TokKind::Newline => return true,
            _ => return false,
        }
    }
    true
}

/// XOR `plain` with an LCG keystream. Mirrors the injected Python decoder
/// exactly (one LCG step per byte, high byte used).
fn xor_keystream(plain: &[u8], key: u32) -> Vec<u8> {
    let mut x = key;
    let mut out = Vec::with_capacity(plain.len());
    for &b in plain {
        x = x.wrapping_mul(1103515245).wrapping_add(12345);
        out.push(b ^ ((x >> 16) as u8));
    }
    out
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Byte ranges for chunking a string without splitting UTF-8 sequences.
fn chunk_bounds(s: &str, max: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut cur = 0usize;
    for (i, ch) in s.char_indices() {
        let l = ch.len_utf8();
        if cur + l > max && i > start {
            out.push((start, i));
            start = i;
            cur = 0;
        }
        cur += l;
    }
    if start < s.len() {
        out.push((start, s.len()));
    }
    out
}

/// Encrypt eligible string literals in place. Returns count.
///
/// Long strings are split into runtime-joined chunks; the whole expression is
/// parenthesised when chunked so surrounding operators keep their precedence.
pub fn encrypt_strings(
    toks: &mut [Token],
    decoder: &str,
    key: u32,
    min_len: usize,
    prot: &HashSet<usize>,
) -> usize {
    // Adjacent literals are implicitly concatenated by Python. Replacing only
    // one side would leave `_d("..") ".."` (a syntax error), so skip chains.
    let mut adj = vec![false; toks.len()];
    for i in 0..toks.len() {
        if toks[i].kind == TokKind::Str
            && (prev_sig(toks, i).is_some_and(|p| toks[p].kind == TokKind::Str)
                || next_sig(toks, i).is_some_and(|n| toks[n].kind == TokKind::Str))
        {
            adj[i] = true;
        }
    }
    let mut count = 0;
    for i in 0..toks.len() {
        if toks[i].kind != TokKind::Str || prot.contains(&i) || adj[i] {
            continue;
        }
        let text = toks[i].text.clone();
        let Some((prefix, _quote, body)) = split_string(&text) else {
            continue;
        };
        let lp = prefix.to_lowercase();
        if lp.contains('f') || lp.contains('b') {
            continue; // f-strings (expressions) and bytes: skip.
        }
        let is_raw = lp.contains('r');
        if !is_raw && body.contains('\\') {
            continue; // conservative: skip escape-heavy strings.
        }
        if body.chars().count() < min_len || body.is_empty() {
            continue;
        }
        let parts: Vec<String> = chunk_bounds(&body, 96)
            .into_iter()
            .map(|(a, b)| {
                let enc = to_hex(&xor_keystream(body[a..b].as_bytes(), key));
                format!("{decoder}(\"{enc}\")")
            })
            .collect();
        toks[i].text = if parts.len() == 1 {
            parts.into_iter().next().unwrap()
        } else {
            format!("({})", parts.join("+"))
        };
        count += 1;
    }
    count
}

fn is_plain_decimal_int(text: &str) -> Option<u64> {
    if text.is_empty() {
        return None;
    }
    if !text.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return None;
    }
    if text.starts_with('_') || text.ends_with('_') || text.contains("__") {
        return None;
    }
    let clean: String = text.chars().filter(|&c| c != '_').collect();
    if clean.is_empty() || clean.len() > 19 {
        return None;
    }
    // Leading zeros (`007`) are a syntax error in py3 anyway; encoding them
    // would FIX the file — still fine to encode, but skip `0` itself noise?
    clean.parse::<u64>().ok()
}

/// Replace decimal ints with `(0xA^0xB)` pairs. Returns count.
pub fn encode_numbers(toks: &mut [Token], seed: u64, prot: &HashSet<usize>) -> usize {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x1234_5678_9ABC_DEF0);
    let mut count = 0;
    for i in 0..toks.len() {
        if toks[i].kind != TokKind::Number || prot.contains(&i) {
            continue;
        }
        // Don't touch numbers directly after a dot (`x.5` lexes whole, but
        // `5 .bit_length`? `5.` is float token; plain guard below suffices).
        if prev_sig(toks, i).map(|p| is_dot(p, toks)).unwrap_or(false) {
            continue;
        }
        let Some(v) = is_plain_decimal_int(&toks[i].text) else {
            continue;
        };
        let a: u64 = rng.random_range(0..=0xFFFFFF);
        let b = v ^ a;
        toks[i].text = format!("(0x{a:x}^0x{b:x})");
        count += 1;
    }
    count
}

/// Replace True/False/None idents with header aliases. Returns count.
pub fn replace_bool_none(
    toks: &mut [Token],
    true_alias: &str,
    false_alias: &str,
    none_alias: &str,
    prot: &HashSet<usize>,
) -> usize {
    let mut count = 0;
    for i in 0..toks.len() {
        if toks[i].kind != TokKind::Ident || prot.contains(&i) {
            continue;
        }
        if prev_sig(toks, i).map(|p| is_dot(p, toks)).unwrap_or(false) {
            continue;
        }
        if toks[i].text == "True" {
            toks[i].text = true_alias.to_string();
            count += 1;
        } else if toks[i].text == "False" {
            toks[i].text = false_alias.to_string();
            count += 1;
        } else if toks[i].text == "None" {
            toks[i].text = none_alias.to_string();
            count += 1;
        }
    }
    count
}

pub fn render(toks: &[Token]) -> String {
    let mut s = String::new();
    for t in toks {
        s.push_str(&t.text);
    }
    s
}

/// Line-based post-processing: trim trailing whitespace, collapse 3+ blank
/// lines, ensure single trailing newline. Returns cleaned string.
pub fn clean_lines(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut blanks = 0;
    for line in s.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            blanks += 1;
            if blanks <= 2 {
                out.push(String::new());
            }
            continue;
        }
        blanks = 0;
        out.push(trimmed.to_string());
    }
    let mut res = out.join("\n");
    res.push('\n');
    res
}

/// Split off shebang (`#!...`) so the header can go after it.
pub fn split_shebang(src: &str) -> (Option<String>, String) {
    if src.starts_with("#!") {
        if let Some(pos) = src.find('\n') {
            return (Some(src[..pos].to_string()), src[pos + 1..].to_string());
        }
        return (Some(src.to_string()), String::new());
    }
    (None, src.to_string())
}

/// Find the last leading `from __future__ import ...` line index (0-based).
/// Header must go after these (Python requires them first).
pub fn future_import_end(lines: &[&str]) -> Option<usize> {
    let mut last: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with("from __future__ import ") {
            last = Some(i);
            continue;
        }
        break;
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints_encode_and_brackets_safe() {
        let empty = HashSet::new();
        let mut toks = crate::lexer::tokenize("x = 42 + 0x1F + 3.5\n");
        let n = encode_numbers(&mut toks, 7, &empty);
        assert_eq!(n, 1); // only the decimal 42
        let s = render(&toks);
        assert!(s.contains("(0x"));
        assert!(s.contains("0x1F"));
        assert!(s.contains("3.5"));
    }

    #[test]
    fn backslash_strings_skipped() {
        let empty = HashSet::new();
        let mut toks = crate::lexer::tokenize("a = \"a\\nb\"\nb = \"plain\"\n");
        let n = encrypt_strings(&mut toks, "__d", 42, 1, &empty);
        assert_eq!(n, 1);
    }

    #[test]
    fn implicit_concat_chain_stays_intact() {
        let empty = HashSet::new();
        let mut toks = crate::lexer::tokenize("x = \"ab\" \"cd\"\ny = \"solo\"\n");
        let n = encrypt_strings(&mut toks, "__d", 7, 1, &empty);
        assert_eq!(n, 1); // only the solo string
        let rendered: String = toks.iter().map(|t| t.text.clone()).collect();
        assert!(rendered.contains("\"ab\" \"cd\""));
    }

    #[test]
    fn long_strings_chunked_and_parenthesised() {
        let empty = HashSet::new();
        let long = "a".repeat(250);
        let src = format!("x = \"{long}\" % ()\n");
        let mut toks = crate::lexer::tokenize(&src);
        let n = encrypt_strings(&mut toks, "__d", 7, 1, &empty);
        assert_eq!(n, 1);
        let rendered: String = toks.iter().map(|t| t.text.clone()).collect();
        assert!(rendered.contains("x = (__d("));
        assert!(rendered.contains(")+__d("));
        assert!(rendered.ends_with(") % ()\n") || rendered.contains(") % ()"));
    }

    #[test]
    fn keystream_roundtrip_matches_decoder_shape() {
        // Rust encoder must mirror the Python decoder: hex of LCG-xored bytes.
        let key = 0xDEADBEEFu32;
        let plain = b"hello world";
        let enc = to_hex(&xor_keystream(plain, key));
        assert_eq!(enc.len(), plain.len() * 2);
        assert!(enc.chars().all(|c| c.is_ascii_hexdigit()));
        // Re-decrypting with the same step order must restore the input.
        let mut x = key;
        let mut out = Vec::new();
        for i in 0..plain.len() {
            x = x.wrapping_mul(1103515245).wrapping_add(12345);
            let b = u8::from_str_radix(&enc[i * 2..i * 2 + 2], 16).unwrap();
            out.push(b ^ ((x >> 16) as u8));
        }
        assert_eq!(out, plain);
    }

    #[test]
    fn case_patterns_protected() {
        let src = "match v:\n    case 42:\n        a = \"lit\"\n    case True:\n        b = 1\n";
        let mut toks = crate::lexer::tokenize(src);
        let prot = case_protected(&toks);
        let ns = encrypt_strings(&mut toks, "__d", 42, 1, &prot);
        assert_eq!(ns, 1); // only the body string, not the pattern
        let nn = encode_numbers(&mut toks, 7, &prot);
        assert_eq!(nn, 1); // only `b = 1`
        let nb = replace_bool_none(&mut toks, "T", "F", "N", &prot);
        assert_eq!(nb, 0); // `case True` untouched
    }
}
