//! `verify` subcommand: obfuscate (optionally pack), run original and
//! obfuscated with the same interpreter, and compare stdout / exit code /
//! (path- and line-number-normalised) stderr.

use crate::pack;
use crate::pipeline::{collect_py_files, obfuscate_source, ObfsOptions};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct VerifyArgs {
    pub input: PathBuf,
    pub opts: ObfsOptions,
    pub pack: bool,
    pub python: String,
    pub timeout: Duration,
    pub skip: Vec<String>,
    pub verbose: bool,
}

struct RunOut {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: Option<i32>,
    timed_out: bool,
}

fn run_python(
    python: &str,
    script: &Path,
    cwd: &Path,
    timeout: Duration,
) -> Result<RunOut, String> {
    let mut child = Command::new(python)
        .arg(script)
        .current_dir(cwd)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONHASHSEED", "0")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run `{python}`: {e}"))?;

    let mut out_pipe = child.stdout.take().ok_or("no stdout pipe")?;
    let mut err_pipe = child.stderr.take().ok_or("no stderr pipe")?;
    let t_out = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out_pipe.read_to_end(&mut v);
        v
    });
    let t_err = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err_pipe.read_to_end(&mut v);
        v
    });

    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    timed_out = true;
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    };

    Ok(RunOut {
        stdout: t_out.join().unwrap_or_default(),
        stderr: t_err.join().unwrap_or_default(),
        code: status.and_then(|s| s.code()),
        timed_out,
    })
}

/// Normalise a stream for comparison: CRLF -> LF, script path -> `<script>`,
/// `, line N` -> `, line #`. Tracebacks then compare structurally.
fn norm_stream(bytes: &[u8], script: &Path) -> String {
    let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    let text = text.replace(&script.display().to_string(), "<script>");
    let mut out = String::with_capacity(text.len());
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if text[i..].starts_with(", line ") {
            out.push_str(", line #");
            i += ", line ".len();
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            let ch = text[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out.trim_end().to_string()
}

/// Case-insensitive substring match of a file's relative path against the
/// `--skip` patterns.
fn matches_skip(rel: &str, patterns: &[String]) -> bool {
    let rel = rel.to_lowercase();
    patterns.iter().any(|p| rel.contains(&p.to_lowercase()))
}

pub fn run_verify(a: &VerifyArgs) -> Result<(), String> {
    // Absolute paths: the child processes run with a different cwd.
    let input_root = fs::canonicalize(&a.input)
        .map_err(|e| format!("input not found: {} ({e})", a.input.display()))?;
    let mut files: Vec<PathBuf> = if input_root.is_file() {
        vec![input_root.clone()]
    } else if input_root.is_dir() {
        let mut v = Vec::new();
        collect_py_files(&input_root, &mut v)
            .map_err(|e| format!("walk {}: {e}", input_root.display()))?;
        v.sort();
        if v.is_empty() {
            return Err(format!("no .py files under {}", input_root.display()));
        }
        v
    } else {
        return Err(format!("input not found: {}", a.input.display()));
    };

    // `--skip`: drop files whose relative path matches any pattern.
    let rel_path = |f: &Path| -> String {
        if input_root.is_file() {
            f.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            f.strip_prefix(&input_root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        }
    };

    let mut skipped = 0usize;
    if !a.skip.is_empty() {
        files.retain(|f| {
            if matches_skip(&rel_path(f), &a.skip) {
                skipped += 1;
                println!("skip  {}", f.display());
                false
            } else {
                true
            }
        });
        if files.is_empty() {
            return Err(format!("all {skipped} file(s) excluded by --skip"));
        }
    }

    let root = std::env::temp_dir().join(format!("pyobf-verify-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).map_err(|e| format!("create {}: {e}", root.display()))?;

    let mut pass = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for f in &files {
        let rel = if input_root.is_file() {
            PathBuf::from(f.file_name().ok_or("bad file name")?)
        } else {
            f.strip_prefix(&input_root)
                .map_err(|e| format!("rel path: {e}"))?
                .to_path_buf()
        };
        let dest = root.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        let base_cwd = f
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let base = run_python(&a.python, f, &base_cwd, a.timeout)?;
        if base.timed_out {
            failures.push(format!(
                "{}: original timed out after {:?}",
                f.display(),
                a.timeout
            ));
            continue;
        }

        let src = fs::read_to_string(f).map_err(|e| format!("read {}: {e}", f.display()))?;
        let (obf, _stats) = obfuscate_source(&src, &a.opts);
        let final_src = if a.pack {
            pack::pack_source(&obf, &a.opts, &a.python)?
        } else {
            obf
        };
        fs::write(&dest, final_src).map_err(|e| format!("write {}: {e}", dest.display()))?;

        let obf_cwd = dest
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let run = run_python(&a.python, &dest, &obf_cwd, a.timeout)?;
        if run.timed_out {
            failures.push(format!(
                "{}: obfuscated timed out after {:?}",
                f.display(),
                a.timeout
            ));
            continue;
        }

        let expected_out = norm_stream(&base.stdout, f);
        let actual_out = norm_stream(&run.stdout, &dest);
        let expected_err = norm_stream(&base.stderr, f);
        let actual_err = norm_stream(&run.stderr, &dest);

        if expected_out == actual_out && base.code == run.code && expected_err == actual_err {
            pass += 1;
            if a.verbose {
                println!("ok  {} (exit {})", f.display(), base.code.unwrap_or(-1));
            }
        } else {
            let mut detail = format!("{}", f.display());
            if base.code != run.code {
                detail.push_str(&format!("\n  exit code: {:?} -> {:?}", base.code, run.code));
            }
            if expected_out != actual_out {
                detail.push_str("\n  stdout differs:");
                for (i, d) in diff_lines(&expected_out, &actual_out)
                    .iter()
                    .take(6)
                    .enumerate()
                {
                    detail.push_str(&format!("\n    [{i}] {d}"));
                }
            }
            if expected_err != actual_err {
                detail.push_str("\n  stderr differs:");
                for (i, d) in diff_lines(&expected_err, &actual_err)
                    .iter()
                    .take(6)
                    .enumerate()
                {
                    detail.push_str(&format!("\n    [{i}] {d}"));
                }
            }
            failures.push(detail);
        }
    }

    if failures.is_empty() {
        let _ = fs::remove_dir_all(&root);
        if skipped > 0 {
            println!(
                "verify: {} file(s) identical, {} skipped",
                files.len(),
                skipped
            );
        } else {
            println!("verify: {} file(s) identical", files.len());
        }
        Ok(())
    } else {
        for f in &failures {
            eprintln!("FAIL {f}");
        }
        if skipped > 0 {
            eprintln!(
                "verify: {}/{} passed, {} skipped; obfuscated copies kept under {}",
                pass,
                files.len(),
                skipped,
                root.display()
            );
        } else {
            eprintln!(
                "verify: {}/{} passed; obfuscated copies kept under {}",
                pass,
                files.len(),
                root.display()
            );
        }
        Err(format!("{} file(s) failed verification", failures.len()))
    }
}

/// First few differing line pairs, for human-readable failure output.
fn diff_lines(expected: &str, actual: &str) -> Vec<String> {
    let e: Vec<&str> = expected.lines().collect();
    let a: Vec<&str> = actual.lines().collect();
    let mut out = Vec::new();
    for i in 0..e.len().max(a.len()) {
        let el = e.get(i).copied().unwrap_or("<missing>");
        let al = a.get(i).copied().unwrap_or("<missing>");
        if el != al {
            out.push(format!("expected `{el}` got `{al}`"));
            if out.len() >= 12 {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::matches_skip;

    #[test]
    fn skip_match_is_case_insensitive_substring() {
        let pats = vec!["Graphing Calculator.PY".to_string()];
        assert!(matches_skip("graphing calculator.py", &pats));
        assert!(matches_skip("sub/dir/graphing calculator.py", &pats));
        assert!(!matches_skip("main.py", &pats));
    }

    #[test]
    fn skip_match_with_no_patterns() {
        assert!(!matches_skip("anything.py", &[]));
    }
}
