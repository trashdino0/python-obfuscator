# python-obfuscator

Fast, runnable-preserving Python source obfuscator written in Rust.
Single binary, no Python dependency at obfuscation time for source output
(obfuscated output needs only the Python standard library; `--pack` output
additionally needs the same Python minor version that produced it).

The hard invariant: **obfuscated output must behave exactly like the input**
(same stdout/stderr/exit code) — enforced by the built-in `verify` mode.

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

Prove behavior is identical (runs original vs obfuscated, compares stdout,
stderr, and exit code; also works with `--pack`):

```sh
./target/release/python-obfuscator verify test/ --verbose
./target/release/python-obfuscator verify test/ --pack
```

Pack to encrypted bytecode behind a loader stub (needs the same Python minor
version at runtime, e.g. 3.12 here):

```sh
./target/release/python-obfuscator input.py -o output.py --pack
```

## What it does (in order)

1. Strips comments and blank-line noise; standalone string statements
   (docstrings) become `pass` (same no-op semantics, keeps empty bodies
   valid — note `__doc__` becomes `None`).
2. Renames user identifiers to `_0x......` (seeded, reproducible):
   - top-level functions/classes, closures, locals, loop vars, `as` targets,
     lambda params, kwargs — renamed **consistently everywhere, including
     inside f-string `{...}` fields** (nested fields, conversions
     `!r`, and format specs like `{x:{w}}` handled recursively).
   - **Scope-aware pooling**: names that provably occur inside a single
     module-level function body get short names (`_0x1`, `_0x2`, …) that are
     reused across sibling functions — the same generated name means
     different variables in different scopes. Module-level names, names
     shared across scopes, f-string mentions outside the owning scope, and
     `global`/`nonlocal` names keep unique long names.
   - Never renamed: keywords, builtins (even if shadowed), imports,
     attributes (`obj.attr`), dunders, `self`/`cls`, methods and class
     attributes (they're reached as attributes), `__all__` exports,
     anything in `--keep`.
   - Keyword args are handled per-callee: calls to your functions rename
     kwargs consistently with renamed params (incl. `Cls(...)` via
     `__init__` params and `cls(...)`); kwargs to builtins
     (`sorted(key=)`), foreign methods (`"...".format(n=)`), and keys
     forwarded into `**kwargs` are left verbatim.
3. Encrypts string literals with a self-contained XOR keystream decoder
   (LCG + hex, one generated `_0x......` function, **no stdlib imports**).
   Long strings are chunked into concatenated decoder calls; adjacent
   literal chains (implicit concatenation) are left intact. Skipped:
   f-strings, bytes, strings with backslash escapes, and `case` patterns
   (calls are illegal there — a call pattern means "class pattern").
4. Encodes decimal ints as `(0xA^0xB)` pairs (skips hex/float/complex and
   `case` patterns, where arbitrary expressions are a syntax error).
5. Aliases `True`/`False`/`None` to generated header names (same singleton
   objects, so `is` checks keep working; `case True:` left literal, since a
   name there becomes a capture pattern).
6. **Control-flow flattening**: function/method bodies that are one flat run
   of simple statements become randomized state machines —
   `while _s>=0: if _s==<hex>: ... elif ...` with shuffled arm order and
   hex state ids; `return expr` becomes `_r = expr; _s = -1`. Skipped when
   the body contains nested suites, loops, `try`, `match`, `yield`/`await`,
   `;`, or backslash continuations; `global`/`nonlocal` declarations are
   always emitted before the uses Python demands.
7. **Dead-code injection**: never-taken `if <false>:` blocks at safe
   module-level spots plus always-true lambda-predicate junk blocks
   (pure arithmetic only) inside function bodies and flattened arms — never
   between a header/decorator and its body, never inside multi-line strings.
8. Keeps shebangs and `from __future__` imports first, as Python requires.

## Options

```
--seed N            RNG seed (default 42; same seed => byte-identical output)
--keep a,b          never rename these names
--min-string-len N  shortest string to encrypt (default 1)
--no-rename --no-strings --no-numbers --no-bool --no-deadcode --no-flatten
--strict            refuse output when dynamic-code risks are detected
                    (warnings always printed; --strict makes them fatal)
--pack              compile to bytecode, encrypt it, embed in a loader stub
                    (output then needs the same Python minor version)
--python PATH       interpreter used by --pack and verify (default: python)
-v/--verbose        per-file stats (renamed/strings/numbers/dead/flat)
```

### `verify` subcommand

```
python-obfuscator verify INPUT [--pack] [--seed N] [--keep ...] [--timeout S] [--skip SUBSTR]...
```

Runs the original and the obfuscated copy under the same interpreter with
fixed env (`PYTHONHASHSEED=0`, UTF-8 I/O, no `__pycache__`), then compares
stdout, stderr, and exit code modulo line numbers and file paths. Failed
runs leave the obfuscated copies in a temp dir for inspection.

`--skip SUBSTR` excludes files whose relative path contains the
case-insensitive substring (repeatable) — handy for GUI scripts that block
in `mainloop()`.

### Risk detector

`eval`/`exec`/`compile` calls, `globals()`/`locals()`, `get_type_hints`,
`functools.partial(fn, kw=...)`, and `from ... import *` are flagged —
they can defeat static renaming. Keep affected names via `--keep` or use
`--strict` to refuse writing output.

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

Other trade-offs:

- Docstrings become `pass`, so `__doc__` is `None` in obfuscated output.
- Flattening applies only to straight-line bodies; looping/branching
  functions keep their original shape.
- Traceback line numbers differ (the `verify` comparator normalizes them).
- `--pack` output is bound to the exact Python minor version used to pack.
- `verify` executes scripts to completion; scripts that block in
  `mainloop()` (GUI apps) time out by design — test those with a stub
  driver such as `tools/graph_drive.py`, or exclude them from folder runs
  with `verify DIR --skip SUBSTR`.

## Verification

- `cargo test` — 40 unit tests (lexer, renamer incl. scope-aware pooling,
  kwargs, string engine, flattening eligibility, junk insertion safety).
- `samples/` — `sample1.py` (closures, classes, kwargs, decorators-shape
  constructs, exceptions, lambdas), `sample2.py` (nested f-strings,
  `__future__`, decorators, walrus, `match`, raw/bytes/unicode strings,
  `global`/`nonlocal`), and `edge1-3.py` (implicit concatenation, long
  literals, `%` formatting precedence) all verify **byte-identically**
  before/after obfuscation, plain and `--pack`.
- `test/` → `output/` — `art_shapes.py`, `ascii_banner.py`, `wave_art.py`,
  the torture test `complex_engine.py` (inheritance + dunders, decorator
  factories, generators, closures, `match` with guards, custom exceptions,
  `**kwargs` forwarding, classmethod constructors), `flatten_cases.py`
  (flattening: `global`/`nonlocal`, `del`, multi-line calls, decorators,
  async bodies, unreachable returns, `ZeroDivisionError` parity), and
  `scope_cases.py` (scope-aware renaming: cross-scope name reuse, closures,
  f-string mentions, comprehensions, shadowing, `global` declarations) all
  verify byte-identically, plain and `--pack`.
- `test/graphing calculator.py` (tkinter + turtle GUI) — `verify` can't run
  `mainloop()` apps, so `tools/graph_drive.py` stub-drives the app instead:
  it fakes `tkinter`/`turtle`, simulates equation selection, Submit/Enter
  flows, and the Restart/Add/Exit buttons, records every GUI call with its
  arguments, and prints a digest. Original, obfuscated, packed, and
  alternate-seed builds all print the **same digest** (40,126 recorded GUI
  calls), proving behavior parity for the driven paths.
