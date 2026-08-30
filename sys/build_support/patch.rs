use std::{fs, io, path::Path};

const QUICKJS_BASELINE_FNV64: u64 = 0x3302_116a_b0fc_269c;
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
const PATCHED_FILE_FINGERPRINTS: [(&str, u64); 4] = [
    ("api-test.c", 0x7af4_1bd9_e9b2_68b4),
    ("quickjs.c", 0x9700_47a0_ca58_f454),
    ("quickjs-jit.h", 0xd531_9715_3b42_8610),
    ("quickjs-jit-helpers.h", 0xfc63_8662_ecd7_71c7),
];

pub fn apply_patch_set(source_dir: &Path, patch_dir: &Path) -> io::Result<()> {
    let actual = baseline_fingerprint(source_dir)?;
    if actual != QUICKJS_BASELINE_FNV64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "QuickJS baseline mismatch: expected fd0a021 source-set fingerprint \
                 {QUICKJS_BASELINE_FNV64:#018x}, got {actual:#018x}"
            ),
        ));
    }

    let mut patches = fs::read_dir(patch_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "patch")
        })
        .collect::<Vec<_>>();
    patches.sort();
    if patches.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no QuickJS integration patches found",
        ));
    }
    for patch in patches {
        apply_unified_patch(source_dir, &fs::read_to_string(patch)?)?;
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
        hash = fnv1a64_from(hash, &fs::read(source_dir.join(file))?);
    }
    Ok(hash)
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
                return invalid("patch hunk line counts do not match its header");
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
        return invalid("patch context does not match the pinned QuickJS baseline");
    }
    Ok(())
}

fn invalid<T>(message: &str) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidData, message))
}
