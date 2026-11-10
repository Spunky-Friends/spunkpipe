use crate::{Error, Result};
use std::path::{Path, PathBuf};

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Checks that a manifest path is a safe, portable, relative path using `/` separators.
///
/// Manifest paths come from a remote server, so this is the only thing standing between a
/// malicious manifest and writes outside the managed folder.
pub fn validate_rel_path(p: &str) -> Result<()> {
    let bad = |why: &'static str| Err(Error::UnsafePath(p.to_string(), why));
    if p.is_empty() {
        return bad("empty");
    }
    if p.len() > 1024 {
        return bad("too long");
    }
    if p.starts_with('/') {
        return bad("absolute");
    }
    if p.contains('\\') {
        return bad("backslash");
    }
    if p.chars().any(|c| c.is_control()) {
        return bad("control character");
    }
    if p.chars()
        .any(|c| matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return bad("character not portable");
    }
    for comp in p.split('/') {
        if comp.is_empty() {
            return bad("empty component");
        }
        if comp == "." || comp == ".." {
            return bad("relative component");
        }
        if comp.ends_with('.') || comp.ends_with(' ') {
            return bad("trailing dot or space");
        }
        if comp.to_ascii_lowercase().starts_with(".spunkpipe") {
            return bad("reserved name");
        }
        let stem = comp.split('.').next().unwrap_or("").to_ascii_uppercase();
        if RESERVED.contains(&stem.as_str()) {
            return bad("reserved name");
        }
    }
    Ok(())
}

/// Joins a validated relative path onto `base`.
pub fn join_safe(base: &Path, rel: &str) -> Result<PathBuf> {
    validate_rel_path(rel)?;
    let mut out = base.to_path_buf();
    for comp in rel.split('/') {
        out.push(comp);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_paths() {
        for p in ["a", "CurryMod/ModInfo.xml", "Mods/x y/z.dll", "a.b/c.d.e"] {
            validate_rel_path(p).unwrap_or_else(|e| panic!("{p}: {e}"));
        }
    }

    #[test]
    fn rejects_dangerous_paths() {
        for p in [
            "", "/etc/passwd", "../x", "a/../b", "a//b", "./a", "a/./b", "a\\b", "C:/x", "a/b/",
            "con", "NUL.txt", "a/aux.dll", "x.", "x ", "a\0b", ".spunkpipe-staging/x", "a/*",
        ] {
            assert!(validate_rel_path(p).is_err(), "should reject {p:?}");
        }
    }
}
