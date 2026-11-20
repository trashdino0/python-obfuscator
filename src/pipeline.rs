use crate::flatten;
use crate::junk;
use crate::lexer::{tokenize, TokKind};
use crate::rename;
use crate::transform::{
    case_protected, clean_lines, docstrings_to_pass, encode_numbers, encrypt_strings,
    future_import_end, make_header, render, replace_bool_none, split_shebang, strip_comments,
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub use crate::transform::{ObfsOptions, Stats};

/// Parse a comma-separated `--keep` list.
pub fn parse_keep(s: &str) -> HashSet<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

/// Full obfuscation pipeline for one source file. Deterministic for a seed.
pub fn obfuscate_source(src: &str, opts: &ObfsOptions) -> (String, Stats) {
    // Strip a UTF-8 BOM if present (otherwise it would survive mid-file).
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let (shebang, body) = split_shebang(src);
    let mut toks = tokenize(&body);

    strip_comments(&mut toks);
    docstrings_to_pass(&mut toks);

    let mut stats = Stats {
        renamed: 0,
        strings_encrypted: 0,
        numbers_encoded: 0,
        bool_none_replaced: 0,
        dead_blocks: 0,
        flattened: 0,
    };

    if opts.do_rename {
        let (res, _frozen) = rename::build_map(&toks, opts.seed, &opts.keep);
        stats.renamed = res.renamed_count;
        let skip = rename::foreign_kwarg_indices(&toks, &res.map);
        rename::apply_rename(&mut toks, &res.map, &skip);
    }

    // Names already in use (post-rename) so generated header/junk names can't collide.
    let mut used: HashSet<String> = toks
        .iter()
        .filter(|t| t.kind == TokKind::Ident)
        .map(|t| t.text.clone())
        .collect();

    let (header, key) = make_header(opts.seed, &mut used);

    // `case` patterns only allow literals — keep those tokens intact.
    let prot = case_protected(&toks);

    if opts.do_strings {
        stats.strings_encrypted =
            encrypt_strings(&mut toks, &header.decoder, key, opts.min_string_len, &prot);
    }
    if opts.do_numbers {
        stats.numbers_encoded = encode_numbers(&mut toks, opts.seed, &prot);
    }
    if opts.do_bool_none {
        stats.bool_none_replaced = replace_bool_none(
            &mut toks,
            &header.true_alias,
            &header.false_alias,
            &header.none_alias,
            &prot,
        );
    }

    // Control-flow flattening runs last, right before rendering: the state
    // machine it emits introduces fresh identifiers that must not be touched
    // by the other transforms.
    if opts.do_flatten {
        let mut frng = ChaCha8Rng::seed_from_u64(opts.seed ^ 0xF1A7_7E71_2345_6789);
        let (ftoks, n) = flatten::flatten(toks, &mut frng, &mut used, &header.none_alias);
        toks = ftoks;
        stats.flattened = n;
    }

    let rendered = render(&toks);
    let cleaned = clean_lines(&rendered);

    let mut body_out = cleaned;
    if opts.do_deadcode {
        let (with_dead, n) = junk::insert_deadcode(&body_out, opts.seed, &mut used);
        body_out = with_dead;
        stats.dead_blocks = n;
    }

    // Assemble: shebang, banner, [future imports...], header, rest.
    let banner = format!("# Obfuscated with python-obfuscator (seed {})", opts.seed);
    let lines: Vec<&str> = body_out.lines().collect();
    let mut out = String::new();
    if let Some(sb) = shebang {
        out.push_str(&sb);
        out.push('\n');
    }
    out.push_str(&banner);
    out.push('\n');
    match future_import_end(&lines) {
        Some(end) => {
            for l in &lines[..=end] {
                out.push_str(l);
                out.push('\n');
            }
            out.push_str(&header.text);
            for l in &lines[end + 1..] {
                out.push_str(l);
                out.push('\n');
            }
        }
        None => {
            out.push_str(&header.text);
            out.push_str(&body_out);
        }
    }
    (out, stats)
}

/// Recursively collect `.py` files under `dir` (sorted by the caller if needed).
pub fn collect_py_files(dir: &Path, acc: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            collect_py_files(&p, acc)?;
        } else if p.extension().map(|e| e == "py").unwrap_or(false) {
            acc.push(p);
        }
    }
    Ok(())
}
