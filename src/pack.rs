//! `--pack`: compile the obfuscated source to a code object with CPython,
//! marshal + XOR + zlib + base64 it, and emit a small loader stub that
//! decrypts and `exec`s it. The loader source is itself run through the
//! obfuscation pipeline, so even the stub is renamed/junked.
//!
//! The marshalled code object is bound to the Python minor version that
//! packed it — the loader checks `sys.version_info[:2]` and refuses to run
//! on a different interpreter.

use crate::pipeline::{obfuscate_source, ObfsOptions};
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const PACKER: &str = r#"
import sys, marshal, zlib, base64
src_path, key = sys.argv[1], int(sys.argv[2])
with open(src_path, encoding="utf-8") as fh:
    code = compile(fh.read(), "<frozen>", "exec")
raw = marshal.dumps(code)
xored = bytes(b ^ ((key >> ((i % 8) * 8)) & 255) for i, b in enumerate(raw))
blob = base64.b64encode(zlib.compress(xored, 9)).decode()
maj, minor = sys.version_info[:2]
loader = (
    "import sys as _s, marshal as _m, zlib as _z, base64 as _b\n"
    "if _s.version_info[:2] != (@MAJ@, @MIN@):\n"
    "    raise RuntimeError(\"this file needs Python @MAJ@.@MIN@, got %s.%s\" % _s.version_info[:2])\n"
    "_d = _b.b64decode(\"@BLOB@\")\n"
    "_k = @KEY@\n"
    "_x = bytes(v ^ ((_k >> ((i % 8) * 8)) & 255) for i, v in enumerate(_z.decompress(_d)))\n"
    "_c = _m.loads(_x)\n"
    "del _s, _m, _z, _b, _d, _k, _x\n"
    "exec(_c, globals())\n"
    "del _c\n"
)
sys.stdout.write(
    loader.replace("@MAJ@", str(maj))
    .replace("@MIN@", str(minor))
    .replace("@KEY@", str(key))
    .replace("@BLOB@", blob)
)
"#;

static SEQ: AtomicU64 = AtomicU64::new(0);
/// Pack obfuscated source into a loader stub. `python` must be the
/// interpreter that will run the output.
pub fn pack_source(obf_src: &str, opts: &ObfsOptions, python: &str) -> Result<String, String> {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = std::env::temp_dir();
    let in_path = tmp.join(format!("pyobf-pack-{}-{n}.py", std::process::id()));
    let packer_path = tmp.join(format!("pyobf-pack-{}-{n}-packer.py", std::process::id()));
    fs::write(&in_path, obf_src).map_err(|e| format!("pack: write {}: {e}", in_path.display()))?;
    fs::write(&packer_path, PACKER)
        .map_err(|e| format!("pack: write {}: {e}", packer_path.display()))?;

    let mut rng = ChaCha8Rng::seed_from_u64(opts.seed ^ 0x7E57_0BF5_C47E_0A11);
    let key: u64 = rng.random_range(0..=u64::MAX);

    let out = Command::new(python)
        .arg(&packer_path)
        .arg(&in_path)
        .arg(key.to_string())
        .output()
        .map_err(|e| format!("pack: failed to run `{python}`: {e}"))?;
    let _ = fs::remove_file(&in_path);
    let _ = fs::remove_file(&packer_path);
    if !out.status.success() {
        return Err(format!(
            "pack: compile failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let loader =
        String::from_utf8(out.stdout).map_err(|e| format!("pack: loader not utf-8: {e}"))?;

    // Obfuscate the loader itself (different seed so its names differ).
    let mut lopts = opts.clone();
    lopts.seed = opts.seed ^ 0xD15E_A5E5_25E7_4F5A;
    let (final_loader, _) = obfuscate_source(&loader, &lopts);
    Ok(final_loader)
}
