//! The source scanner: the functional core's observation source for module
//! boundaries. Given a crate's `src/`, it lists `.rs` files, walks the `mod`-declared
//! module graph reachable from the crate root, and extracts the `crate::…` module
//! paths a file imports via `use` — comments, string literals, and macro bodies
//! stripped, so only real, file-based, reachable imports are observed (PROJECT.md).
//! Pure string and path processing: it depends on no model type, only `std`.

use std::path::{Path, PathBuf};

/// Internal module paths imported by `source`, normalized to absolute `crate::…`
/// form. `current_module` is the importing file's module; a `use` inside an inline
/// `mod name { … }` is attributed to that submodule, so `self`/`super` resolve against
/// the real enclosing module, not the file's. Only `use` declarations are observed;
/// grouped and glob forms are expanded; raw identifiers (`r#name`) are canonicalized;
/// paths whose first segment is an external crate are ignored. Bare path expressions
/// and macro-generated imports are out of scope (PROJECT.md): comments and string
/// literals are stripped, and macro bodies are removed, so a `use` written inside one
/// is a macro-generated import and is not observed. Returns sorted, de-duplicated paths.
pub(crate) fn imported_module_paths(
    source: &str,
    current_module: &str,
    root_modules: &[String],
) -> Vec<String> {
    let cleaned = strip_macro_bodies(&strip_comments_and_strings(source));
    let mut paths = Vec::new();
    for (module, tree) in use_trees_with_modules(&cleaned, current_module) {
        for leaf in expand_use_tree(&tree) {
            if let Some(absolute) = normalize_module_path(&leaf, &module, root_modules) {
                paths.push(absolute);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Remove macro bodies so a `use` written inside a macro — a macro-generated import,
/// out of scope per the module-boundary spec — is not mistaken for a real import. Two
/// forms are stripped: a `macro_rules! name <delim>…<delim>` **definition** (name and
/// balanced body), and a macro **invocation** `ident! <delim>…<delim>` (the balanced
/// body; the `ident!` head is kept, harmlessly). Runs on already comment/string-stripped
/// text, so every delimiter is structural and a `macro`/`!` inside a comment or string is
/// not matched. A real `use` is never inside a macro body, so nothing real is dropped.
/// The body delimiter may be `{}`, `()`, or `[]`. Never panics on malformed input.
fn strip_macro_bodies(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some(end) = macro_rules_body_end(bytes, i) {
            // `macro_rules! name <delim>…<delim>` — drop the name and the body.
            out.push(b' ');
            i = end;
        } else if bytes[i] == b'!' && i > 0 && is_ident_byte(bytes[i - 1]) {
            // A macro invocation `ident! <delim>…<delim>`: keep the `!`, drop the body.
            // The `!` of `macro_rules!` is never reached here — the definition arm above
            // consumes it. `!=` / unary `!expr` are not invocations: the byte after `!`
            // is not an opening delimiter, so `macro_invocation_body_end` returns `None`.
            match macro_invocation_body_end(bytes, i) {
                Some(end) => {
                    out.push(b'!');
                    out.push(b' ');
                    i = end;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// If a `macro_rules! name <delim>…<delim>` definition begins at `i`, return the index
/// just past its balanced closing delimiter; otherwise `None`. `macro_rules` must be a
/// standalone word, followed by `!`, a macro name, and an opening `{`/`(`/`[`.
fn macro_rules_body_end(bytes: &[u8], i: usize) -> Option<usize> {
    const KW: &[u8] = b"macro_rules";
    if !bytes[i..].starts_with(KW) || (i > 0 && is_ident_byte(bytes[i - 1])) {
        return None;
    }
    let skip_ws = |mut j: usize| {
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        j
    };
    let mut j = skip_ws(i + KW.len());
    if bytes.get(j) != Some(&b'!') {
        return None;
    }
    j = skip_ws(j + 1);
    // The macro name (one or more identifier bytes).
    let name_start = j;
    while j < bytes.len() && is_ident_byte(bytes[j]) {
        j += 1;
    }
    if j == name_start {
        return None;
    }
    balanced_group_end(bytes, skip_ws(j))
}

/// If `bytes[i]` is the `!` of a macro invocation `ident! <delim>…<delim>` (the caller
/// has checked an identifier byte immediately precedes), return the index past the
/// balanced body; otherwise `None`. The opening delimiter may follow whitespace.
fn macro_invocation_body_end(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    balanced_group_end(bytes, j)
}

/// Index just past the balanced delimiter group opening at `j` (which must be `{`, `(`,
/// or `[`), or `None` if `j` is not an opening delimiter. Strings and comments are
/// already stripped, so every delimiter is structural and same-delimiter groups nest
/// correctly. An unterminated group (malformed input) ends at end of input, not a panic.
fn balanced_group_end(bytes: &[u8], j: usize) -> Option<usize> {
    let (open, close) = match bytes.get(j) {
        Some(b'{') => (b'{', b'}'),
        Some(b'(') => (b'(', b')'),
        Some(b'[') => (b'[', b']'),
        _ => return None,
    };
    let mut depth = 0usize;
    let mut k = j;
    while k < bytes.len() {
        if bytes[k] == open {
            depth += 1;
        } else if bytes[k] == close {
            depth -= 1;
            if depth == 0 {
                return Some(k + 1);
            }
        }
        k += 1;
    }
    Some(bytes.len())
}

/// Remove comments and string literals — line (`//`), block (`/* */`), normal and
/// byte strings (`"…"`, `b"…"`, honoring `\"`/`\\`), and raw strings (`r"…"`,
/// `r#"…"#`, `br#"…"#`, any number of hashes) — so their contents can never be
/// mistaken for a `use` declaration: a `//` or a `use …;` written inside any of them
/// is ignored. Char literals are recognized minimally so a quote-bearing one (`'"'`)
/// does not open a spurious string; a lifetime (`'a`) is emitted as ordinary text.
/// Bare path expressions and macro-generated imports remain out of scope (PROJECT.md).
/// UTF-8 is preserved: kept bytes are decoded once and never split, because every
/// region boundary cut on is ASCII.
fn strip_comments_and_strings(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            // Line comment: drop to end of line.
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            // Block comment: Rust nests these, so track depth and drop through to the
            // `*/` that closes the outermost one — otherwise commented-out code that
            // itself contains a `/* */` would re-expose a `use` after the inner close.
            i += 2;
            let mut depth = 1usize;
            while i + 1 < bytes.len() && depth > 0 {
                if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if let Some((hashes, quote)) = raw_string_prefix(bytes, i) {
            // Raw string `r#*"…"#*`: no escapes; closed by `"` plus the same number
            // of `#`. Drop the whole literal so its text is never scanned.
            i = quote + 1;
            while i < bytes.len() {
                if bytes[i] == b'"' && raw_closing_matches(bytes, i + 1, hashes) {
                    i += 1 + hashes;
                    break;
                }
                i += 1;
            }
        } else if bytes[i] == b'"' {
            // String (or byte-string) literal: drop it, honoring `\"` and `\\`.
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
        } else if bytes[i] == b'\'' {
            // A char literal must be skipped whole so a quote it contains (`'"'`)
            // cannot open a spurious string. A lifetime (`'a`) has no closing quote
            // and is emitted as ordinary text.
            if i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                // Escaped char literal (`'\n'`, `'\''`, `'\u{…}'`): skip the opening
                // quote and the backslash, then the escaped character itself (which may
                // be a `'`, as in `'\''`), then scan to the closing quote. Skipping the
                // escaped character first is what keeps `'\''` from ending on its own
                // escaped quote and leaking the real closing quote.
                i += 2;
                if i < bytes.len() {
                    i += 1;
                }
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                i += 1;
            } else if i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
                // Simple char literal (`'x'`, `'"'`).
                i += 3;
            } else {
                // A lifetime or stray quote.
                out.push(bytes[i]);
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// If a raw string literal begins at `i` — `r` or `br` at a token boundary, then any
/// number of `#`, then `"` — return `(hash_count, index_of_opening_quote)`. A leading
/// `r`/`b` that is the tail of an identifier is not a prefix.
fn raw_string_prefix(bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    if i > 0 && is_ident_byte(bytes[i - 1]) {
        return None;
    }
    let mut j = i;
    if bytes.get(j) == Some(&b'b') {
        j += 1;
    }
    if bytes.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while bytes.get(j) == Some(&b'#') {
        hashes += 1;
        j += 1;
    }
    if bytes.get(j) == Some(&b'"') {
        Some((hashes, j))
    } else {
        None
    }
}

/// Whether `hashes` `#` characters start at `at` — the closing delimiter that, with
/// the preceding `"`, terminates a raw string opened with the same number of hashes.
fn raw_closing_matches(bytes: &[u8], at: usize, hashes: usize) -> bool {
    (0..hashes).all(|k| bytes.get(at + k) == Some(&b'#'))
}

/// Each `use … ;` statement paired with the module that lexically encloses it. The
/// walk tracks inline `mod name { … }` nesting by brace depth, so a `use` inside an
/// inline submodule is attributed to that submodule (e.g. `crate::a::inner`) rather
/// than the file's module (`crate::a`); `self`/`super` then resolve against the real
/// enclosing module, and a bare first segment inside an inline submodule is external
/// even when the file is the crate root. A `mod name;` with no inline body encloses
/// nothing. The text is already comment/string/macro-stripped, so every brace is
/// structural; a `use … ;` is consumed whole, so its own group braces
/// (`use a::{b, c};`) never perturb the depth.
fn use_trees_with_modules(source: &str, base_module: &str) -> Vec<(String, String)> {
    let bytes = source.as_bytes();
    let mut trees = Vec::new();
    // (inline module name, brace depth at which its body opened).
    let mut mod_stack: Vec<(String, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if keyword_starts_at(bytes, i, b"use") {
            let start = i + 3;
            match source[start..].find(';') {
                Some(rel) => {
                    trees.push((
                        effective_module(base_module, &mod_stack),
                        source[start..start + rel].trim().to_string(),
                    ));
                    i = start + rel + 1;
                    continue;
                }
                None => break,
            }
        }
        if let Some((name_start, name_end, brace)) = inline_mod_at(bytes, i) {
            mod_stack.push((
                canonical_segment(source[name_start..name_end].trim()).to_string(),
                depth,
            ));
            i = brace; // let the `{` arm below increment the depth
            continue;
        }
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                while mod_stack.last().is_some_and(|(_, d)| *d == depth) {
                    mod_stack.pop();
                }
            }
            _ => {}
        }
        i += 1;
    }
    trees
}

/// The module path enclosing a `use`, formed from the file's `base` module and the
/// names of the inline `mod`s currently open around it.
fn effective_module(base: &str, mod_stack: &[(String, usize)]) -> String {
    let mut module = base.to_string();
    for (name, _) in mod_stack {
        module.push_str("::");
        module.push_str(name);
    }
    module
}

/// Whether `keyword` appears as a standalone word starting exactly at `i` (bounded by
/// non-identifier bytes on both sides).
fn keyword_starts_at(bytes: &[u8], i: usize, keyword: &[u8]) -> bool {
    if !bytes[i..].starts_with(keyword) {
        return false;
    }
    let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
    let after = i + keyword.len();
    let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
    before_ok && after_ok
}

/// If an inline module declaration `mod <ident> {` begins at `i` (a standalone `mod`
/// keyword whose name is followed, after optional whitespace, by `{`), return
/// `(name_start, name_end, index_of_opening_brace)`; otherwise `None` — a `mod name;`
/// with no body, or not a declaration. Only an inline body encloses a `use`.
fn inline_mod_at(bytes: &[u8], i: usize) -> Option<(usize, usize, usize)> {
    if !is_mod_declaration_keyword(bytes, i) {
        return None;
    }
    let mut j = i + 3;
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    let name_start = j;
    while j < bytes.len() && !bytes[j].is_ascii_whitespace() && bytes[j] != b';' && bytes[j] != b'{'
    {
        j += 1;
    }
    let name_end = j;
    if name_end == name_start {
        return None;
    }
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    if bytes.get(j) == Some(&b'{') {
        Some((name_start, name_end, j))
    } else {
        None
    }
}

/// Canonicalize one path segment by stripping a leading raw-identifier marker
/// (`r#name` -> `name`). Rust resolves `mod r#type;` to the source file `type.rs`,
/// so the file-derived path, the `mod` declaration, and a `use r#type::…` path must
/// all reduce to the same module identity; this is the single place that reduction
/// lives. A segment with no `r#` prefix is returned unchanged.
fn canonical_segment(segment: &str) -> &str {
    segment.strip_prefix("r#").unwrap_or(segment)
}

/// Canonicalize a whole `::`-joined module path segment-by-segment (see
/// [`canonical_segment`]), so a boundary's declared path and an observed path compare
/// in one vocabulary regardless of which uses the raw-identifier form.
pub(crate) fn canonical_module_path(path: &str) -> String {
    path.split("::")
        .map(canonical_segment)
        .collect::<Vec<_>>()
        .join("::")
}

fn is_ident_byte(byte: u8) -> bool {
    // Any non-ASCII byte (>= 0x80) is a UTF-8 lead/continuation byte of a Unicode
    // identifier character (Rust allows non-ASCII identifiers, e.g. `use貓`). Treating
    // it as an identifier byte keeps keyword detection (`use`, `mod`) from firing inside
    // a Unicode identifier: `keyword_at("use貓;", …, "use")` must be `None`, since `use貓`
    // is one identifier, not the `use` keyword.
    byte == b'_' || byte.is_ascii_alphanumeric() || byte >= 0x80
}

/// Expand a use tree into leaf paths: `a::{b, c::d}` -> `a::b`, `a::c::d`; drop
/// `::*` and ` as alias`; `{self}` resolves to the prefix module.
fn expand_use_tree(tree: &str) -> Vec<String> {
    let tree = tree.trim();
    match tree.find('{') {
        Some(open) => {
            let prefix = tree[..open].trim();
            let inner = brace_content(&tree[open..]);
            let mut out = Vec::new();
            for part in split_top_commas(&inner) {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                if part == "self" {
                    let module = prefix.trim_end_matches(':').trim();
                    if !module.is_empty() {
                        out.push(module.to_string());
                    }
                } else {
                    out.extend(expand_use_tree(&format!("{prefix}{part}")));
                }
            }
            out
        }
        None => {
            let leaf = match tree.find(" as ") {
                Some(idx) => &tree[..idx],
                None => tree,
            };
            let leaf = leaf.trim().strip_suffix("::*").unwrap_or(leaf.trim());
            let leaf = leaf.trim_end_matches(':');
            if leaf.is_empty() {
                Vec::new()
            } else {
                vec![leaf.to_string()]
            }
        }
    }
}

/// Content inside the first `{ … }` of `s` (which must start with `{`), honoring
/// nesting.
fn brace_content(s: &str) -> String {
    let mut depth = 0;
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '{' => {
                depth += 1;
                if depth == 1 {
                    continue;
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        out.push(ch);
    }
    out
}

/// Split on commas at brace depth 0.
fn split_top_commas(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '{' => {
                depth += 1;
                current.push(ch);
            }
            '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

/// Resolve a use path to an absolute `crate::…` module path, or `None` if it refers
/// to an external crate. A first segment of `crate`/`self`/`super` resolves as usual.
/// A first segment that names a crate-root module (`root_modules`) resolves to
/// `crate::…` **only when the importing file is the crate root** (`current_module ==
/// "crate"`): there a sibling `mod` is in scope and shadows the extern prelude, so a
/// bare `use foo::…` is the local module. In a submodule a bare first segment reaches
/// only the extern prelude (a bare crate-root-module path there is either an external
/// crate or a compile error), so it is external. A path written with a leading `::`
/// (`use ::foo::…`) is explicitly the external/global crate and is always external. Any
/// other first segment is external (`None`).
fn normalize_module_path(
    path: &str,
    current_module: &str,
    root_modules: &[String],
) -> Option<String> {
    // A leading `::` is an explicit external/global path (`use ::serde::…`): it bypasses
    // the local-module shadow, so it is external even if a crate-root module shares the
    // name. Checked before segments are split so the marker is not lost.
    if path.trim_start().starts_with("::") {
        return None;
    }
    let segments: Vec<&str> = path
        .split("::")
        .map(|segment| canonical_segment(segment.trim()))
        .filter(|segment| !segment.is_empty())
        .collect();
    let (first, rest) = segments.split_first()?;
    match *first {
        "crate" => Some(segments.join("::")),
        "self" => {
            let mut out: Vec<&str> = current_module
                .split("::")
                .filter(|s| !s.is_empty())
                .collect();
            out.extend(rest);
            Some(out.join("::"))
        }
        "super" => {
            let mut out: Vec<&str> = current_module
                .split("::")
                .filter(|s| !s.is_empty())
                .collect();
            let mut tail = &segments[..];
            while let Some((segment, next)) = tail.split_first() {
                if *segment != "super" {
                    break;
                }
                out.pop();
                tail = next;
            }
            // An over-popped `super` (more `super`s than ancestors) drops the `crate`
            // root, leaving a path that is not crate-rooted — it names no internal module
            // (and the source does not compile). Return `None` rather than a malformed
            // root-less path, which would otherwise be mistaken for an outward edge.
            if out.first() != Some(&"crate") {
                return None;
            }
            out.extend(tail.iter().copied());
            Some(out.join("::"))
        }
        other => {
            // A bare first segment names a crate-root module only at the crate root,
            // where a sibling `mod` is in scope and shadows the extern prelude — there a
            // bare `use foo::…` is the local module, so resolve to `crate::…`. In a
            // submodule the same bare path reaches only the extern prelude (it is an
            // external crate, or a compile error), so it is external. Gating on
            // `current_module == "crate"` keeps a submodule's `use serde::…` external
            // even when a `mod serde;` exists at the crate root. (Explicit external is
            // written `::name`, handled above.)
            if current_module == "crate" && root_modules.iter().any(|module| module == other) {
                let mut out = vec!["crate"];
                out.extend(segments.iter().copied());
                Some(out.join("::"))
            } else {
                None
            }
        }
    }
}

/// Every `(file, module path)` that belongs to the governed `module` (it *is* `module`
/// or sits beneath it) **and** is reachable in the crate's module graph (see
/// [`reachable_modules`]). An undeclared orphan file is not a governed source file even
/// when its path would map under the module, because Rust never compiles it — so its
/// imports must not be observed. Operates on a precomputed file list so the crate's
/// source is scanned once per boundary.
pub(crate) fn governed_files(
    src_dir: &Path,
    files: &[PathBuf],
    module: &str,
    reachable: &std::collections::BTreeSet<String>,
) -> Vec<(PathBuf, String)> {
    let beneath = format!("{module}::");
    files
        .iter()
        .filter_map(|file| {
            let relative = file.strip_prefix(src_dir).ok()?;
            let module_path = file_module_path(relative);
            if !reachable.contains(&module_path) {
                return None;
            }
            if module_path == module || module_path.starts_with(&beneath) {
                Some((file.clone(), module_path))
            } else {
                None
            }
        })
        .collect()
}

/// The set of module paths reachable from the crate root via `mod` declarations — the
/// crate's real module graph as Rust compiles it, observed from source. The walk seeds at
/// `crate` (the crate-root file(s) `lib.rs` / `main.rs`), reads each reachable module's
/// file(s) for their top-level `mod name;` / `mod name { … }` declarations, and adds the
/// child `crate::…::name`.
///
/// A file's existence alone does **not** make it a module: an undeclared orphan file —
/// at the crate root (`src/serde.rs` with no `mod serde;`) or in a subtree
/// (`src/kernel/orphan.rs` that `kernel` never declares) — is never reached, so it is not
/// governed and its imports are not observed, matching the compiler (which never compiles
/// it). This realizes the spec's "a sibling `mod` is in scope" rule for the whole module
/// tree, not just the crate root. A reachable file that cannot be read is a scan error
/// ("cannot judge"), never a silent skip.
///
/// File-backed children only: an inline `mod name { … }` adds `crate::…::name` (harmless —
/// no file maps to it), but its own file-backed sub-`mod`s sit at brace depth > 0 and are
/// not walked — a documented partial-coverage gap, the safe direction (a missed import,
/// never a false one).
pub(crate) fn reachable_modules(
    src_dir: &Path,
    files: &[PathBuf],
) -> Result<std::collections::BTreeSet<String>, String> {
    // Index files by their path-derived module path so a module's file(s) are found fast.
    let mut by_module: std::collections::BTreeMap<String, Vec<&PathBuf>> = Default::default();
    for file in files {
        if let Ok(relative) = file.strip_prefix(src_dir) {
            by_module
                .entry(file_module_path(relative))
                .or_default()
                .push(file);
        }
    }

    let mut reachable = std::collections::BTreeSet::new();
    reachable.insert("crate".to_string());
    let mut queue = vec!["crate".to_string()];
    while let Some(module) = queue.pop() {
        let Some(module_files) = by_module.get(&module) else {
            continue; // an inline module owns no file of its own; nothing to read
        };
        for file in module_files {
            let text = std::fs::read_to_string(file)
                .map_err(|err| format!("cannot read source file '{}': {err}", file.display()))?;
            for child in declared_modules(&text) {
                let child_path = format!("{module}::{child}");
                if reachable.insert(child_path.clone()) {
                    queue.push(child_path);
                }
            }
        }
    }
    Ok(reachable)
}

/// Names of modules declared at the top level (brace depth 0) of `source` via
/// `mod <ident>;` or `mod <ident> { … }`, at any visibility (`pub mod`, `pub(crate)
/// mod`, …). Comments, string/char literals, and macro bodies are stripped first, so a
/// commented-out, quoted, or macro-generated `mod` is not counted; a `mod` nested inside
/// another item (depth > 0) declares a child module, not a crate-root one, and is
/// skipped. Names are canonicalized (`r#name` -> `name`). Robust over malformed input:
/// it never panics (the same tolerance as the `use` scanner).
fn declared_modules(source: &str) -> Vec<String> {
    // Strip macro bodies as well as comments/strings, the same hygiene the `use`
    // scanner applies: a `mod` written inside a macro body is macro-generated and out
    // of scope, so it must not be observed as a real declaration. (A `macro_rules!`
    // body is already excluded by brace depth; this also closes the `()`/`[]`-delimited
    // invocation gap, where `mod` would otherwise sit at brace depth 0.)
    let cleaned = strip_macro_bodies(&strip_comments_and_strings(source));
    let bytes = cleaned.as_bytes();
    let mut names = Vec::new();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
            }
            b'm' if depth == 0 && is_mod_declaration_keyword(bytes, i) => {
                // Read the identifier after `mod`, then confirm a `;` or `{` follows so
                // only real declarations (not a stray `mod` token) are recorded.
                let mut j = i + 3;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let start = j;
                while j < bytes.len()
                    && !bytes[j].is_ascii_whitespace()
                    && bytes[j] != b';'
                    && bytes[j] != b'{'
                {
                    j += 1;
                }
                let ident = cleaned[start..j].trim();
                let mut k = j;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                if !ident.is_empty() && matches!(bytes.get(k), Some(b';') | Some(b'{')) {
                    names.push(canonical_segment(ident).to_string());
                }
                i += 3;
            }
            _ => i += 1,
        }
    }
    names
}

/// Whether a standalone `mod` keyword begins at `i` (bounded by non-identifier bytes) —
/// the head of a possible module declaration, not a substring like `module`.
fn is_mod_declaration_keyword(bytes: &[u8], i: usize) -> bool {
    keyword_starts_at(bytes, i, b"mod")
}

/// The module path of a source file from its path relative to `src/`:
/// `lib.rs`/`main.rs`/`mod.rs` contribute no segment; `kernel/foo.rs` ->
/// `crate::kernel::foo`.
fn file_module_path(relative: &Path) -> String {
    let components: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    let mut segments = vec![String::from("crate")];
    let last = components.len().saturating_sub(1);
    for (index, component) in components.iter().enumerate() {
        if index == last {
            let stem = component.strip_suffix(".rs").unwrap_or(component);
            if !matches!(stem, "mod" | "lib" | "main") {
                segments.push(canonical_segment(stem).to_string());
            }
        } else {
            segments.push(canonical_segment(component).to_string());
        }
    }
    segments.join("::")
}

/// All `.rs` files under `dir`, recursively. A directory that cannot be read (or an
/// entry that cannot be resolved) is a scan error, never a silent skip: a skipped
/// subtree could hide a real module-boundary violation — "cannot judge", not "nothing
/// to judge", the same rule as an unreadable governed file.
pub(crate) fn rust_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|err| {
        format!(
            "cannot read governed source directory '{}': {err}",
            dir.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            format!(
                "cannot read an entry in governed source directory '{}': {err}",
                dir.display()
            )
        })?;
        let path = entry.path();
        if path.is_dir() {
            found.extend(rust_files(&path)?);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            found.push(path);
        }
    }
    // Sort so the governed-file order — and hence module-violation order in the report —
    // is deterministic, independent of the filesystem's `read_dir` order.
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_expands_groups_and_resolves_relative_imports() {
        let source = r#"
            // a line comment mentioning use crate::ignored::me;
            use crate::a::{b, c::d};
            use super::sibling::X;
            use self::inner::Y;
            use serde::Deserialize;
            use crate::z::*;
        "#;
        let imports = imported_module_paths(source, "crate::kernel", &[]);
        assert!(imports.contains(&"crate::a::b".to_string()), "{imports:?}");
        assert!(
            imports.contains(&"crate::a::c::d".to_string()),
            "{imports:?}"
        );
        // `super` from crate::kernel resolves to crate.
        assert!(
            imports.contains(&"crate::sibling::X".to_string()),
            "{imports:?}"
        );
        // `self` resolves against the current module.
        assert!(
            imports.contains(&"crate::kernel::inner::Y".to_string()),
            "{imports:?}"
        );
        // glob keeps the module prefix.
        assert!(imports.contains(&"crate::z".to_string()), "{imports:?}");
        // external first segment is ignored, and commented-out imports are not seen.
        assert!(!imports.iter().any(|p| p.contains("serde")), "{imports:?}");
        assert!(
            !imports.iter().any(|p| p.contains("ignored")),
            "{imports:?}"
        );
    }

    #[test]
    fn super_past_the_crate_root_is_not_an_internal_module() {
        // `crate::a` has one ancestor (`crate`); `super::super` over-pops past the root.
        // Such a path names no internal module (and does not compile), so it must not be
        // observed — never a malformed root-less path like "other::X".
        let over = "use super::super::other::X;\n";
        assert!(
            imported_module_paths(over, "crate::a", &[]).is_empty(),
            "over-popped super must yield no import: {:?}",
            imported_module_paths(over, "crate::a", &[])
        );
        // A single `super` from `crate::a` still resolves to `crate::other::X`.
        let ok = "use super::other::X;\n";
        assert_eq!(
            imported_module_paths(ok, "crate::a", &[]),
            vec!["crate::other::X".to_string()],
        );
    }

    #[test]
    fn scanner_ignores_comments_and_string_literals() {
        // A `//` inside a string must not eat a real `use` later on the same line.
        let url = r#"let u = "http://example.com"; use crate::real::A;"#;
        let imports = imported_module_paths(url, "crate::kernel", &[]);
        assert!(
            imports.contains(&"crate::real::A".to_string()),
            "{imports:?}"
        );

        // A `use …;` written inside a string is not a real import.
        let in_string = r#"let s = "use crate::ghost::Z;";"#;
        assert!(
            imported_module_paths(in_string, "crate::kernel", &[]).is_empty(),
            "a use inside a string must not be observed"
        );

        // A quote-bearing char literal must not open a spurious string and swallow code.
        let quote_char = r#"let q = '"'; use crate::real::B;"#;
        let imports = imported_module_paths(quote_char, "crate::kernel", &[]);
        assert!(
            imports.contains(&"crate::real::B".to_string()),
            "{imports:?}"
        );

        // A lifetime must not break use detection or produce a spurious path.
        let lifetime = "fn f<'a>(x: &'a str) {} use crate::a::b;";
        let imports = imported_module_paths(lifetime, "crate::kernel", &[]);
        assert_eq!(imports, vec!["crate::a::b".to_string()], "{imports:?}");
    }

    #[test]
    fn scanner_handles_nested_block_comments() {
        // Rust nests block comments. Commenting out code that itself contains a
        // `/* */` must not let the inner `*/` re-expose the rest as live code: a
        // `use` inside the (nested) comment must not be observed, while a real `use`
        // after the outer close still is.
        let source = r#"
            /*
            fn old() {
                /* tweak later */
                use crate::legacy::Thing;
            }
            */
            use crate::current::A;
        "#;
        let imports = imported_module_paths(source, "crate::kernel", &[]);
        assert!(
            imports.contains(&"crate::current::A".to_string()),
            "the real use after the nested comment must be observed: {imports:?}"
        );
        assert!(
            !imports.iter().any(|p| p.contains("legacy")),
            "a use inside a nested block comment must not be observed: {imports:?}"
        );
    }

    #[test]
    fn scanner_resolves_root_relative_bare_use() {
        // A root-relative bare `use kernel::…` (legal only at the crate root) names a
        // crate-root module, not an external crate, so it resolves to `crate::kernel::…`.
        // An unknown first segment is still external. With no root modules known, the
        // bare path stays external (the conservative pre-fix behavior).
        let source = "use kernel::Thing; use serde::Deserialize;";
        let roots = vec!["kernel".to_string()];
        let imports = imported_module_paths(source, "crate", &roots);
        assert!(
            imports.contains(&"crate::kernel::Thing".to_string()),
            "a bare use of a crate-root module must resolve to crate::…: {imports:?}"
        );
        assert!(
            !imports.iter().any(|p| p.contains("serde")),
            "an unknown first segment stays external: {imports:?}"
        );
        assert!(
            imported_module_paths(source, "crate", &[])
                .iter()
                .all(|p| !p.contains("kernel")),
            "with no root modules known, the bare path is treated as external"
        );
    }

    #[test]
    fn scanner_treats_leading_colon_path_as_external() {
        // `use ::serde::…` is an explicit external/global path: the leading `::` bypasses
        // the local-module shadow, so it is external even when a crate-root module shares
        // the name. (Before the fix the leading `::` was dropped and the path was
        // mis-resolved as the internal `crate::serde::…`.)
        let roots = vec!["serde".to_string()];
        assert!(
            imported_module_paths("use ::serde::Deserialize;", "crate", &roots).is_empty(),
            "a leading-:: path must be external even when its head matches a root module"
        );
        // Sanity: without the leading `::`, the same head IS the local module, because a
        // crate-root module shadows the extern prelude in a bare `use` path.
        assert!(
            imported_module_paths("use serde::Deserialize;", "crate", &roots)
                .contains(&"crate::serde::Deserialize".to_string()),
            "a bare head matching a root module resolves locally (shadowing rule)"
        );
    }

    #[test]
    fn scanner_resolves_root_relative_use_only_at_crate_root() {
        // The crate-root-module shadow holds ONLY at the crate root. In a submodule a
        // bare `use serde::…` reaches the extern prelude (external), even when `serde`
        // is a crate-root module — matching the compiler. Resolving it to
        // `crate::serde::…` would be a false positive that fails a module boundary.
        let roots = vec!["serde".to_string()];
        assert!(
            imported_module_paths("use serde::Value;", "crate", &roots)
                .contains(&"crate::serde::Value".to_string()),
            "at the crate root, a bare crate-root-module path resolves locally"
        );
        assert!(
            imported_module_paths("use serde::Value;", "crate::sub", &roots).is_empty(),
            "in a submodule, a bare first segment is external even if it matches a root module"
        );
    }

    #[test]
    fn scanner_preserves_non_ascii_module_paths() {
        // strip is UTF-8 safe: a non-ASCII module path survives stripping intact.
        let source = "use crate::café::Item;";
        let imports = imported_module_paths(source, "crate::kernel", &[]);
        assert!(
            imports.contains(&"crate::café::Item".to_string()),
            "{imports:?}"
        );
    }

    #[test]
    fn scanner_ignores_raw_and_byte_strings() {
        // A `use …;` inside a raw string (any hash count) is not an import.
        for src in [
            r##"let s = r"use crate::ghost::Z;";"##,
            r##"let s = r#"use crate::ghost::Z;"#;"##,
            r##"let s = br#"use crate::ghost::Z;"#;"##,
            r#"let s = b"use crate::ghost::Z;";"#,
        ] {
            assert!(
                imported_module_paths(src, "crate::kernel", &[]).is_empty(),
                "a use inside a (raw/byte) string must not be observed: {src}"
            );
        }

        // A `//` and an inner `"#` inside a raw string must not eat a following use.
        // (Two outer hashes so the inner `"#` does not close it.)
        let tricky = r####"let s = r##"http://x "# inside"##; use crate::real::C;"####;
        let imports = imported_module_paths(tricky, "crate::kernel", &[]);
        assert!(
            imports.contains(&"crate::real::C".to_string()),
            "{imports:?}"
        );

        // `r` / `b` as ordinary identifiers (not raw-string prefixes) are unaffected.
        let idents = "let r = 1; let b = 2; use crate::real::D;";
        assert_eq!(
            imported_module_paths(idents, "crate::kernel", &[]),
            vec!["crate::real::D".to_string()]
        );
    }

    #[test]
    fn scanner_does_not_panic_on_odd_input() {
        // Truncated / malformed inputs must never panic (robustness over precision).
        for src in [
            "r#\"unterminated raw string",
            "\"unterminated string",
            "/* unterminated block",
            "'",
            "r",
            "use ",
            "use crate::",
            "mod ",
            "mod foo",
            "}",
            "",
        ] {
            let _ = imported_module_paths(src, "crate::kernel", &[]);
            let _ = declared_modules(src);
            let _ = strip_macro_bodies(src);
        }
    }

    #[test]
    fn use_inside_a_macro_body_is_not_observed() {
        // A `use` written inside a macro — a `macro_rules!` definition OR a macro
        // invocation — is a macro-generated import (out of scope): it must not be
        // observed. A real `use` outside the macro still is.
        let source = r#"
            macro_rules! m {
                () => { use crate::ghost::Thing; };
                ($x:tt) => {{ use crate::ghost::Other; }};
            }
            with_imports! { use crate::ghost::FromInvocation; }
            use crate::real::A;
        "#;
        let imports = imported_module_paths(source, "crate", &[]);
        assert_eq!(
            imports,
            vec!["crate::real::A".to_string()],
            "macro definition and invocation bodies are skipped; the real use is kept: {imports:?}"
        );
        // Every body delimiter form ({}, (), []) is skipped, for both definitions and
        // invocations.
        for body in [
            "macro_rules! m { () => { use crate::ghost::T; }; }",
            "macro_rules! m ( () => { use crate::ghost::T; }; )",
            "macro_rules! m [ () => { use crate::ghost::T; }; ]",
            "some_macro! { use crate::ghost::T; }",
            "some_macro!( use crate::ghost::T; )",
            "some_macro![ use crate::ghost::T; ]",
        ] {
            assert!(
                imported_module_paths(body, "crate", &[]).is_empty(),
                "no import observed from a macro body: {body}"
            );
        }
        // `!=` and unary `!` are not macro invocations — a following real use is kept.
        let not_macros = "let _ = a != b; let _ = !flag; use crate::real::B;";
        assert!(
            imported_module_paths(not_macros, "crate", &[]).contains(&"crate::real::B".to_string()),
            "`!=` / unary `!` must not be treated as a macro invocation"
        );
    }

    #[test]
    fn keyword_detection_does_not_fire_inside_a_unicode_identifier() {
        // Rust allows non-ASCII identifiers. `use貓` / `mod貓` are single identifiers, so
        // the leading `use` / `mod` is NOT a keyword: the byte after it (the lead byte of
        // `貓`, >= 0x80) is an identifier byte. A regression guard for ASCII-only
        // `is_ident_byte`, which used to split the identifier and fire a false keyword.
        assert!(!keyword_starts_at("use貓;".as_bytes(), 0, b"use"));
        assert!(!keyword_starts_at("mod貓 {}".as_bytes(), 0, b"mod"));
        // The genuine keyword (followed by whitespace) is still detected.
        assert!(keyword_starts_at("use 貓;".as_bytes(), 0, b"use"));
        // And nothing is observed as an import or a declared module from the identifier.
        assert!(imported_module_paths("fn use貓() {}", "crate", &[]).is_empty());
        assert!(declared_modules("fn mod貓() {}").is_empty());
    }

    #[test]
    fn declared_modules_finds_only_top_level_declarations() {
        let source = r#"
            pub mod kernel;
            mod projection;
            pub(crate) mod runner;
            mod inline { mod nested_child; }   // nested_child is depth 1, not a root module
            // mod commented_out;
            fn f() { let _ = "mod string_literal;"; }
        "#;
        let mut mods = declared_modules(source);
        mods.sort();
        assert_eq!(
            mods,
            vec![
                "inline".to_string(),
                "kernel".to_string(),
                "projection".to_string(),
                "runner".to_string(),
            ],
            "only top-level mod declarations count; nested, commented, and quoted are excluded"
        );
    }

    #[test]
    fn reachable_modules_follows_mod_declarations_not_filenames() {
        // The crate root declares `mod kernel;`, but two orphan files exist that no `mod`
        // brings into scope: a root orphan (`serde.rs`) and a subtree orphan
        // (`kernel/orphan.rs`, which `kernel.rs` never declares). Only `crate` and
        // `crate::kernel` are reachable; the orphans are not — at the root OR in a subtree.
        let dir = std::env::temp_dir().join(format!("modou-reach-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("kernel")).expect("create temp src/kernel");
        std::fs::write(
            src.join("lib.rs"),
            "pub mod kernel;\nuse serde::Deserialize;\n",
        )
        .expect("write lib.rs");
        std::fs::write(src.join("kernel.rs"), "// kernel declares no submodule\n")
            .expect("write kernel.rs");
        std::fs::write(src.join("serde.rs"), "// root orphan, undeclared\n")
            .expect("write serde.rs");
        std::fs::write(
            src.join("kernel/orphan.rs"),
            "use crate::projection::Thing;\n",
        )
        .expect("write kernel/orphan.rs");

        let files = rust_files(&src).expect("list files");
        let reachable = reachable_modules(&src, &files).expect("walk modules");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(reachable.contains("crate"), "{reachable:?}");
        assert!(
            reachable.contains("crate::kernel"),
            "a declared `mod kernel;` is reachable: {reachable:?}"
        );
        assert!(
            !reachable.contains("crate::serde"),
            "an undeclared root orphan is not reachable: {reachable:?}"
        );
        assert!(
            !reachable.contains("crate::kernel::orphan"),
            "an undeclared subtree orphan is not reachable: {reachable:?}"
        );
    }

    #[test]
    fn raw_identifiers_are_canonicalized() {
        // `mod r#type;` compiles to `type.rs`, so the `mod` token, the `use` path, and
        // the file path must all reduce to the same identity (`type`).
        assert_eq!(canonical_segment("r#type"), "type");
        assert_eq!(canonical_segment("type"), "type");
        assert_eq!(
            canonical_module_path("crate::r#type::r#mod"),
            "crate::type::mod"
        );
        assert_eq!(
            declared_modules("pub mod r#type;"),
            vec!["type".to_string()]
        );
        assert_eq!(
            imported_module_paths("use crate::r#type::Thing;", "crate", &[]),
            vec!["crate::type::Thing".to_string()],
            "a raw-identifier use path is canonicalized to its plain form"
        );
    }

    #[test]
    fn use_inside_an_inline_module_is_attributed_to_that_module() {
        // A `self`/`super` import inside an inline `mod inner { … }` resolves against the
        // inline submodule, not the file's module: `self` -> crate::a::inner::…, and
        // `super` from crate::a::inner -> crate::a.
        let source = "mod inner { use self::leaf::Thing; use super::sibling::X; }";
        let imports = imported_module_paths(source, "crate::a", &[]);
        assert!(
            imports.contains(&"crate::a::inner::leaf::Thing".to_string()),
            "self must resolve against the inline submodule: {imports:?}"
        );
        assert!(
            imports.contains(&"crate::a::sibling::X".to_string()),
            "super from the inline submodule resolves to crate::a: {imports:?}"
        );
        // A bare first segment inside an inline submodule is external even when the file
        // IS the crate root (the enclosing module is crate::inner, not crate).
        let bare = imported_module_paths(
            "mod inner { use kernel::Thing; }",
            "crate",
            &["kernel".to_string()],
        );
        assert!(
            bare.is_empty(),
            "a bare use inside an inline submodule is external: {bare:?}"
        );
        // A top-level use (no inline module) is unchanged.
        assert_eq!(
            imported_module_paths("use self::leaf::Thing;", "crate::a", &[]),
            vec!["crate::a::leaf::Thing".to_string()]
        );
    }

    #[test]
    fn declared_modules_ignores_a_mod_inside_a_macro_invocation() {
        // A `mod` written inside a macro body is macro-generated and out of scope — the
        // same rule the `use` scanner already applies. `()`/`[]`-delimited invocations
        // were the gap (a `macro_rules!` body is already excluded by brace depth).
        assert!(declared_modules("some_macro!( mod ghost; );").is_empty());
        assert!(declared_modules("some_macro![ mod ghost; ];").is_empty());
        assert!(declared_modules("macro_rules! m { () => { mod ghost; }; }").is_empty());
        // A real top-level declaration is still found.
        assert_eq!(declared_modules("mod real;"), vec!["real".to_string()]);
    }

    #[test]
    fn escaped_quote_char_literal_is_consumed_whole() {
        // `'\''` must be skipped as a whole so the following string's fake `use` is still
        // stripped and the real `use` after it is observed (a regression guard for the
        // escaped-char skip that used to leak the closing quote).
        let src = r#"let _q = '\''; let _s = "use crate::ghost::Z;"; use crate::real::A;"#;
        assert_eq!(
            imported_module_paths(src, "crate::kernel", &[]),
            vec!["crate::real::A".to_string()],
            "an escaped-quote char literal must not leak and expose a fake use"
        );
    }
}
