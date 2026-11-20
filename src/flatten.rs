//! Control-flow flattening for straight-line function bodies.
//!
//! A selected body like
//!
//! ```python
//! def f(a):
//!     x = a + 1
//!     y = x * 2
//!     return y
//! ```
//!
//! becomes a randomised state machine:
//!
//! ```python
//! def f(a):
//!     _s = 0x4f2
//!     _r = None
//!     while _s >= 0:
//!         if _s == 0x4f2:
//!             x = a + 1
//!             _s = 0x91c
//!         elif _s == 0x91c:
//!             y = x * 2
//!             _s = -1
//!         else:
//!             _s = -1
//!     return _r
//! ```
//!
//! Only bodies that are one flat run of simple statements qualify. Anything
//! with nested suites (`if`/`for`/`while`/`try`/`with`/`match`/`def`/`class`),
//! `yield`/`await`, `;`, or backslash continuations is left untouched, so the
//! transform never has to reason about `break`/`continue`/`finally`.
//!
//! Semantics preserved: `return expr` becomes `_r = expr; _s = -1` and the
//! value is returned after the loop; bare `return` assigns the None alias.
//! Falling off the end of the body returns `_r` (None unless overwritten),
//! which matches implicit `return None`.

use crate::lexer::{TokKind, Token};
use crate::transform::unique_name;
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashSet;

struct Line {
    /// One past the last token of this physical line.
    end: usize,
    /// Leading whitespace text (empty at column 0).
    indent: String,
    /// Significant token indices (Ident/Number/Str/Op), in order.
    sig: Vec<usize>,
}

impl Line {
    fn is_blank(&self) -> bool {
        self.sig.is_empty()
    }
}

struct Plan {
    def_line: usize,
    body_last: usize,
    body_indent: String,
    stmts: Vec<(usize, usize)>,
}

/// Group tokens into physical lines; newlines inside brackets do not split.
fn split_lines(toks: &[Token]) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut cur = Line {
        end: 0,
        indent: String::new(),
        sig: Vec::new(),
    };
    let mut depth = 0i32;
    let mut at_start = true;
    for (i, t) in toks.iter().enumerate() {
        if at_start {
            if t.kind == TokKind::Whitespace {
                cur.indent = t.text.clone();
                at_start = false;
                continue;
            }
            at_start = false;
        }
        match t.kind {
            TokKind::Newline if depth == 0 => {
                cur.end = i + 1;
                lines.push(std::mem::replace(
                    &mut cur,
                    Line {
                        end: i + 1,
                        indent: String::new(),
                        sig: Vec::new(),
                    },
                ));
                at_start = true;
            }
            TokKind::Whitespace | TokKind::Comment | TokKind::Newline => {}
            TokKind::Op => {
                match t.text.as_str() {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {}
                }
                cur.sig.push(i);
            }
            _ => cur.sig.push(i),
        }
    }
    if cur.end < toks.len() || !cur.sig.is_empty() {
        cur.end = toks.len();
        lines.push(cur);
    }
    lines
}

/// Is this line a `def`/`async def` header that is self-contained on one
/// physical line (ends with `:` and has nothing after it)?
fn is_def_line(toks: &[Token], line: &Line) -> bool {
    let Some(&first) = line.sig.first() else {
        return false;
    };
    let t = &toks[first];
    if t.kind != TokKind::Ident {
        return false;
    }
    let has_def = if t.text == "def" {
        true
    } else if t.text == "async" {
        line.sig
            .get(1)
            .map(|&i| toks[i].kind == TokKind::Ident && toks[i].text == "def")
            .unwrap_or(false)
    } else {
        false
    };
    if !has_def {
        return false;
    }
    let Some(&last) = line.sig.last() else {
        return false;
    };
    toks[last].kind == TokKind::Op && toks[last].text == ":"
}

/// Flat statements only: no nested suites, generators, `;`, or `\` joins.
fn stmt_allowed(toks: &[Token], line: &Line) -> bool {
    let first = &toks[line.sig[0]];
    if first.kind == TokKind::Op && first.text == "@" {
        return false;
    }
    if first.kind == TokKind::Ident {
        match first.text.as_str() {
            "def" | "class" | "async" | "if" | "elif" | "else" | "for" | "while" | "try"
            | "except" | "finally" | "with" | "match" | "case" => return false,
            _ => {}
        }
    }
    for &i in &line.sig {
        let t = &toks[i];
        if t.kind == TokKind::Op && (t.text == ";" || t.text == "\\") {
            return false;
        }
        if t.kind == TokKind::Ident && (t.text == "yield" || t.text == "await") {
            return false;
        }
    }
    true
}

fn ptok(out: &mut Vec<Token>, text: String) {
    out.push(Token {
        kind: TokKind::Op,
        text,
    });
}

pub fn flatten(
    toks: Vec<Token>,
    rng: &mut ChaCha8Rng,
    used: &mut HashSet<String>,
    none_alias: &str,
) -> (Vec<Token>, usize) {
    let lines = split_lines(&toks);
    let mut plans: Vec<Plan> = Vec::new();
    let mut li = 0usize;
    while li < lines.len() {
        if !is_def_line(&toks, &lines[li]) {
            li += 1;
            continue;
        }
        let hdr_indent = lines[li].indent.chars().count();
        let mut b = li + 1;
        while b < lines.len() && lines[b].is_blank() {
            b += 1;
        }
        if b >= lines.len() {
            li += 1;
            continue;
        }
        if lines[b].indent.chars().count() <= hdr_indent
            || !lines[b].indent.starts_with(&lines[li].indent)
        {
            li += 1;
            continue;
        }
        let body_indent = lines[b].indent.clone();
        let mut stmts: Vec<(usize, usize)> = Vec::new();
        let mut last = b;
        let mut ok = true;
        while b < lines.len() {
            if lines[b].is_blank() {
                b += 1;
                continue;
            }
            if lines[b].indent.chars().count() <= hdr_indent {
                break;
            }
            if lines[b].indent != body_indent || !stmt_allowed(&toks, &lines[b]) {
                ok = false;
                break;
            }
            stmts.push((lines[b].sig[0], *lines[b].sig.last().unwrap()));
            last = b;
            b += 1;
        }
        if ok && (3..=40).contains(&stmts.len()) {
            plans.push(Plan {
                def_line: li,
                body_last: last,
                body_indent,
                stmts,
            });
            li = b;
        } else {
            li += 1;
        }
    }

    if plans.is_empty() {
        return (toks, 0);
    }

    let s_name = unique_name(rng, used);
    let r_name = unique_name(rng, used);

    let mut out: Vec<Token> = Vec::with_capacity(toks.len() + plans.len() * 64);
    let mut cursor = 0usize;
    let mut count = 0usize;
    for plan in &plans {
        let header_end = lines[plan.def_line].end;
        if cursor < header_end {
            out.extend_from_slice(&toks[cursor..header_end]);
        }
        emit_body(&mut out, &toks, plan, &s_name, &r_name, none_alias, rng);
        cursor = lines[plan.body_last].end;
        count += 1;
    }
    if cursor < toks.len() {
        out.extend_from_slice(&toks[cursor..]);
    }
    (out, count)
}

fn emit_body(
    out: &mut Vec<Token>,
    toks: &[Token],
    plan: &Plan,
    s: &str,
    r: &str,
    none_alias: &str,
    rng: &mut ChaCha8Rng,
) {
    let n = plan.stmts.len();
    let mut states: Vec<u32> = Vec::with_capacity(n);
    while states.len() < n {
        let v: u32 = rng.random_range(1..=0x7FFF_FFFF);
        if !states.contains(&v) {
            states.push(v);
        }
    }
    // Randomised arm order: dispatch is by equality, so order is irrelevant —
    // except for `global`/`nonlocal`, which Python requires to appear
    // textually before any use of the name. Those arms stay first (in their
    // original relative order); the rest are shuffled freely.
    let mut decls: Vec<usize> = Vec::new();
    let mut rest: Vec<usize> = Vec::new();
    for (k, &(a, _)) in plan.stmts.iter().enumerate() {
        let t = &toks[a];
        if t.kind == TokKind::Ident && (t.text == "global" || t.text == "nonlocal") {
            decls.push(k);
        } else {
            rest.push(k);
        }
    }
    for i in (1..rest.len()).rev() {
        let j = rng.random_range(0..=i);
        rest.swap(i, j);
    }
    let mut order = decls;
    order.extend(rest);
    let bi = &plan.body_indent;
    let i4 = format!("{bi}    ");
    let i8 = format!("{bi}        ");
    ptok(out, format!("{bi}{s}={:#x}\n", states[0]));
    ptok(out, format!("{bi}{r}={none_alias}\n"));
    ptok(out, format!("{bi}while {s}>=0:\n"));
    for (pos, &k) in order.iter().enumerate() {
        let kw = if pos == 0 { "if" } else { "elif" };
        ptok(out, format!("{i4}{kw} {s}=={:#x}:\n", states[k]));
        let (a, b) = plan.stmts[k];
        let is_return = toks[a].kind == TokKind::Ident && toks[a].text == "return";
        if is_return {
            if a == b {
                ptok(out, format!("{i8}{r}={none_alias}\n"));
            } else {
                ptok(out, format!("{i8}{r}="));
                // Skip the whitespace between `return` and its expression.
                let mut s0 = a + 1;
                while s0 < b && toks[s0].kind == TokKind::Whitespace {
                    s0 += 1;
                }
                out.extend_from_slice(&toks[s0..=b]);
                ptok(out, "\n".to_string());
            }
            ptok(out, format!("{i8}{s}=-1\n"));
        } else {
            ptok(out, i8.clone());
            out.extend_from_slice(&toks[a..=b]);
            ptok(out, "\n".to_string());
            let next = if k + 1 < n {
                format!("{:#x}", states[k + 1])
            } else {
                "-1".to_string()
            };
            ptok(out, format!("{i8}{s}={next}\n"));
        }
    }
    ptok(out, format!("{i4}else:\n"));
    ptok(out, format!("{i8}{s}=-1\n"));
    ptok(out, format!("{bi}return {r}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn run(src: &str) -> (String, usize) {
        let toks = crate::lexer::tokenize(src);
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let mut used = HashSet::new();
        let (out, n) = flatten(toks, &mut rng, &mut used, "None");
        (crate::transform::render(&out), n)
    }

    #[test]
    fn flattens_flat_function() {
        let src = "def f(a):\n    x = a + 1\n    y = x * 2\n    return y\n";
        let (text, n) = run(src);
        assert_eq!(n, 1);
        assert!(text.contains("def f(a):"));
        assert!(text.contains("while "));
        assert!(text.contains("return "));
        assert!(!text.contains("\n    x = a + 1\n"));
    }

    #[test]
    fn bails_on_nested_block() {
        let src = "def f(a):\n    x = a\n    if x:\n        x = 1\n    return x\n";
        let (text, n) = run(src);
        assert_eq!(n, 0);
        assert_eq!(text, src);
    }

    #[test]
    fn bails_on_short_body() {
        let src = "def f(a):\n    x = a\n    return x\n";
        let (_, n) = run(src);
        assert_eq!(n, 0);
    }

    #[test]
    fn bails_on_generator() {
        let src = "def f(n):\n    x = n\n    y = x + 1\n    yield y\n";
        let (_, n) = run(src);
        assert_eq!(n, 0);
    }

    #[test]
    fn return_expression_is_captured() {
        let src = "def f(a, b):\n    c = a + b\n    d = c * 3\n    return d + 1\n";
        let (text, n) = run(src);
        assert_eq!(n, 1);
        // The original expression must survive inside the state machine.
        assert!(text.contains("d + 1"));
        assert!(!text.contains("return d + 1"));
    }

    #[test]
    fn multiline_statement_stays_one_unit() {
        let src = "def f(a, b):\n    x = a + b\n    y = max(\n        x,\n        a,\n    )\n    return y\n";
        let (text, n) = run(src);
        assert_eq!(n, 1);
        assert!(text.contains("max("));
    }

    #[test]
    fn nonlocal_declaration_stays_before_uses() {
        let src = "def o():\n    n = 1\n    def i():\n        nonlocal n\n        n += 1\n        n *= 2\n        return n\n    return i\n";
        let (text, n) = run(src);
        assert_eq!(n, 1);
        let decl = text.find("nonlocal ").expect("nonlocal kept");
        let use1 = text.find("n += 1").expect("stmt kept");
        let use2 = text.find("n *= 2").expect("stmt kept");
        assert!(
            decl < use1 && decl < use2,
            "declaration must precede uses:\n{text}"
        );
    }

    #[test]
    fn global_declaration_stays_before_uses() {
        let src = "def b():\n    global g\n    g += 1\n    g *= 2\n    return g\n";
        let (text, n) = run(src);
        assert_eq!(n, 1);
        let decl = text.find("global ").expect("global kept");
        let use1 = text.find("g += 1").expect("stmt kept");
        assert!(decl < use1, "declaration must precede uses:\n{text}");
    }
}
