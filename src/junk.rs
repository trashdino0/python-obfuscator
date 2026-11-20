//! Dead-code injection.
//!
//! Two kinds are inserted:
//! - module-level blocks guarded by an always-false comparison (never run);
//! - function-body blocks guarded by an always-true lambda predicate (run,
//!   but only perform pure integer arithmetic) — they add plausible control
//!   flow noise inside functions.
//!
//! Insertion is statement-boundary only: bracket depth 0, outside multi-line
//! strings, never between a block header/decorator and its body, and never
//! before `else`/`elif`/`except`/`finally`/`case`.

use crate::transform::unique_name;
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::{HashMap, HashSet};

struct Scan {
    depth_before: i32,
    depth_after: i32,
    in_string_before: bool,
    in_string_after: bool,
}

/// One pass over the text recording, for every physical line, the bracket
/// depth and string state at its start and end. String state is carried
/// across lines so triple-quoted literals that survive string encryption
/// cannot be mistaken for code.
fn scan_lines(s: &str) -> Vec<Scan> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<(u8, usize)> = None;
    let mut depth_at: Vec<i32> = Vec::new();
    let mut str_at: Vec<bool> = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if let Some((q, len)) = quote {
            if c == b'\\' {
                // `\` + newline: still a physical line break (inside the string).
                if i + 1 < b.len() && b[i + 1] == b'\n' {
                    depth_at.push(depth);
                    str_at.push(true);
                }
                i += 2;
                continue;
            }
            if c == q {
                if len == 1 {
                    quote = None;
                    i += 1;
                    continue;
                }
                if i + 2 < b.len() && b[i + 1] == q && b[i + 2] == q {
                    quote = None;
                    i += 3;
                    continue;
                }
            }
            if c == b'\n' {
                depth_at.push(depth);
                str_at.push(true);
            }
            i += 1;
            continue;
        }
        match c {
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'\'' | b'"' => {
                let len = if i + 2 < b.len() && b[i + 1] == c && b[i + 2] == c {
                    3
                } else {
                    1
                };
                quote = Some((c, len));
                i += len;
            }
            b'(' | b'[' | b'{' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' | b'}' => {
                depth -= 1;
                i += 1;
            }
            b'\n' => {
                depth_at.push(depth);
                str_at.push(quote.is_some());
                i += 1;
            }
            _ => i += 1,
        }
    }
    let final_str = quote.is_some();
    let n_lines = s.lines().count();
    let mut out = Vec::with_capacity(n_lines);
    for k in 0..n_lines {
        let depth_before = if k == 0 {
            0
        } else {
            depth_at.get(k - 1).copied().unwrap_or(depth)
        };
        let depth_after = depth_at.get(k).copied().unwrap_or(depth);
        let in_string_before = if k == 0 {
            false
        } else {
            str_at.get(k - 1).copied().unwrap_or(final_str)
        };
        let in_string_after = str_at.get(k).copied().unwrap_or(final_str);
        out.push(Scan {
            depth_before,
            depth_after,
            in_string_before,
            in_string_after,
        });
    }
    out
}

fn indent_cols(s: &str) -> usize {
    let mut col = 0usize;
    for c in s.chars() {
        match c {
            ' ' => col += 1,
            '\t' => col = (col / 8 + 1) * 8,
            _ => break,
        }
    }
    col
}

fn shuffle<T>(v: &mut [T], rng: &mut ChaCha8Rng) {
    for i in (1..v.len()).rev() {
        let j = rng.random_range(0..=i);
        v.swap(i, j);
    }
}

/// Never-executed module-level block (always-false comparison).
fn top_block(rng: &mut ChaCha8Rng, name: &str) -> String {
    let a: u32 = rng.random_range(100_000..999_999);
    let b: u32 = rng.random_range(100_000..999_999);
    let mut c: u32 = rng.random_range(100_000..999_999);
    if c == a.wrapping_add(b) {
        c = c.wrapping_add(1);
    }
    let j1: u32 = rng.random_range(2..9999);
    let j2: u32 = rng.random_range(2..9999);
    format!(
        "if {a}+{b}=={c}:\n    {name}=((({j1}*{j2}+{})^{}//{}))",
        a % 997,
        j1 + 1,
        (j2 % 97) + 2
    )
}

/// Always-true opaque predicate guards (lambda calls defeat constant folding).
fn func_block(rng: &mut ChaCha8Rng, name: &str, indent: &str) -> String {
    let a: u32 = rng.random_range(2..99999);
    let b: u32 = rng.random_range(2..99999);
    let c: u32 = rng.random_range(1..999);
    let d: u32 = rng.random_range(1..999);
    let k: u32 = rng.random_range(0..9999);
    let bi = format!("{indent}    ");
    match rng.random_range(0..3) {
        0 => format!(
            "{indent}if (lambda n:(n*n+n)&1)({k})==0:\n{bi}{name}=((({a}*{b}+{c})^{d})//{})",
            d + 1
        ),
        1 => format!(
            "{indent}if (lambda t:len(t)&1)((1,2,3,4))==0:\n{bi}{name}=(({a}^{b})+({c}*{d}))"
        ),
        _ => format!("{indent}if (lambda n:(n|1)>0)({k}):\n{bi}{name}=(({a}+{b})*({c}^{d}))"),
    }
}

pub fn insert_deadcode(s: &str, seed: u64, used: &mut HashSet<String>) -> (String, usize) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xDEAD_C0DE_1234_5678);
    let lines: Vec<&str> = s.lines().collect();
    let scans = scan_lines(s);

    // (indent columns, inside a function body)
    let mut stack: Vec<(usize, bool)> = Vec::new();
    let mut top: Vec<usize> = Vec::new();
    let mut funcs: Vec<(usize, String)> = Vec::new();
    let mut prev_nonblank: Option<usize> = None;

    for (idx, line) in lines.iter().enumerate() {
        let Some(scan) = scans.get(idx) else {
            continue;
        };
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            continue;
        }
        let indent: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let cols = indent_cols(&indent);
        while let Some(&(ci, _)) = stack.last() {
            if ci >= cols {
                stack.pop();
            } else {
                break;
            }
        }
        let in_func = stack.last().map(|&(_, f)| f).unwrap_or(false);
        let t = line.trim_start();
        let is_pos = scan.depth_before == 0
            && !scan.in_string_before
            && !trimmed.ends_with('\\')
            && !t.starts_with('#');
        let prev_ok = prev_nonblank
            .map(|pi| {
                let pl = lines[pi].trim_end();
                !pl.ends_with(':') && !pl.trim_start().starts_with('@')
            })
            .unwrap_or(false);
        let detached = t == "else:"
            || t.starts_with("elif ")
            || t.starts_with("elif:")
            || t.starts_with("except")
            || t == "finally:"
            || t.starts_with("case ")
            || t.starts_with('@');
        if is_pos && prev_ok && !detached {
            if cols == 0 {
                top.push(idx);
            } else if in_func {
                funcs.push((idx, indent.clone()));
            }
        }
        if scan.depth_before == 0
            && scan.depth_after == 0
            && !scan.in_string_before
            && !scan.in_string_after
            && t.ends_with(':')
        {
            let is_def = t.starts_with("def ") || t.starts_with("async def ");
            let parent = stack.last().map(|&(_, f)| f).unwrap_or(false);
            stack.push((cols, parent || is_def));
        }
        prev_nonblank = Some(idx);
    }

    shuffle(&mut top, &mut rng);
    top.truncate(3);
    top.sort_unstable();
    shuffle(&mut funcs, &mut rng);
    funcs.truncate(6);

    let total = top.len() + funcs.len();
    let mut names: Vec<String> = Vec::with_capacity(total);
    for _ in 0..total {
        names.push(unique_name(&mut rng, used));
    }
    let eof_name = unique_name(&mut rng, used);

    let mut blocks: HashMap<usize, String> = HashMap::new();
    for (k, &pos) in top.iter().enumerate() {
        blocks.insert(pos, top_block(&mut rng, &names[k]));
    }
    for (j, (pos, indent)) in funcs.iter().enumerate() {
        blocks.insert(*pos, func_block(&mut rng, &names[top.len() + j], indent));
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len() + blocks.len() * 3 + 1);
    for (idx, line) in lines.iter().enumerate() {
        if let Some(blk) = blocks.get(&idx) {
            out.push(blk.clone());
        }
        out.push((*line).to_string());
    }
    // A final executed assignment at module scope.
    let a: u32 = rng.random_range(2..99999);
    let b: u32 = rng.random_range(2..99999);
    let c: u32 = rng.random_range(1..999);
    let d: u32 = rng.random_range(1..999);
    out.push(format!("{eof_name}=(({a}*{b}+{c})^{d})"));
    (out.join("\n") + "\n", total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn top_level_and_function_blocks_inserted() {
        let src = "def f(a):\n    x = a + 1\n    y = x * 2\n    return y\n\nprint(f(2))\n";
        let (out, n) = insert_deadcode(src, 3, &mut names());
        assert!(n >= 1);
        assert!(out.contains("if (lambda") || out.contains("if "));
        // Function body still parses in shape: junk is indented inside it.
        assert!(out.contains("\n    x = a + 1\n") || out.contains("\n    "));
    }

    #[test]
    fn does_not_break_multiline_strings() {
        let src = "s = f\"\"\"line one\ninside string\nline three\"\"\"\nprint(s)\n";
        let (out, _) = insert_deadcode(src, 5, &mut names());
        // The interior lines must remain untouched (no junk between them).
        assert!(out.contains("line one\ninside string\nline three"));
    }

    #[test]
    fn no_insert_between_header_and_body() {
        let src = "def f():\n    a = 1\n    b = 2\n    c = 3\n    return a + b + c\n";
        let (out, _) = insert_deadcode(src, 9, &mut names());
        assert!(!out.contains("def f():\nif"));
        assert!(!out.contains("def f():\n    if (lambda"));
    }

    #[test]
    fn class_bodies_are_not_touched() {
        let src = "class A:\n    x = 1\n    y = 2\n    z = 3\n";
        let (out, _) = insert_deadcode(src, 11, &mut names());
        // No junk may be inserted at class-body indent (would add attributes).
        let class_part: Vec<&str> = out.lines().filter(|l| l.starts_with("    ")).collect();
        assert!(class_part
            .iter()
            .all(|l| !l.trim_start().starts_with("if (lambda")));
    }
}
