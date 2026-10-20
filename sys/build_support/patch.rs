use std::{fs, io, path::Path};

const QUICKJS_BASELINE_FNV64: u64 = 0x8aa108a5eef213d4;
const EXPECTED_PATCHES: [(&str, u64); 30] = [
    ("0001-rquickjs-jit.patch", 0x60420d3ebfdf2a1d),
    ("0002-runtime-feedback.patch", 0xb3bc90bb226a9dac),
    ("0003-element-layout.patch", 0x0bde3e3bcdb1a36d),
    ("0004-tier1-globals.patch", 0x6a3298e4dda1473e),
    ("0005-tier1-constructors.patch", 0x713b227fd9a38a14),
    ("0006-tier1-regexp.patch", 0x8d27c52d21b4c739),
    ("0007-msan-slot-boundary.patch", 0x80f331b455e8c47a),
    ("0008-tier1-atom-values.patch", 0x580d79051ac98a26),
    ("0009-tier1-arith-slow.patch", 0x50e3b82eefea0a4a),
    ("0010-native-entry-cache.patch", 0x13a15e6a0be8208b),
    ("0011-helper-pc-cursor.patch", 0x0cace719f30249a8),
    ("0012-shape-retention.patch", 0x1c2b7897cd04965d),
    ("0013-feedback-gate.patch", 0xf98604a90a3429ce),
    ("0014-inactive-probes.patch", 0x9b66a35572123ae8),
    ("0015-cold-object-probes.patch", 0xadc299364845bfd6),
    ("0017-shape-generation.patch", 0xa8ee772c985cc7f4),
    ("0018-inline-frame-recovery.patch", 0x3076ce7b21ae9e98),
    ("0019-array-feedback.patch", 0xa03b0c8185462a50),
    ("0020-typed-array-guard.patch", 0x2401765b8991338a),
    ("0021-helper-boundary.patch", 0x77de2d26db687100),
    ("0022-inline-refcount.patch", 0xe0a170c40a99f808),
    ("0023-fast-native-entry.patch", 0x882399f2bc1eafba),
    ("0025-tier1-closures.patch", 0xda8e552a1ce00901),
    ("0026-tier1-generic-ops.patch", 0x8765f909b288278c),
    ("0027-tier1-exception-regions.patch", 0xe46b2be58d78024e),
    ("0028-tier1-iteration.patch", 0xdb379243b8f4cd5e),
    ("0029-native-call-convention.patch", 0x6c616ed95c98854b),
    ("0032-object-fast-paths.patch", 0x72cc8c9c593a4bbf),
    ("0033-quickjs-ng-0.16.patch", 0xc4de85519d1ecc87),
    ("0034-portable-slot-none.patch", 0xb1e88a6341b8ebda),
];
pub(crate) const BASELINE_FILES: [&str; 19] = [
    "api-test.c",
    "builtin-array-fromasync.h",
    "builtin-iterator-zip-keyed.h",
    "builtin-iterator-zip.h",
    "cutils.h",
    "dtoa.c",
    "dtoa.h",
    "libregexp-opcode.h",
    "libregexp.c",
    "libregexp.h",
    "libunicode-table.h",
    "libunicode.c",
    "libunicode.h",
    "list.h",
    "quickjs-atom.h",
    "quickjs-c-atomics.h",
    "quickjs-opcode.h",
    "quickjs.c",
    "quickjs.h",
];
const PATCHED_FILE_FINGERPRINTS: [(&str, u64); 21] = [
    ("api-test.c", 0x66cbd3282568024d),
    ("builtin-array-fromasync.h", 0x37e111a1a062265e),
    ("builtin-iterator-zip-keyed.h", 0x321d8804fc19e3bf),
    ("builtin-iterator-zip.h", 0x9cb244947a975786),
    ("cutils.h", 0xa68306399e4f16e8),
    ("dtoa.c", 0x1aff2cc2d08cf224),
    ("dtoa.h", 0x602f733bbb27f6e1),
    ("libregexp-opcode.h", 0x926b0c2be4da4499),
    ("libregexp.c", 0xb12bb3778b6fe314),
    ("libregexp.h", 0x93548abacc6488ee),
    ("libunicode-table.h", 0x9a35f51667742c4d),
    ("libunicode.c", 0x228d92f88863aaff),
    ("libunicode.h", 0xd54ba7ea1e52cf68),
    ("list.h", 0xb33770f7b76da3d8),
    ("quickjs-atom.h", 0x30b49116b6a2aa99),
    ("quickjs-c-atomics.h", 0x490b0f29f6313fc0),
    ("quickjs.c", 0xbcd96d9859f06421),
    ("quickjs-jit.h", 0x2602b4ce99957784),
    ("quickjs-jit-helpers.h", 0x7a0c9a1fbeb20014),
    ("quickjs-opcode.h", 0x3d05cfdf5cf72930),
    ("quickjs.h", 0x32a80f3fee4f3bb4),
];

pub fn apply_patch_set(source_dir: &Path, patch_dir: &Path) -> io::Result<()> {
    let actual = baseline_fingerprint(source_dir)?;
    if actual != QUICKJS_BASELINE_FNV64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "QuickJS baseline mismatch: expected 0fdea21 source-set fingerprint \
                 {QUICKJS_BASELINE_FNV64:#018x}, got {actual:#018x}"
            ),
        ));
    }

    let mut patches = fs::read_dir(patch_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    patches.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "patch")
    });
    patches.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    let actual_names = patches
        .iter()
        .map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 patch filename"))?;
    let expected_names = EXPECTED_PATCHES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    if actual_names != expected_names {
        return invalid("QuickJS patch-set manifest mismatch");
    }
    let patch_texts = patches
        .iter()
        .zip(EXPECTED_PATCHES)
        .map(|(patch, (_, expected_digest))| {
            let bytes = fs::read(patch)?;
            // Git may materialize text files with CRLF on Windows.  The
            // manifest pins the canonical LF bytes, so accept only that
            // newline-only checkout transformation; every other byte remains
            // covered by the digest below.
            let canonical = canonicalize_crlf(&bytes);
            if fnv1a64(&canonical) != expected_digest {
                return invalid("QuickJS integration patch digest mismatch");
            }
            std::str::from_utf8(&canonical)
                .map(str::to_owned)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "patch is not UTF-8"))
        })
        .collect::<io::Result<Vec<_>>>()?;
    normalize_baseline_newlines(source_dir)?;
    for patch in &patch_texts {
        apply_unified_patch(source_dir, patch)?;
    }
    for (file, expected) in PATCHED_FILE_FINGERPRINTS {
        let actual = fnv1a64(&fs::read(source_dir.join(file))?);
        if actual != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "patched {file} fingerprint mismatch: expected {expected:#018x}, \
                     got {actual:#018x}"
                ),
            ));
        }
    }
    Ok(())
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_from(0xcbf29ce484222325, bytes)
}

fn fnv1a64_from(seed: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(seed, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn baseline_fingerprint(source_dir: &Path) -> io::Result<u64> {
    let mut hash = 0xcbf29ce484222325;
    for file in BASELINE_FILES {
        hash = fnv1a64_from(hash, file.as_bytes());
        hash = fnv1a64_from(hash, &[0]);
        let bytes = fs::read(source_dir.join(file))?;
        hash = fnv1a64_from(hash, &canonicalize_crlf(&bytes));
    }
    Ok(hash)
}

fn normalize_baseline_newlines(source_dir: &Path) -> io::Result<()> {
    for file in BASELINE_FILES {
        let path = source_dir.join(file);
        let bytes = fs::read(&path)?;
        let canonical = canonicalize_crlf(&bytes);
        if bytes != canonical {
            fs::write(path, canonical)?;
        }
    }
    Ok(())
}

fn canonicalize_crlf(bytes: &[u8]) -> Vec<u8> {
    let mut canonical = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
        }
        canonical.push(bytes[index]);
        index += 1;
    }
    canonical
}

fn apply_unified_patch(root: &Path, patch: &str) -> io::Result<()> {
    let lines = patch.split_inclusive('\n').collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        if !lines[index].starts_with("diff --git ") {
            index += 1;
            continue;
        }
        index += 1;
        while index < lines.len() && !lines[index].starts_with("--- ") {
            index += 1;
        }
        if index + 1 >= lines.len() || !lines[index + 1].starts_with("+++ ") {
            return invalid("patch file header is incomplete");
        }
        let old_path = patch_path(lines[index].trim_end(), "--- ")?;
        let new_path = patch_path(lines[index + 1].trim_end(), "+++ ")?;
        index += 2;
        let path = new_path.as_deref().or(old_path.as_deref()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "patch has no target path")
        })?;
        if !PATCHED_FILE_FINGERPRINTS
            .iter()
            .any(|(allowed, _)| *allowed == path)
        {
            return invalid("patch targets a file outside the pinned source manifest");
        }
        let original = match old_path {
            Some(_) => fs::read_to_string(root.join(path))?,
            None => String::new(),
        };
        let original_lines = original.split_inclusive('\n').collect::<Vec<_>>();
        let mut output = String::with_capacity(original.len());
        let mut cursor = 0;

        while index < lines.len() && !lines[index].starts_with("diff --git ") {
            if !lines[index].starts_with("@@ ") {
                index += 1;
                continue;
            }
            let (old_start, old_count, new_count) = parse_hunk_header(lines[index])?;
            let target = old_start.saturating_sub(1);
            if target < cursor || target > original_lines.len() {
                return invalid("patch hunk has an invalid or overlapping source range");
            }
            for line in &original_lines[cursor..target] {
                output.push_str(line);
            }
            cursor = target;
            index += 1;
            let mut removed = 0;
            let mut added = 0;
            while index < lines.len()
                && !lines[index].starts_with("@@ ")
                && !lines[index].starts_with("diff --git ")
            {
                let line = lines[index];
                match line.as_bytes().first().copied() {
                    Some(b' ') => {
                        require_source_line(&original_lines, cursor, &line[1..])?;
                        output.push_str(&line[1..]);
                        cursor += 1;
                        removed += 1;
                        added += 1;
                    }
                    Some(b'-') => {
                        require_source_line(&original_lines, cursor, &line[1..])?;
                        cursor += 1;
                        removed += 1;
                    }
                    Some(b'+') => {
                        output.push_str(&line[1..]);
                        added += 1;
                    }
                    Some(b'\\') => {}
                    _ => return invalid("patch hunk contains an invalid line prefix"),
                }
                index += 1;
            }
            if removed != old_count || added != new_count {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "patch hunk at old line {old_start} counted {removed}/{added} lines, expected {old_count}/{new_count}"
                    ),
                ));
            }
        }
        for line in &original_lines[cursor..] {
            output.push_str(line);
        }
        if new_path.is_some() {
            fs::write(root.join(path), output)?;
        } else {
            fs::remove_file(root.join(path))?;
        }
    }
    Ok(())
}

fn patch_path(line: &str, prefix: &str) -> io::Result<Option<String>> {
    let path = line
        .strip_prefix(prefix)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid patch path"))?
        .split_whitespace()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty patch path"))?;
    if path == "/dev/null" {
        return Ok(None);
    }
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unsafe patch path"))?;
    if path.contains('/') || path == "." || path == ".." {
        return invalid("patch paths must name one QuickJS source file");
    }
    Ok(Some(path.to_owned()))
}

fn parse_hunk_header(line: &str) -> io::Result<(usize, usize, usize)> {
    let body = line
        .strip_prefix("@@ -")
        .and_then(|line| line.split_once(" @@").map(|parts| parts.0))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid hunk header"))?;
    let (old, new) = body
        .split_once(" +")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid hunk ranges"))?;
    let (old_start, old_count) = parse_range(old)?;
    let (_, new_count) = parse_range(new)?;
    Ok((old_start, old_count, new_count))
}

fn parse_range(range: &str) -> io::Result<(usize, usize)> {
    let (start, count) = range.split_once(',').unwrap_or((range, "1"));
    Ok((
        start
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid hunk start"))?,
        count
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid hunk count"))?,
    ))
}

fn require_source_line(original: &[&str], cursor: usize, expected: &str) -> io::Result<()> {
    if original.get(cursor).copied() != Some(expected) {
        let nearby = original
            .iter()
            .enumerate()
            .skip(cursor.saturating_sub(256))
            .take(512)
            .find_map(|(index, line)| (*line == expected).then_some(index.saturating_add(1)));
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "patch context mismatch at source line {}: expected {:?}, found {:?}, nearby match {:?}",
                cursor.saturating_add(1),
                expected,
                original.get(cursor).copied(),
                nearby
            ),
        ));
    }
    Ok(())
}

fn invalid<T>(message: &str) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidData, message))
}
