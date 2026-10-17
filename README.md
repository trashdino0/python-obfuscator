# python-obfuscator

Fast, runnable-preserving Python source obfuscator written in Rust.
Single binary, no Python dependency at obfuscation time
(obfuscated output needs only the Python standard library).

## Quick start

```sh
cargo build --release
./target/release/python-obfuscator input.py -o output.py
python output.py   # behaves exactly like input.py
```

Obfuscate a whole tree (mirrors `*.py` into the output dir):

```sh
./target/release/python-obfuscator src/ -o out/ --seed 7 --verbose
```

## What it does (in order)

1. Strips comments and blank-line noise; standalone string statements
   (docstrings) become `pass` (same no-op semantics, keeps empty bodies valid).
2. Renames user identifiers to `_0x......` (seeded, reproducible):
   - top-level functions/classes, closures, locals, loop vars, `as` targets,
     lambda params, kwargs — renamed **consistently everywhere, including
     inside f-string `{...}` fields** (nested fields, conversions
     `!r`, and format specs like `{x:{w}}` handled recursively).
   - Never renamed: keywords, builtins (even if shadowed), imports,
     attributes (`obj.attr`), dunders, `self`/`cls`, methods and class
     attributes (they're reached as attributes), `__all__` exports,
     anything in `--keep`.
   - Keyword args are handled per-callee: calls to your functions rename
     kwargs consistently with renamed params (incl. `Cls(...)` via
     `__init__` params and `cls(...)`); kwargs to builtins
     (`sorted(key=)`), foreign methods (`"...".format(n=)`), and keys
     forwarded into `**kwargs` are left verbatim.
3. Encrypts plain/raw string literals with XOR + base64 and injects a
   `base64`-stdlib decoder stub. Skipped: f-strings, bytes, strings with
   backslash escapes (conservative), and `case` patterns (calls are illegal
   there — a call pattern means "class pattern," not "value").
4. Encodes decimal ints as `(0xA^0xB)` pairs (skips hex/float/complex and
   `case` patterns, where arbitrary expressions are a syntax error).
5. Aliases `True`/`False`/`None` to generated header names (same singleton
   objects, so `is` checks keep working; `case True:` left literal, since a
   name there becomes a capture pattern).
6. Injects never-taken `if <false>: ...` blocks at safe top-level spots
   (never between a header/decorator and its body) plus inert arithmetic
   assignments at end of file.
7. Keeps shebangs and `from __future__` imports first, as Python requires.

## Options

```
--seed N            RNG seed (default 42; same seed => byte-identical output)
--keep a,b          never rename these names
--min-string-len N  shortest string to encrypt (default 1)
--no-rename --no-strings --no-numbers --no-bool --no-deadcode
-v/--verbose
```

## Known limitations (documented, not bugs)

Dynamic name use defeats any static renamer — keep such names via `--keep`:

- `getattr(obj, "name")` / `setattr` / `globals()["name"]` / `eval` with
  renamed identifiers,
- string type annotations (`x: "MyClass"`) evaluated via
  `typing.get_type_hints`,
- `__all__`-style export lists built dynamically,
- `functools.partial(fn, kw=...)` with kwargs (captured before `fn`'s
  renamed params apply) and higher-order callbacks invoked with kwargs
  (`def run(cb): cb(flag=True)` where `cb` may be a builtin at runtime).

Control-flow flattening and `marshal`/`zlib` packing are intentionally out
of scope for v1 (high breakage risk).

## Verification

- `cargo test` — 15 unit tests (lexer, renamer, kwargs, transforms).
- `samples/` — `sample1.py` (closures, classes, kwargs, decorators-shape
  constructs, exceptions, lambdas) and `sample2.py` (nested f-strings,
  `__future__`, decorators, walrus, `match`, raw/bytes/unicode strings,
  `global`/`nonlocal`) both run **byte-identically** before/after
  obfuscation, across seeds, and even double-obfuscated.
- `test/` → `output/` — `art_shapes.py`, `ascii_banner.py`, `wave_art.py`,
  and the torture test `complex_engine.py` (inheritance + dunders,
  decorator factories, generators, closures, `match` with guards,
  custom exceptions, `**kwargs` forwarding, classmethod constructors)
  all run **byte-identically** after obfuscation.
