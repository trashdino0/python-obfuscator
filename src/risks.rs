//! Static risk detector: finds patterns whose behaviour can change under
//! renaming/transforms because they resolve names dynamically. These are
//! warnings, not errors — the code may still be fine (e.g. the dynamic name
//! happens to be in `--keep`).

use crate::lexer::{TokKind, Token};

pub struct Risk {
    pub line: usize,
    pub msg: String,
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

fn is_dot(idx: usize, toks: &[Token]) -> bool {
    toks[idx].kind == TokKind::Op && toks[idx].text == "."
}

/// Index of the `(` if the identifier at `i` is a call callee.
fn call_paren(toks: &[Token], i: usize) -> Option<usize> {
    let n = next_sig(toks, i)?;
    if toks[n].kind == TokKind::Op && toks[n].text == "(" {
        Some(n)
    } else {
        None
    }
}

/// Does the call opened at `open` contain a depth-1 `name=` keyword argument?
fn has_kwarg(toks: &[Token], open: usize) -> bool {
    let mut depth = 0i32;
    let mut j = open;
    let mut in_lambda = false;
    while j < toks.len() {
        let t = &toks[j];
        if t.kind == TokKind::Op {
            match t.text.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                ":" if depth == 1 => in_lambda = false,
                _ => {}
            }
        }
        if depth == 1 && t.kind == TokKind::Ident {
            if t.text == "lambda" {
                in_lambda = true;
            } else if !in_lambda {
                if let Some(ni) = next_sig(toks, j) {
                    if toks[ni].kind == TokKind::Op && toks[ni].text == "=" {
                        return true;
                    }
                }
            }
        }
        j += 1;
    }
    false
}

/// Scan for risk patterns. `line` numbers are 1-based (post-shebang).
pub fn detect(toks: &[Token]) -> Vec<Risk> {
    let mut out: Vec<Risk> = Vec::new();
    let mut line = 1usize;
    for i in 0..toks.len() {
        let t = &toks[i];
        if t.kind == TokKind::Newline {
            line += 1;
            continue;
        }
        if t.kind != TokKind::Ident {
            continue;
        }
        let bare = prev_sig(toks, i).map(|p| !is_dot(p, toks)).unwrap_or(true);
        let call = call_paren(toks, i);
        let mut push = |msg: String| {
            if !out.iter().any(|r| r.line == line && r.msg == msg) && out.len() < 50 {
                out.push(Risk { line, msg });
            }
        };
        match t.text.as_str() {
            "eval" | "exec" | "compile" if bare && call.is_some() => {
                push(format!(
                    "{}(...) builds/executes code at runtime — names constructed in strings are not renamed (use --keep or --no-rename for this file)",
                    t.text
                ));
            }
            "globals" | "locals" if bare && call.is_some() => {
                push(format!(
                    "{}() is string-keyed at runtime — lookups like {}()[...] break under renaming",
                    t.text, t.text
                ));
            }
            "get_type_hints" if call.is_some() => {
                push(
                    "get_type_hints() resolves string annotations by name — \"...\"-annotations are not rewritten".to_string(),
                );
            }
            "partial" if call.is_some() => {
                // Both `partial(...)` and `functools.partial(...)`.
                if let Some(p) = call {
                    if has_kwarg(toks, p) {
                        push(
                            "functools.partial(..., kw=...) forwards kwarg names as data — they are not renamed".to_string(),
                        );
                    }
                }
            }
            "import" => {
                if let Some(n) = next_sig(toks, i) {
                    if toks[n].kind == TokKind::Op && toks[n].text == "*" {
                        push(
                            "`from ... import *` — only names re-bound in this file are renamed; references to star-imported names stay verbatim".to_string(),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    #[test]
    fn detects_eval_globals_and_star_import() {
        let src = "from os import *\nx = 1\neval(\"x\")\nglobals()[\"x\"]\n";
        let risks = detect(&tokenize(src));
        assert!(risks.iter().any(|r| r.msg.contains("import *")));
        assert!(risks.iter().any(|r| r.msg.contains("eval(")));
        assert!(risks.iter().any(|r| r.msg.contains("globals(")));
    }

    #[test]
    fn detects_partial_kwargs_and_type_hints() {
        let src = "import functools\np = functools.partial(f, x=1)\ntyping.get_type_hints(C)\n";
        let risks = detect(&tokenize(src));
        assert!(risks.iter().any(|r| r.msg.contains("partial(")));
        assert!(risks.iter().any(|r| r.msg.contains("get_type_hints")));
    }

    #[test]
    fn calm_code_has_no_risks() {
        let src = "import json\nclass A:\n    def m(self, x=1):\n        return {\"k\": x}\nprint(A().m())\n";
        assert!(detect(&tokenize(src)).is_empty());
    }

    #[test]
    fn eval_as_attribute_is_not_flagged() {
        // `obj.eval(` is an attribute, not the builtin.
        let src = "obj.eval(\"x\")\nexecutor.run()\n";
        let risks = detect(&tokenize(src));
        assert!(!risks.iter().any(|r| r.msg.contains("eval(")));
    }
}
