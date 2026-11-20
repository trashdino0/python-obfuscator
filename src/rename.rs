use crate::lexer::{parse_string, split_string, tokenize, TokKind, Token};
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::{HashMap, HashSet};

/// Python hard keywords + soft contextual keywords we conservatively freeze.
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "match", "case", "type",
];

/// Common builtins — never renamed, even if shadowed (safety over coverage).
const BUILTINS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "copyright",
    "credits",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "exit",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "license",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "quit",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "vars",
    "zip",
    "__import__",
    "__build_class__",
    "__debug__",
    "Ellipsis",
    "NotImplemented",
];

fn is_dunder(name: &str) -> bool {
    name.len() > 4 && name.starts_with("__") && name.ends_with("__")
}

/// Non-identifier token kinds we skip when looking for neighbours.
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

/// Collect names bound by `import ...` / `from ... import ...` — frozen.
fn collect_imported(toks: &[Token]) -> HashSet<String> {
    let mut frozen = HashSet::new();
    let mut i = 0;
    while i < toks.len() {
        if toks[i].kind == TokKind::Ident && (toks[i].text == "import" || toks[i].text == "from") {
            let is_from = toks[i].text == "from";
            i += 1;
            // Scan until end of logical line (newline at depth 0 or `;`).
            let mut depth = 0i32;
            let mut expect_bound_name = !is_from; // `import a` binds `a`
            let mut after_from_import = false;
            while i < toks.len() {
                let t = &toks[i];
                if t.kind == TokKind::Newline && depth == 0 {
                    break;
                }
                if t.kind == TokKind::Op {
                    match t.text.as_str() {
                        "(" | "[" | "{" => depth += 1,
                        ")" | "]" | "}" => depth -= 1,
                        ";" if depth == 0 => break,
                        "." => {
                            expect_bound_name = false;
                            i += 1;
                            continue;
                        }
                        "," => {
                            expect_bound_name = !is_from || after_from_import;
                            i += 1;
                            continue;
                        }
                        _ => {}
                    }
                    // `as` handling below needs ident check; fall through.
                }
                if t.kind == TokKind::Ident {
                    if t.text == "import" && is_from {
                        after_from_import = true;
                        expect_bound_name = true;
                        i += 1;
                        continue;
                    }
                    if t.text == "as" {
                        // Next ident is the bound name.
                        if let Some(ni) = next_sig(toks, i) {
                            if toks[ni].kind == TokKind::Ident {
                                frozen.insert(toks[ni].text.clone());
                            }
                        }
                        expect_bound_name = false;
                        i += 1;
                        continue;
                    }
                    if expect_bound_name && t.text != "*" && !KEYWORDS.contains(&t.text.as_str()) {
                        // For `import a.b`, only the top-level `a` is bound.
                        // We froze on `import`, skip idents after dots.
                        let dotted = prev_sig(toks, i).map(|p| is_dot(p, toks)).unwrap_or(false);
                        if !dotted {
                            // Peek: if followed by `.`, this is `a` in `a.b`
                            // (binds `a`); still freeze it.
                            frozen.insert(t.text.clone());
                            expect_bound_name = false;
                        }
                    }
                }
                // `*` in from-imports.
                if t.kind == TokKind::Op && t.text == "*" {
                    expect_bound_name = false;
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    frozen
}

/// Collect `__all__ = [...]` string entries so star-exports keep working
/// (those strings must keep matching the real public names).
fn collect_all_exports(toks: &[Token]) -> HashSet<String> {
    let mut exports = HashSet::new();
    let mut i = 0;
    while i < toks.len() {
        if toks[i].kind == TokKind::Ident && toks[i].text == "__all__" {
            if let Some(ni) = next_sig(toks, i) {
                if toks[ni].kind == TokKind::Op && toks[ni].text == "=" {
                    // Walk to end of logical line, collecting simple strings.
                    let mut depth = 0i32;
                    let mut j = ni + 1;
                    while j < toks.len() {
                        let t = &toks[j];
                        if t.kind == TokKind::Newline && depth == 0 {
                            break;
                        }
                        if t.kind == TokKind::Op {
                            match t.text.as_str() {
                                "(" | "[" | "{" => depth += 1,
                                ")" | "]" | "}" => depth -= 1,
                                _ => {}
                            }
                        }
                        if t.kind == TokKind::Str {
                            if let Some((prefix, _q, body)) = split_string(&t.text) {
                                let lp = prefix.to_lowercase();
                                if !lp.contains('f')
                                    && !lp.contains('b')
                                    && !body.contains('\\')
                                    && !body.is_empty()
                                    && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                                {
                                    exports.insert(body);
                                }
                            }
                        }
                        j += 1;
                    }
                }
            }
        }
        i += 1;
    }
    exports
}

const ASSIGN_OPS: &[&str] = &[
    "=", ":=", "+=", "-=", "*=", "/=", "//=", "%=", "**=", "&=", "|=", "^=", ">>=", "<<=", "@=",
];

/// Collect candidate user-defined names.
fn collect_defined(toks: &[Token]) -> HashSet<String> {
    let mut defined = HashSet::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        if t.kind != TokKind::Ident {
            i += 1;
            continue;
        }
        // Skip attribute positions entirely.
        if prev_sig(toks, i).map(|p| is_dot(p, toks)).unwrap_or(false) {
            i += 1;
            continue;
        }
        let name = t.text.clone();
        // `def NAME` / `class NAME`
        if name == "def" || name == "class" {
            if let Some(ni) = next_sig(toks, i) {
                if toks[ni].kind == TokKind::Ident {
                    defined.insert(toks[ni].text.clone());
                }
            }
            i += 1;
            continue;
        }
        // `for TARGET`, `with ... as TARGET`, `except ... as NAME`,
        // `global`/`nonlocal` declare (map handles via global rename),
        // function/lambda params handled by the generic assignment scan
        // plus an explicit param-region scan below.
        if ["for", "as", "global", "nonlocal"].contains(&name.as_str()) {
            if let Some(ni) = next_sig(toks, i) {
                if toks[ni].kind == TokKind::Ident && !KEYWORDS.contains(&toks[ni].text.as_str()) {
                    // `for a, b in` — also grab tuple partners.
                    let mut j = ni;
                    loop {
                        if toks[j].kind == TokKind::Ident
                            && prev_sig(toks, j).map(|p| !is_dot(p, toks)).unwrap_or(true)
                        {
                            defined.insert(toks[j].text.clone());
                        }
                        // walk `, ident` chains
                        let mut k = j + 1;
                        while k < toks.len()
                            && matches!(toks[k].kind, TokKind::Whitespace | TokKind::Comment)
                        {
                            k += 1;
                        }
                        if k < toks.len() && toks[k].kind == TokKind::Op && toks[k].text == "," {
                            let mut m = k + 1;
                            while m < toks.len()
                                && matches!(toks[m].kind, TokKind::Whitespace | TokKind::Comment)
                            {
                                m += 1;
                            }
                            if m < toks.len() && toks[m].kind == TokKind::Ident {
                                j = m;
                                continue;
                            }
                        }
                        break;
                    }
                }
            }
            i += 1;
            continue;
        }
        // Assignment-ish: `NAME <assignop>` or annotated `NAME :`.
        if let Some(ni) = next_sig(toks, i) {
            let nt = &toks[ni];
            if nt.kind == TokKind::Op && ASSIGN_OPS.contains(&nt.text.as_str()) {
                defined.insert(name);
                i += 1;
                continue;
            }
            // Walrus inside expressions, annotation `x: int`.
            if nt.kind == TokKind::Op && nt.text == ":" {
                // `:` also appears in slices/dicts/lambdas/defs — only treat
                // as annotation when plausible: previous context is a lone
                // name in a def/class body or `x: T = ...`. We accept
                // over-collection (safe under global consistent mapping).
                defined.insert(name);
                i += 1;
                continue;
            }
        }
        i += 1;
    }
    // Function/lambda parameter regions: `def f(...):` params and
    // `lambda ... :` params.
    collect_params(toks, &mut defined);
    defined
}

fn collect_params(toks: &[Token], defined: &mut HashSet<String>) {
    let mut i = 0;
    while i < toks.len() {
        if toks[i].kind == TokKind::Ident && (toks[i].text == "def" || toks[i].text == "lambda") {
            let is_def = toks[i].text == "def";
            let mut j = i + 1;
            // Skip whitespace/comments; for `def`, skip the function name.
            while j < toks.len()
                && matches!(
                    toks[j].kind,
                    TokKind::Whitespace | TokKind::Comment | TokKind::Newline
                )
            {
                j += 1;
            }
            if is_def {
                if j < toks.len() && toks[j].kind == TokKind::Ident {
                    j += 1; // function name itself
                }
            }
            while j < toks.len()
                && matches!(
                    toks[j].kind,
                    TokKind::Whitespace | TokKind::Comment | TokKind::Newline
                )
            {
                j += 1;
            }
            // Expect `(` for defs; lambda has no parens.
            if is_def {
                if j >= toks.len() || toks[j].text != "(" {
                    i += 1;
                    continue;
                }
                j += 1;
                let mut depth = 1i32;
                while j < toks.len() && depth > 0 {
                    let t = &toks[j];
                    if t.kind == TokKind::Op {
                        match t.text.as_str() {
                            "(" | "[" | "{" => depth += 1,
                            ")" => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    if depth >= 1
                        && t.kind == TokKind::Ident
                        && !KEYWORDS.contains(&t.text.as_str())
                        && prev_sig(toks, j).map(|p| !is_dot(p, toks)).unwrap_or(true)
                    {
                        // `*`, `/` markers are Ops; any ident here is a param
                        // name or part of a default expr. Over-collection is
                        // safe (global consistent mapping, builtins frozen).
                        defined.insert(t.text.clone());
                    }
                    j += 1;
                }
            } else {
                // lambda params run until the matching `:`.
                let mut depth = 0i32;
                while j < toks.len() {
                    let t = &toks[j];
                    if t.kind == TokKind::Op {
                        match t.text.as_str() {
                            "(" | "[" | "{" => depth += 1,
                            ")" | "]" | "}" => depth -= 1,
                            ":" if depth == 0 => break,
                            _ => {}
                        }
                    }
                    if depth == 0
                        && t.kind == TokKind::Ident
                        && !KEYWORDS.contains(&t.text.as_str())
                        && prev_sig(toks, j).map(|p| !is_dot(p, toks)).unwrap_or(true)
                    {
                        defined.insert(t.text.clone());
                    }
                    j += 1;
                }
            }
        }
        i += 1;
    }
}

/// Physical-line view used for indentation-based block tracking.
struct LineInfo {
    indent: usize,
    /// Significant token indices (no whitespace/comments/newlines).
    sig: Vec<usize>,
}

fn split_lines(toks: &[Token]) -> Vec<LineInfo> {
    let mut lines: Vec<LineInfo> = vec![LineInfo {
        indent: 0,
        sig: vec![],
    }];
    let mut at_start = true;
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            TokKind::Newline => {
                lines.push(LineInfo {
                    indent: 0,
                    sig: vec![],
                });
                at_start = true;
            }
            TokKind::Whitespace if at_start => {
                // Only leading whitespace counts as indent. A Whitespace
                // token elsewhere on the line is ignored here.
                lines.last_mut().unwrap().indent = t.text.chars().count();
            }
            TokKind::Comment => {}
            TokKind::Whitespace => {}
            _ => {
                at_start = false;
                lines.last_mut().unwrap().sig.push(i);
            }
        }
    }
    lines
}

/// Names bound directly in a `class` body (methods, class attributes).
/// These are always accessed as attributes (`obj.name`, `Cls.name`) which
/// we never rename — so the definitions must stay frozen too.
fn class_scoped_names(toks: &[Token]) -> HashSet<String> {
    let lines = split_lines(toks);
    let mut out: HashSet<String> = HashSet::new();
    // Stack of (is_class, body_indent) for enclosing def/class blocks.
    let mut stack: Vec<(bool, usize)> = vec![];

    for (li, ln) in lines.iter().enumerate() {
        if ln.sig.is_empty() {
            continue;
        }
        while let Some(&(_, b)) = stack.last() {
            if ln.indent < b {
                stack.pop();
            } else {
                break;
            }
        }
        let first = toks[ln.sig[0]].text.clone();
        // Decorator lines: evaluated in enclosing scope, handled by the
        // normal consistent-rename path — don't freeze here.
        if first == "@" {
            continue;
        }
        let is_async_def = first == "async" && ln.sig.len() > 1 && toks[ln.sig[1]].text == "def";
        let is_def = first == "def" || is_async_def;
        let is_class = first == "class";

        if is_def || is_class {
            let name_pos = if is_class {
                1
            } else if is_async_def {
                2
            } else {
                1
            };
            let in_class = matches!(stack.last(), Some((true, _)));
            if is_def && in_class {
                if let Some(&ni) = ln.sig.get(name_pos) {
                    if toks[ni].kind == TokKind::Ident {
                        out.insert(toks[ni].text.clone());
                    }
                }
            }
            if is_class {
                // One-liner bodies: `class A: x = 1` — freeze names after
                // the header colon on the same line.
                let mut depth = 0i32;
                let mut past_colon = false;
                for &ti in &ln.sig {
                    let t = &toks[ti];
                    if !past_colon {
                        if t.kind == TokKind::Op {
                            match t.text.as_str() {
                                "[" | "(" | "{" => depth += 1,
                                "]" | ")" | "}" => depth -= 1,
                                ":" if depth == 0 => past_colon = true,
                                _ => {}
                            }
                        }
                        continue;
                    }
                    if t.kind == TokKind::Ident
                        && prev_sig(toks, ti).map(|p| !is_dot(p, toks)).unwrap_or(true)
                        && !KEYWORDS.contains(&t.text.as_str())
                    {
                        out.insert(t.text.clone());
                    }
                }
            }
            // Push this block with its body indent (lookahead).
            let mut body_indent = usize::MAX; // one-liner default
            for future in lines.iter().skip(li + 1) {
                if future.sig.is_empty() {
                    continue;
                }
                if future.indent > ln.indent {
                    body_indent = future.indent;
                }
                break;
            }
            stack.push((is_class, body_indent));
            continue;
        }

        // Ordinary line directly in a class body: freeze bound names.
        if matches!(stack.last(), Some((true, _))) {
            for &ti in &ln.sig {
                let t = &toks[ti];
                if t.kind == TokKind::Ident
                    && prev_sig(toks, ti).map(|p| !is_dot(p, toks)).unwrap_or(true)
                    && !KEYWORDS.contains(&t.text.as_str())
                {
                    out.insert(t.text.clone());
                }
            }
        }
    }
    out
}

fn gen_name(rng: &mut ChaCha8Rng, used: &HashSet<String>) -> String {
    loop {
        let n: u32 = rng.random_range(0..0xFFFFFF);
        let cand = format!("_0x{n:06x}");
        if !used.contains(&cand) {
            return cand;
        }
    }
}

/// Token spans of module-level `def`/`async def` blocks: from the header
/// token to the last token of the body (nested defs included).
fn top_level_scopes(toks: &[Token]) -> Vec<(usize, usize)> {
    let lines = split_lines(toks);
    let mut scopes = Vec::new();
    let mut li = 0;
    while li < lines.len() {
        let ln = &lines[li];
        if ln.sig.is_empty() || ln.indent != 0 {
            li += 1;
            continue;
        }
        let first = toks[ln.sig[0]].text.as_str();
        let is_async_def = first == "async" && ln.sig.len() > 1 && toks[ln.sig[1]].text == "def";
        if first != "def" && !is_async_def {
            li += 1;
            continue;
        }
        let start = ln.sig[0];
        // Body: following lines while blank or indented.
        let mut lj = li;
        let mut k = li + 1;
        while k < lines.len() {
            let lk = &lines[k];
            if lk.sig.is_empty() {
                k += 1;
                continue;
            }
            if lk.indent == 0 {
                break;
            }
            lj = k;
            k += 1;
        }
        let end = lines[lj].sig.last().copied().unwrap_or(start);
        scopes.push((start, end));
        li = k;
    }
    scopes
}

/// Non-attribute mentions of `name`: identifier tokens (skipping `obj.attr`
/// positions) plus f-string tokens whose text mentions it.
fn name_occurrences(toks: &[Token], name: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            TokKind::Ident => {
                if t.text == name && !prev_sig(toks, i).map(|p| is_dot(p, toks)).unwrap_or(false) {
                    out.push(i);
                }
            }
            TokKind::Str => {
                if let Some((prefix, _, _)) = split_string(&t.text) {
                    if prefix.to_lowercase().contains('f') && word_contains(&t.text, name) {
                        out.push(i);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Is `name` mentioned in any `global`/`nonlocal` statement?
fn declared_global_or_nonlocal(toks: &[Token], name: &str) -> bool {
    let mut i = 0;
    while i < toks.len() {
        if toks[i].kind == TokKind::Ident
            && (toks[i].text == "global" || toks[i].text == "nonlocal")
        {
            let mut depth = 0i32;
            let mut j = i + 1;
            while j < toks.len() {
                let t = &toks[j];
                if t.kind == TokKind::Newline && depth == 0 {
                    break;
                }
                if t.kind == TokKind::Op {
                    match t.text.as_str() {
                        "(" | "[" | "{" => depth += 1,
                        ")" | "]" | "}" => depth -= 1,
                        ";" if depth == 0 => break,
                        _ => {}
                    }
                }
                if t.kind == TokKind::Ident && t.text == name {
                    return true;
                }
                j += 1;
            }
        }
        i += 1;
    }
    false
}

/// Does `hay` contain `needle` delimited by non-identifier characters?
fn word_contains(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    if nb.len() > hb.len() {
        return false;
    }
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 128;
    for i in 0..=(hb.len() - nb.len()) {
        if &hb[i..i + nb.len()] == nb {
            let before_ok = i == 0 || !is_ident(hb[i - 1]);
            let after_ok = i + nb.len() == hb.len() || !is_ident(hb[i + nb.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

pub struct RenameResult {
    pub map: HashMap<String, String>,
    pub renamed_count: usize,
}

/// Build global rename map: original -> obfuscated.
pub fn build_map(
    toks: &[Token],
    seed: u64,
    keep: &HashSet<String>,
) -> (RenameResult, HashSet<String>) {
    let imported = collect_imported(toks);
    let defined = collect_defined(toks);
    // `__all__` entries must keep their public names (star-import contract).
    let mut keep = keep.clone();
    keep.extend(collect_all_exports(toks));

    let mut frozen: HashSet<String> = HashSet::new();
    frozen.extend(KEYWORDS.iter().map(|s| s.to_string()));
    frozen.extend(BUILTINS.iter().map(|s| s.to_string()));
    frozen.extend(imported.iter().cloned());
    frozen.extend(keep.iter().cloned());
    // Methods and class attributes are accessed as attributes (`obj.name`),
    // which are never renamed — so their definitions stay frozen too.
    frozen.extend(class_scoped_names(toks).iter().cloned());
    // `self`/`cls` conventional first args stay readable and safe.
    frozen.insert("self".to_string());
    frozen.insert("cls".to_string());
    // Throwaway wildcard stays a wildcard (`case _0x...` would bind).
    frozen.insert("_".to_string());

    let mut names: Vec<String> = defined
        .into_iter()
        .filter(|n| !frozen.contains(n) && !is_dunder(n))
        .filter(|n| !KEYWORDS.contains(&n.as_str()))
        .collect();
    names.sort();
    names.dedup();

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    // Every identifier text in the file is off-limits for generated names:
    // a freshly minted name must never be something the source might refer to.
    let mut hard: HashSet<String> = toks
        .iter()
        .filter(|t| t.kind == TokKind::Ident)
        .map(|t| t.text.clone())
        .collect();
    hard.extend(frozen.iter().cloned());

    // Scope-aware assignment. Names whose every non-attribute mention falls
    // inside one module-level function body are local to that body, so
    // different bodies may share the same short generated name. Everything
    // else (module-level names, names spread across scopes, `global`/
    // `nonlocal` names) stays globally unique.
    let scopes = top_level_scopes(toks);
    let mut global_names: Vec<String> = Vec::new();
    let mut local_names: Vec<Vec<String>> = vec![Vec::new(); scopes.len()];
    for n in &names {
        let mut scope_of: Option<usize> = None;
        let mut ok = !declared_global_or_nonlocal(toks, n);
        if ok {
            for i in name_occurrences(toks, n) {
                match scopes.iter().position(|&(s, e)| i >= s && i <= e) {
                    Some(si) => {
                        if scope_of.is_some_and(|p| p != si) {
                            ok = false;
                            break;
                        }
                        scope_of = Some(si);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
        }
        match (ok, scope_of) {
            (true, Some(si)) => local_names[si].push(n.clone()),
            _ => global_names.push(n.clone()),
        }
    }

    let mut map = HashMap::new();
    for n in &global_names {
        let ob = gen_name(&mut rng, &hard);
        hard.insert(ob.clone());
        map.insert(n.clone(), ob);
    }
    for locals in &local_names {
        // Short per-scope names, reused across scopes. They always stay well
        // below the six-digit shape of global-pool names, so the two pools
        // can never collide with each other or with source identifiers.
        let mut counter: u32 = 1;
        for n in locals {
            let ob = if counter >= 0x100000 {
                let g = gen_name(&mut rng, &hard);
                hard.insert(g.clone());
                g
            } else {
                loop {
                    let cand = format!("_0x{counter:x}");
                    counter += 1;
                    if !hard.contains(&cand) {
                        break cand;
                    }
                }
            };
            map.insert(n.clone(), ob);
        }
    }

    (
        RenameResult {
            map,
            renamed_count: names.len(),
        },
        frozen,
    )
}

/// Scan a `(`-opened parameter region, collecting depth-1 identifiers
/// (excluding keywords and attribute positions). Stops at the matching `)`.
fn scan_param_idents(toks: &[Token], open: usize) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut depth = 0i32;
    let mut j = open;
    while j < toks.len() {
        let t = &toks[j];
        if t.kind == TokKind::Op {
            match t.text.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                "]" | "}" => depth -= 1,
                _ => {}
            }
        }
        if depth == 1
            && t.kind == TokKind::Ident
            && !KEYWORDS.contains(&t.text.as_str())
            && prev_sig(toks, j).map(|p| !is_dot(p, toks)).unwrap_or(true)
        {
            out.insert(t.text.clone());
        }
        j += 1;
    }
    out
}

/// Remove the `**kwargs` catcher from a param set: the `**` op token
/// followed by its name at depth 1.
fn remove_kwargs_catcher(toks: &[Token], open: usize, params: &mut HashSet<String>) {
    let mut depth = 0i32;
    let mut j = open;
    while j < toks.len() {
        let t = &toks[j];
        if t.kind == TokKind::Op {
            match t.text.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                "]" | "}" => depth -= 1,
                "**" if depth == 1 => {
                    if let Some(ni) = next_sig(toks, j) {
                        if toks[ni].kind == TokKind::Ident {
                            params.remove(&toks[ni].text);
                        }
                    }
                }
                _ => {}
            }
        }
        j += 1;
    }
}

/// Function/method name -> explicit parameter names (excluding the `**kwargs`
/// catcher, whose forwarded keys are runtime data and must stay verbatim).
/// Includes `NAME = lambda ...` simple assignments. Class names map to their
/// `__init__` params (union across classes); `cls` maps to the same union.
fn explicit_params(toks: &[Token]) -> HashMap<String, HashSet<String>> {
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    let mut i = 0;
    while i < toks.len() {
        let is_def = toks[i].kind == TokKind::Ident && toks[i].text == "def";
        let is_async_def = toks[i].kind == TokKind::Ident
            && toks[i].text == "async"
            && next_sig(toks, i)
                .is_some_and(|ni| toks[ni].kind == TokKind::Ident && toks[ni].text == "def");
        if is_def || is_async_def {
            let name_idx = if is_async_def {
                next_sig(toks, i)
                    .and_then(|d| next_sig(toks, d))
                    .unwrap_or(toks.len())
            } else {
                next_sig(toks, i).unwrap_or(toks.len())
            };
            if name_idx < toks.len() && toks[name_idx].kind == TokKind::Ident {
                let fname = toks[name_idx].text.clone();
                if let Some(p) = next_sig(toks, name_idx) {
                    if toks[p].kind == TokKind::Op && toks[p].text == "(" {
                        let mut params = scan_param_idents(toks, p);
                        remove_kwargs_catcher(toks, p, &mut params);
                        out.entry(fname).or_default().extend(params);
                    }
                }
            }
            i += 1;
            continue;
        }
        // `NAME = lambda <params> : ...` — record under NAME.
        if toks[i].kind == TokKind::Ident && toks[i].text == "lambda" {
            if let Some(eq) = prev_sig(toks, i) {
                if toks[eq].kind == TokKind::Op && toks[eq].text == "=" {
                    if let Some(nm) = prev_sig(toks, eq) {
                        let boundary = prev_sig(toks, nm).is_none_or(|b| {
                            matches!(toks[b].kind, TokKind::Newline)
                                || (toks[b].kind == TokKind::Op && toks[b].text == ";")
                        });
                        if toks[nm].kind == TokKind::Ident && boundary {
                            let fname = toks[nm].text.clone();
                            // Params run to the first depth-0 `:`.
                            let mut params = HashSet::new();
                            let mut catcher: Option<String> = None;
                            let mut depth = 0i32;
                            let mut k = i + 1;
                            while k < toks.len() {
                                let t = &toks[k];
                                if t.kind == TokKind::Op {
                                    match t.text.as_str() {
                                        "(" | "[" | "{" => depth += 1,
                                        ")" | "]" | "}" => depth -= 1,
                                        ":" if depth == 0 => break,
                                        "**" if depth == 0 => {
                                            if let Some(n2) = next_sig(toks, k) {
                                                if toks[n2].kind == TokKind::Ident {
                                                    catcher = Some(toks[n2].text.clone());
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                if depth == 0
                                    && t.kind == TokKind::Ident
                                    && t.text != "lambda"
                                    && !KEYWORDS.contains(&t.text.as_str())
                                {
                                    params.insert(t.text.clone());
                                }
                                k += 1;
                            }
                            if let Some(c) = catcher {
                                params.remove(&c);
                            }
                            out.entry(fname).or_default().extend(params);
                        }
                    }
                }
            }
        }
        i += 1;
    }
    // Classes instantiate via `__init__`: map each class name (and `cls`)
    // to the union of all `__init__` explicit params.
    let mut classes: HashSet<String> = HashSet::new();
    let mut k = 0;
    while k < toks.len() {
        if toks[k].kind == TokKind::Ident && toks[k].text == "class" {
            if let Some(ni) = next_sig(toks, k) {
                if toks[ni].kind == TokKind::Ident {
                    classes.insert(toks[ni].text.clone());
                }
            }
        }
        k += 1;
    }
    if let Some(init_params) = out.get("__init__").cloned() {
        for c in &classes {
            out.entry(c.clone())
                .or_default()
                .extend(init_params.iter().cloned());
        }
        out.entry("cls".to_string())
            .or_default()
            .extend(init_params);
    }
    out
}

/// Simple `Alias = Original` bindings (bare idents only), for resolving
/// function aliases at call sites (`paint = paint_dragon; paint(size=8)`).
fn simple_aliases(toks: &[Token]) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    let mut i = 0;
    while i < toks.len() {
        if toks[i].kind == TokKind::Ident
            && !KEYWORDS.contains(&toks[i].text.as_str())
            && prev_sig(toks, i).map(|p| !is_dot(p, toks)).unwrap_or(true)
        {
            if let Some(eq) = next_sig(toks, i) {
                if toks[eq].kind == TokKind::Op && toks[eq].text == "=" {
                    if let Some(v) = next_sig(toks, eq) {
                        if toks[v].kind == TokKind::Ident
                            && !KEYWORDS.contains(&toks[v].text.as_str())
                        {
                            // Must be a bare name, not a call or attribute.
                            let after = next_sig(toks, v);
                            let ok = after.is_none_or(|a| {
                                !(toks[a].kind == TokKind::Op
                                    && (toks[a].text == "(" || toks[a].text == "."))
                            });
                            if ok {
                                aliases.insert(toks[i].text.clone(), toks[v].text.clone());
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    aliases
}

fn resolve_alias<'a>(name: &'a str, aliases: &'a HashMap<String, String>) -> &'a str {
    let mut cur = name;
    for _ in 0..8 {
        match aliases.get(cur) {
            Some(next) => cur = next,
            None => break,
        }
    }
    cur
}

/// Token indices of keyword-argument names that must NOT be renamed.
/// Per-kwarg rule at each call:
/// - callee user-backed (in rename map, alias-resolved) or a known user
///   method or `cls`, AND the kwarg is an explicit param of that callee
///   -> rename (consistent with the renamed def).
/// - otherwise (builtins like `sorted(key=)`, foreign methods like
///   `"...".format(n=)`, `**kwargs`-forwarded keys) -> protect.
pub fn foreign_kwarg_indices(toks: &[Token], map: &HashMap<String, String>) -> HashSet<usize> {
    let methods = class_scoped_names(toks);
    let explicit = explicit_params(toks);
    let aliases = simple_aliases(toks);
    let empty: HashSet<String> = HashSet::new();
    let mut prot = HashSet::new();
    let mut i = 0;
    while i < toks.len() {
        if toks[i].kind == TokKind::Ident {
            if let Some(pi) = next_sig(toks, i) {
                if toks[pi].kind == TokKind::Op && toks[pi].text == "(" {
                    // `def name(` / `class name(` are definitions, not calls.
                    let after_def = prev_sig(toks, i).is_some_and(|p| {
                        toks[p].kind == TokKind::Ident
                            && (toks[p].text == "def" || toks[p].text == "class")
                    });
                    if after_def {
                        i += 1;
                        continue;
                    }
                    let is_attr = prev_sig(toks, i).map(|p| is_dot(p, toks)).unwrap_or(false);
                    let eff: &str = if is_attr {
                        &toks[i].text
                    } else {
                        resolve_alias(&toks[i].text, &aliases)
                    };
                    let backed = if is_attr {
                        methods.contains(eff)
                    } else {
                        // `cls(...)` is a call to the class itself: it is
                        // frozen (never renamed) but still user-backed, so
                        // its kwargs must follow `__init__`'s renamed params.
                        map.contains_key(eff) || eff == "cls"
                    };
                    let params = explicit.get(eff).unwrap_or(&empty);
                    // Scan to the matching close paren, collecting depth-1
                    // `name=` kwargs (skipping lambda-param regions).
                    let mut depth = 0i32;
                    let mut j = pi;
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
                                        if !(backed && params.contains(&t.text)) {
                                            prot.insert(j);
                                        }
                                    }
                                }
                            }
                        }
                        j += 1;
                    }
                    i += 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    prot
}

/// Apply rename map to identifier tokens (except attribute positions and
/// protected foreign-kwarg names), including identifiers inside f-string
/// replacement fields (recursively).
pub fn apply_rename(toks: &mut [Token], map: &HashMap<String, String>, skip: &HashSet<usize>) {
    for i in 0..toks.len() {
        if toks[i].kind == TokKind::Str {
            if let Some((prefix, _, _)) = split_string(&toks[i].text) {
                if prefix.to_lowercase().contains('f') {
                    toks[i].text = rename_fstring(&toks[i].text, map);
                }
            }
            continue;
        }
        if toks[i].kind != TokKind::Ident || skip.contains(&i) {
            continue;
        }
        if prev_sig(toks, i).map(|p| is_dot(p, toks)).unwrap_or(false) {
            continue; // `obj.attr` — attribute stays.
        }
        if let Some(rep) = map.get(&toks[i].text) {
            toks[i].text = rep.clone();
        }
    }
}

/// Rename plain identifiers in a token slice (no f-string recursion;
/// used for code already extracted from an f-string field).
fn apply_rename_shallow(toks: &mut [Token], map: &HashMap<String, String>, skip: &HashSet<usize>) {
    for i in 0..toks.len() {
        if toks[i].kind != TokKind::Ident || skip.contains(&i) {
            continue;
        }
        if prev_sig(toks, i).map(|p| is_dot(p, toks)).unwrap_or(false) {
            continue;
        }
        if let Some(rep) = map.get(&toks[i].text) {
            toks[i].text = rep.clone();
        }
    }
}

/// Rename identifiers inside an f-string token's replacement fields.
/// Literal text, conversions and nested-field structure are preserved
/// byte-for-byte; only code identifiers are rewritten, so output stays
/// runnable even if field splitting is imperfect (worst case: a name is
/// left unrenamed, never broken).
fn rename_fstring(text: &str, map: &HashMap<String, String>) -> String {
    let Some((prefix, quote, body)) = split_string(text) else {
        return text.to_string();
    };
    if !prefix.to_lowercase().contains('f') {
        return text.to_string();
    }
    format!("{prefix}{quote}{}{quote}", rename_fields(&body, map))
}

/// Rewrite `{...}` fields of an f-string body (or format spec).
fn rename_fields(s: &str, map: &HashMap<String, String>) -> String {
    let b = s.as_bytes();
    let n = b.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < n {
        if b[i] >= 128 {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let c = b[i] as char;
        if c == '{' {
            if i + 1 < n && b[i + 1] as char == '{' {
                out.push_str("{{");
                i += 2;
                continue;
            }
            match find_field_end(s, i) {
                Some(end) => {
                    out.push('{');
                    out.push_str(&rename_field_inner(&s[i + 1..end], map));
                    out.push('}');
                    i = end + 1;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            }
            continue;
        }
        if c == '}' {
            if i + 1 < n && b[i + 1] as char == '}' {
                out.push_str("}}");
                i += 2;
            } else {
                out.push(c);
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Find the byte index of the `}` closing the field opened at `open`.
/// Skips over nested strings and tracks nested `{...}` (dict/set literals).
fn find_field_end(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let n = b.len();
    let mut depth = 1i32;
    let mut i = open + 1;
    while i < n {
        let byte = b[i];
        if byte >= 128 {
            let ch = s[i..].chars().next().unwrap();
            i += ch.len_utf8();
            continue;
        }
        // Skip nested string literals (they may contain braces).
        if byte as char == '\'' || byte as char == '"' || "rRbBuUfF".contains(byte as char) {
            if let Some((_, end)) = parse_string(s, i) {
                i = end;
                continue;
            }
        }
        match byte as char {
            '\\' => {
                i += 2; // tolerated (3.12+ allows escapes in fields)
            }
            '{' => {
                depth += 1;
                i += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// Split field `inner` into (expr, rest) at the first top-level `!` or `:`
/// (conversion / format-spec separator), tracking brackets and strings.
fn split_field(inner: &str) -> Option<(&str, &str)> {
    let b = inner.as_bytes();
    let n = b.len();
    let mut depth = 0i32;
    let mut i = 0;
    while i < n {
        let byte = b[i];
        if byte >= 128 {
            let ch = inner[i..].chars().next().unwrap();
            i += ch.len_utf8();
            continue;
        }
        if byte as char == '\'' || byte as char == '"' || "rRbBuUfF".contains(byte as char) {
            if let Some((_, end)) = parse_string(inner, i) {
                i = end;
                continue;
            }
        }
        match byte as char {
            '(' | '[' | '{' => {
                depth += 1;
                i += 1;
            }
            ')' | ']' | '}' => {
                depth -= 1;
                i += 1;
            }
            '!' | ':' if depth == 0 => return Some((&inner[..i], &inner[i..])),
            _ => i += 1,
        }
    }
    None
}

fn rename_field_inner(inner: &str, map: &HashMap<String, String>) -> String {
    match split_field(inner) {
        Some((expr, rest)) => format!("{}{}", rename_expr(expr, map), rename_spec(rest, map)),
        None => rename_expr(inner, map),
    }
}

/// Rename the conversion/spec tail of a field (`!r`, `:spec`, `!r:spec`).
/// Nested fields inside specs are processed recursively.
fn rename_spec(rest: &str, map: &HashMap<String, String>) -> String {
    if let Some(after_bang) = rest.strip_prefix('!') {
        match after_bang.chars().next() {
            Some(conv) => {
                let after_conv = &after_bang[conv.len_utf8()..];
                if let Some(spec) = after_conv.strip_prefix(':') {
                    format!("!{conv}:{}", rename_fields(spec, map))
                } else {
                    rest.to_string()
                }
            }
            None => rest.to_string(),
        }
    } else if let Some(spec) = rest.strip_prefix(':') {
        format!(":{}", rename_fields(spec, map))
    } else {
        rest.to_string()
    }
}

/// Rename identifiers in a field expression (tokenized, so strings and
/// attributes are respected; nested f-strings handled recursively).
fn rename_expr(expr: &str, map: &HashMap<String, String>) -> String {
    let mut toks = tokenize(expr);
    let skip = foreign_kwarg_indices(&toks, map);
    apply_rename_shallow(&mut toks, map, &skip);
    let mut out = String::with_capacity(expr.len());
    for t in &toks {
        if t.kind == TokKind::Str {
            if let Some((prefix, _, _)) = split_string(&t.text) {
                if prefix.to_lowercase().contains('f') {
                    out.push_str(&rename_fstring(&t.text, map));
                    continue;
                }
            }
        }
        out.push_str(&t.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    #[test]
    fn kwargs_forwarding_protected() {
        let src = "def show(name=\"d\", **kw):\n    return name\nmode = \"fast\"\nprint(show(name=\"x\", mode=mode))\n";
        let mut toks = tokenize(src);
        let keep = HashSet::new();
        let (res, _) = build_map(&toks, 1, &keep);
        let omode = res.map.get("mode").unwrap().clone();
        let skip = foreign_kwarg_indices(&toks, &res.map);
        apply_rename(&mut toks, &res.map, &skip);
        let r: String = toks.iter().map(|t| t.text.clone()).collect();
        // Kwarg *name* stays (forwarded into **kw); its *value* still renames.
        assert!(r.contains(&format!("mode={omode}")));
        assert!(!r.contains("name=\"x\"")); // explicit param: renamed
    }

    #[test]
    fn class_init_kwargs_follow_params() {
        let src = "class C:\n    def __init__(self, width=1):\n        self.width = width\nc = C(width=5)\n";
        let mut toks = tokenize(src);
        let keep = HashSet::new();
        let (res, _) = build_map(&toks, 1, &keep);
        let ow = res.map.get("width").unwrap().clone();
        let oc = res.map.get("C").unwrap().clone();
        let skip = foreign_kwarg_indices(&toks, &res.map);
        apply_rename(&mut toks, &res.map, &skip);
        let r: String = toks.iter().map(|t| t.text.clone()).collect();
        assert!(r.contains(&format!("def __init__(self, {ow}=1)")));
        assert!(r.contains(&format!("{oc}({ow}=5)")));
    }

    #[test]
    fn foreign_kwargs_protected() {
        let src = "def foo(bar, key=1):\n    return bar\nprint(foo(1, key=2), sep=\",\")\nitems = sorted([3, 1], key=abs)\n";
        let mut toks = tokenize(src);
        let keep = HashSet::new();
        let (res, _) = build_map(&toks, 1, &keep);
        assert!(res.map.contains_key("key")); // user param still renamed...
        let skip = foreign_kwarg_indices(&toks, &res.map);
        apply_rename(&mut toks, &res.map, &skip);
        let r: String = toks.iter().map(|t| t.text.clone()).collect();
        assert!(r.contains("key=abs")); // ...but not in sorted()
        assert!(r.contains("sep=")); // nor in print()
        assert!(r.contains("def _0x")); // def site renamed
        assert!(r.contains("(1, _0x")); // user call site follows def
    }

    #[test]
    fn method_kwargs_follow_renamed_params() {
        let src = "class A:\n    def m(self, x=1):\n        return x\na = A()\na.m(x=2)\n";
        let mut toks = tokenize(src);
        let keep = HashSet::new();
        let (res, _) = build_map(&toks, 1, &keep);
        let ox = res.map.get("x").unwrap().clone();
        let skip = foreign_kwarg_indices(&toks, &res.map);
        apply_rename(&mut toks, &res.map, &skip);
        let r: String = toks.iter().map(|t| t.text.clone()).collect();
        assert!(r.contains(&format!("def m(self, {ox}=1)")));
        assert!(r.contains(&format!(".m({ox}=2)")));
    }

    #[test]
    fn fstring_fields_renamed_consistently() {
        let src = "def foo(bar):\n    x = bar + 1\n    return f\"val {x} {bar!r:{x}}\"\n";
        let mut toks = tokenize(src);
        let keep = HashSet::new();
        let (res, _) = build_map(&toks, 1, &keep);
        let ox = res.map.get("x").unwrap().clone();
        let obar = res.map.get("bar").unwrap().clone();
        let skip = foreign_kwarg_indices(&toks, &res.map);
        apply_rename(&mut toks, &res.map, &skip);
        let rendered: String = toks.iter().map(|t| t.text.clone()).collect();
        // Original bare names must be gone from code positions...
        assert!(!rendered.contains("bar +") && !rendered.contains("(bar)"));
        // ...but present only inside renamed f-string as new names.
        assert!(rendered.contains(&format!("{{{ox}}}")));
        assert!(rendered.contains(&format!("{{{obar}!r")));
        // Literal text untouched, escapes intact.
        let src2 = "w = 5\ns = f\"{{literal}} {w:>3}\"\n";
        let mut t2 = tokenize(src2);
        let (res2, _) = build_map(&t2, 1, &keep);
        let skip2 = foreign_kwarg_indices(&t2, &res2.map);
        apply_rename(&mut t2, &res2.map, &skip2);
        let r2: String = t2.iter().map(|t| t.text.clone()).collect();
        assert!(r2.contains("{{literal}}"));
        assert!(r2.contains(":>3}"));
    }

    #[test]
    fn class_members_frozen() {
        let src = "class Calc:\n    factor = 2\n    def add(self, a):\n        return a + self.factor\nobj = Calc()\nprint(obj.add(1))\n";
        let mut toks = tokenize(src);
        let keep = HashSet::new();
        let (res, _) = build_map(&toks, 1, &keep);
        assert!(!res.map.contains_key("add"));
        assert!(!res.map.contains_key("factor"));
        assert!(res.map.contains_key("Calc"));
        assert!(res.map.contains_key("obj"));
        let skip = foreign_kwarg_indices(&toks, &res.map);
        apply_rename(&mut toks, &res.map, &skip);
        let r: String = toks.iter().map(|t| t.text.clone()).collect();
        assert!(r.contains("def add("));
        assert!(r.contains(".add("));
    }

    #[test]
    fn attributes_and_imports_frozen() {
        let src = "import os\nfrom sys import path\ndef foo(bar):\n    x = bar + 1\n    obj.attr = x\n    return os.getcwd()\n";
        let toks = tokenize(src);
        let keep = HashSet::new();
        let (res, _) = build_map(&toks, 1, &keep);
        assert!(res.map.contains_key("foo"));
        assert!(res.map.contains_key("bar"));
        assert!(res.map.contains_key("x"));
        assert!(!res.map.contains_key("os"));
        assert!(!res.map.contains_key("path"));
        assert!(!res.map.contains_key("attr"));
        assert!(!res.map.contains_key("getcwd"));
    }

    #[test]
    fn disjoint_scopes_reuse_short_names() {
        let src = "def a():\n    alpha = 1\n    return alpha\n\ndef b():\n    beta = 2\n    return beta\n";
        let (res, _) = build_map(&tokenize(src), 1, &HashSet::new());
        let al = res.map.get("alpha").unwrap().clone();
        let be = res.map.get("beta").unwrap().clone();
        assert_eq!(al, be, "sibling scopes may share a short name");
        assert_eq!(res.map.get("a").unwrap(), res.map.get("b").unwrap());
        assert!(
            al.starts_with("_0x") && al.len() <= 5,
            "short form expected, got {al}"
        );
        assert!(al != "_0x0", "must not look like a source identifier");
    }

    #[test]
    fn module_shared_name_stays_globally_unique() {
        let src = "def a():\n    total = 1\n    return total\n\ntotal = a()\nprint(total)\n";
        let (res, _) = build_map(&tokenize(src), 1, &HashSet::new());
        let t = res.map.get("total").unwrap();
        assert_eq!(t.len(), 9, "module-shared name must use the global pool");
    }

    #[test]
    fn closure_reference_is_renamed_consistently() {
        let src = "def outer():\n    seed = 5\n    def inner():\n        return seed + 1\n    return inner()\n";
        let mut toks = tokenize(src);
        let (res, _) = build_map(&toks, 1, &HashSet::new());
        let s = res.map.get("seed").unwrap().clone();
        assert!(
            s.starts_with("_0x") && s.len() <= 5,
            "closure-captured local stays in its scope pool, got {s}"
        );
        let skip = foreign_kwarg_indices(&toks, &res.map);
        apply_rename(&mut toks, &res.map, &skip);
        let r: String = toks.iter().map(|t| t.text.clone()).collect();
        assert!(r.contains(&format!("{s} = 5")));
        assert!(r.contains(&format!("return {s} + 1")));
    }

    #[test]
    fn global_declaration_prevents_pooling() {
        let src = "def bump():\n    global tot\n    tot = 1\n    return tot\n";
        let (res, _) = build_map(&tokenize(src), 1, &HashSet::new());
        let t = res.map.get("tot").unwrap();
        assert_eq!(t.len(), 9, "`global` names must stay globally unique");
    }

    #[test]
    fn attribute_mention_does_not_disqualify_pooling() {
        let src = "def f(o):\n    alpha = 1\n    o.alpha = alpha\n    return alpha\n";
        let mut toks = tokenize(src);
        let (res, _) = build_map(&toks, 1, &HashSet::new());
        let al = res.map.get("alpha").unwrap().clone();
        assert_eq!(al, "_0x1");
        let skip = foreign_kwarg_indices(&toks, &res.map);
        apply_rename(&mut toks, &res.map, &skip);
        let r: String = toks.iter().map(|t| t.text.clone()).collect();
        assert!(r.contains(".alpha ="), "attribute positions are untouched");
        assert!(r.contains(&format!("= {al}")) && r.contains(&format!("return {al}")));
    }

    #[test]
    fn module_level_fstring_mention_prevents_pooling() {
        let src = "def f():\n    alpha = 1\n    return alpha\n\nprint(f\"{alpha}\")\n";
        let (res, _) = build_map(&tokenize(src), 1, &HashSet::new());
        let al = res.map.get("alpha").unwrap();
        assert_eq!(
            al.len(),
            9,
            "module-level f-string mention means the name is not scope-confined"
        );
        // ...but mentions inside the owning scope are fine.
        let src2 = "def g():\n    alpha = 1\n    return f\"{alpha}\"\n";
        let (res2, _) = build_map(&tokenize(src2), 1, &HashSet::new());
        assert_eq!(res2.map.get("alpha").unwrap(), "_0x1");
    }
}
