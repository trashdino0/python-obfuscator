mod lexer;
mod rename;
mod transform;

use clap::Parser;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use lexer::tokenize;
use transform::{
    case_protected, clean_lines, docstrings_to_pass, encode_numbers, encrypt_strings,
    future_import_end, insert_deadcode, make_header, render, replace_bool_none, split_shebang,
    strip_comments, ObfsOptions, Stats,
};

#[derive(Parser, Debug)]
#[command(
    name = "python-obfuscator",
    version,
    about = "Advance Python source obfuscator (runnable-preserving)"
)]
struct Cli {
    /// Input .py file or directory (recursive)
    input: PathBuf,
    /// Output .py file or directory
    #[arg(short, long)]
    output: PathBuf,
    /// RNG seed for reproducible output
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Comma-separated names to never rename
    #[arg(long, default_value = "")]
    keep: String,
    /// Disable identifier renaming
    #[arg(long, default_value_t = false)]
    no_rename: bool,
    /// Disable string encryption
    #[arg(long, default_value_t = false)]
    no_strings: bool,
    /// Disable integer encoding
    #[arg(long, default_value_t = false)]
    no_numbers: bool,
    /// Disable True/False/None aliasing
    #[arg(long, default_value_t = false)]
    no_bool: bool,
    /// Disable dead-code injection
    #[arg(long, default_value_t = false)]
    no_deadcode: bool,
    /// Minimum string length to encrypt
    #[arg(long, default_value_t = 1)]
    min_string_len: usize,
    /// Print per-file stats
    #[arg(short, long, default_value_t = false)]
    verbose: bool,
}

fn parse_keep(s: &str) -> HashSet<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn obfuscate_source(src: &str, opts: &ObfsOptions) -> (String, Stats) {
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
        .filter(|t| t.kind == lexer::TokKind::Ident)
        .map(|t| t.text.clone())
        .collect();

    let (header, key) = make_header(opts.seed, &mut used);

    // `case` patterns only allow literals — keep those tokens intact.
    let prot = case_protected(&toks);

    if opts.do_strings {
        let mut rng = ChaCha8Rng::seed_from_u64(opts.seed ^ 0x57A1_16C5_7EED_00D);
        stats.strings_encrypted = encrypt_strings(
            &mut toks,
            &header.decoder,
            key,
            opts.min_string_len,
            &mut rng,
            &prot,
        );
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

    let rendered = render(&toks);
    let cleaned = clean_lines(&rendered);

    let mut body_out = cleaned;
    if opts.do_deadcode {
        let (with_dead, n) = insert_deadcode(&body_out, opts.seed, &mut used);
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

fn collect_py_files(dir: &Path, acc: &mut Vec<PathBuf>) -> std::io::Result<()> {
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

fn run(cli: &Cli) -> Result<(), String> {
    let opts = ObfsOptions {
        seed: cli.seed,
        keep: parse_keep(&cli.keep),
        do_rename: !cli.no_rename,
        do_strings: !cli.no_strings,
        do_numbers: !cli.no_numbers,
        do_bool_none: !cli.no_bool,
        do_deadcode: !cli.no_deadcode,
        min_string_len: cli.min_string_len,
    };

    if cli.input.is_file() {
        let src = fs::read_to_string(&cli.input)
            .map_err(|e| format!("read {}: {e}", cli.input.display()))?;
        let (out, stats) = obfuscate_source(&src, &opts);
        if let Some(parent) = cli.output.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
            }
        }
        fs::write(&cli.output, out).map_err(|e| format!("write {}: {e}", cli.output.display()))?;
        if cli.verbose {
            eprintln!(
                "{} -> {} | renamed={} strings={} numbers={} bool/none={} dead={}",
                cli.input.display(),
                cli.output.display(),
                stats.renamed,
                stats.strings_encrypted,
                stats.numbers_encoded,
                stats.bool_none_replaced,
                stats.dead_blocks
            );
        } else {
            println!(
                "OK {} -> {} (renamed={}, strings={}, numbers={})",
                cli.input.display(),
                cli.output.display(),
                stats.renamed,
                stats.strings_encrypted,
                stats.numbers_encoded
            );
        }
        return Ok(());
    }

    if cli.input.is_dir() {
        let mut files = Vec::new();
        collect_py_files(&cli.input, &mut files)
            .map_err(|e| format!("walk {}: {e}", cli.input.display()))?;
        if files.is_empty() {
            return Err(format!("no .py files under {}", cli.input.display()));
        }
        files.sort();
        let mut done = 0;
        for f in &files {
            let rel = f
                .strip_prefix(&cli.input)
                .map_err(|e| format!("rel path: {e}"))?;
            let dest = cli.output.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
            }
            let src = fs::read_to_string(f).map_err(|e| format!("read {}: {e}", f.display()))?;
            let (out, stats) = obfuscate_source(&src, &opts);
            fs::write(&dest, out).map_err(|e| format!("write {}: {e}", dest.display()))?;
            if cli.verbose {
                eprintln!(
                    "{} -> {} | renamed={} strings={}",
                    f.display(),
                    dest.display(),
                    stats.renamed,
                    stats.strings_encrypted
                );
            }
            done += 1;
        }
        println!(
            "OK {done} file(s): {} -> {}",
            cli.input.display(),
            cli.output.display()
        );
        return Ok(());
    }

    Err(format!("input not found: {}", cli.input.display()))
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
