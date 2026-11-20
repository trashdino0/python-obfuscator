mod flatten;
mod junk;
mod lexer;
mod pack;
mod pipeline;
mod rename;
mod risks;
mod transform;
mod verify;

use clap::{Args, Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pipeline::{collect_py_files, obfuscate_source, parse_keep, ObfsOptions};

#[derive(Parser, Debug)]
#[command(
    name = "python-obfuscator",
    version,
    about = "Advanced Python source obfuscator (runnable-preserving)",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    #[command(flatten)]
    args: ObfArgs,
}

#[derive(Args, Debug)]
struct ObfArgs {
    /// Input .py file or directory (recursive)
    input: Option<PathBuf>,
    /// Output .py file or directory
    #[arg(short, long)]
    output: Option<PathBuf>,
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
    /// Disable control-flow flattening
    #[arg(long, default_value_t = false)]
    no_flatten: bool,
    /// Minimum string length to encrypt
    #[arg(long, default_value_t = 1)]
    min_string_len: usize,
    /// Refuse to write output when dynamic-code patterns that break under renaming are found
    #[arg(long, default_value_t = false)]
    strict: bool,
    /// Pack: compile to bytecode and embed it encrypted in a loader stub
    /// (output then needs the same Python minor version to run)
    #[arg(long, default_value_t = false)]
    pack: bool,
    /// Python interpreter used by --pack and `verify`
    #[arg(long, default_value = "python")]
    python: String,
    /// Print per-file stats
    #[arg(short, long, default_value_t = false)]
    verbose: bool,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Obfuscate in memory and check the obfuscated code behaves identically
    Verify {
        /// Input .py file or directory (recursive)
        input: PathBuf,
        #[arg(long, default_value_t = 42)]
        seed: u64,
        #[arg(long, default_value = "")]
        keep: String,
        #[arg(long, default_value_t = 1)]
        min_string_len: usize,
        #[arg(long, default_value_t = false)]
        no_rename: bool,
        #[arg(long, default_value_t = false)]
        no_strings: bool,
        #[arg(long, default_value_t = false)]
        no_numbers: bool,
        #[arg(long, default_value_t = false)]
        no_bool: bool,
        #[arg(long, default_value_t = false)]
        no_deadcode: bool,
        #[arg(long, default_value_t = false)]
        no_flatten: bool,
        /// Verify the packed loader instead of plain transformed source
        #[arg(long, default_value_t = false)]
        pack: bool,
        /// Skip files whose relative path contains this substring,
        /// case-insensitive (repeatable)
        #[arg(long, value_name = "SUBSTRING")]
        skip: Vec<String>,
        /// Python interpreter to use for both runs
        #[arg(long, default_value = "python")]
        python: String,
        /// Per-run timeout in seconds
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        #[arg(short, long, default_value_t = false)]
        verbose: bool,
    },
}

#[allow(clippy::too_many_arguments)]
fn opts_from(
    seed: u64,
    keep: &str,
    no_rename: bool,
    no_strings: bool,
    no_numbers: bool,
    no_bool: bool,
    no_deadcode: bool,
    no_flatten: bool,
    min_string_len: usize,
) -> ObfsOptions {
    ObfsOptions {
        seed,
        keep: parse_keep(keep),
        do_rename: !no_rename,
        do_strings: !no_strings,
        do_numbers: !no_numbers,
        do_bool_none: !no_bool,
        do_deadcode: !no_deadcode,
        do_flatten: !no_flatten,
        min_string_len,
    }
}

fn check_risks(path: &Path, src: &str, strict: bool) -> Result<(), String> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let toks = lexer::tokenize(src);
    let risks = risks::detect(&toks);
    for r in &risks {
        eprintln!("warning: {}:{}: {}", path.display(), r.line, r.msg);
    }
    if strict && !risks.is_empty() {
        return Err(format!(
            "{}: {} risky pattern(s) found and --strict is set (no output written)",
            path.display(),
            risks.len()
        ));
    }
    Ok(())
}

fn write_output(dest: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
        }
    }
    fs::write(dest, content).map_err(|e| format!("write {}: {e}", dest.display()))
}

fn run_obfuscate(a: &ObfArgs) -> Result<(), String> {
    let input = a
        .input
        .as_ref()
        .ok_or("input required (or use the `verify` subcommand)")?;
    let output = a
        .output
        .as_ref()
        .ok_or("--output is required for obfuscation")?;
    let opts = opts_from(
        a.seed,
        &a.keep,
        a.no_rename,
        a.no_strings,
        a.no_numbers,
        a.no_bool,
        a.no_deadcode,
        a.no_flatten,
        a.min_string_len,
    );

    if input.is_file() {
        let src =
            fs::read_to_string(input).map_err(|e| format!("read {}: {e}", input.display()))?;
        check_risks(input, &src, a.strict)?;
        let (mut out, stats) = obfuscate_source(&src, &opts);
        if a.pack {
            out = pack::pack_source(&out, &opts, &a.python)?;
        }
        write_output(output, &out)?;
        if a.verbose {
            eprintln!(
                "{} -> {} | renamed={} strings={} numbers={} bool/none={} dead={} flat={}{}",
                input.display(),
                output.display(),
                stats.renamed,
                stats.strings_encrypted,
                stats.numbers_encoded,
                stats.bool_none_replaced,
                stats.dead_blocks,
                stats.flattened,
                if a.pack { " packed=yes" } else { "" }
            );
        } else {
            println!(
                "OK {} -> {} (renamed={}, strings={}, numbers={}, flat={}{})",
                input.display(),
                output.display(),
                stats.renamed,
                stats.strings_encrypted,
                stats.numbers_encoded,
                stats.flattened,
                if a.pack { ", packed" } else { "" }
            );
        }
        return Ok(());
    }

    if input.is_dir() {
        let mut files = Vec::new();
        collect_py_files(input, &mut files)
            .map_err(|e| format!("walk {}: {e}", input.display()))?;
        if files.is_empty() {
            return Err(format!("no .py files under {}", input.display()));
        }
        files.sort();
        let mut done = 0;
        for f in &files {
            let rel = f
                .strip_prefix(input)
                .map_err(|e| format!("rel path: {e}"))?;
            let dest = output.join(rel);
            let src = fs::read_to_string(f).map_err(|e| format!("read {}: {e}", f.display()))?;
            check_risks(f, &src, a.strict)?;
            let (mut out, stats) = obfuscate_source(&src, &opts);
            if a.pack {
                out = pack::pack_source(&out, &opts, &a.python)?;
            }
            write_output(&dest, &out)?;
            if a.verbose {
                eprintln!(
                    "{} -> {} | renamed={} strings={} flat={}{}",
                    f.display(),
                    dest.display(),
                    stats.renamed,
                    stats.strings_encrypted,
                    stats.flattened,
                    if a.pack { " packed=yes" } else { "" }
                );
            }
            done += 1;
        }
        println!(
            "OK {done} file(s): {} -> {}",
            input.display(),
            output.display()
        );
        return Ok(());
    }

    Err(format!("input not found: {}", input.display()))
}

fn run(cli: &Cli) -> Result<(), String> {
    match &cli.command {
        Some(Cmd::Verify {
            input,
            seed,
            keep,
            min_string_len,
            no_rename,
            no_strings,
            no_numbers,
            no_bool,
            no_deadcode,
            no_flatten,
            pack,
            skip,
            python,
            timeout,
            verbose,
        }) => {
            let opts = opts_from(
                *seed,
                keep,
                *no_rename,
                *no_strings,
                *no_numbers,
                *no_bool,
                *no_deadcode,
                *no_flatten,
                *min_string_len,
            );
            verify::run_verify(&verify::VerifyArgs {
                input: input.clone(),
                opts,
                pack: *pack,
                python: python.clone(),
                timeout: Duration::from_secs(*timeout),
                skip: skip.clone(),
                verbose: *verbose,
            })
        }
        None => run_obfuscate(&cli.args),
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
