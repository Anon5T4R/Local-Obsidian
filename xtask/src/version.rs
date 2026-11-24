//! `cargo xtask version [set X.Y.Z[-pre]]`: the workspace version, the single source of truth.
//!
//! The version lives in `[workspace.package] version` in the root `Cargo.toml`; every crate
//! inherits it with `version.workspace = true`, and the packaging scripts read it from here.
//! The Android app ships on the same tag, so `set` also writes `versionName` in
//! `android/app/build.gradle.kts` (the same X.Y.Z) and raises its `versionCode` by one.

use std::path::Path;

/// The `version = "…"` value inside `[workspace.package]`.
pub fn read(manifest: &str) -> Result<String, String> {
    let (_, line) = find_line(manifest)?;
    let v = line.split_once('=').map(|(_, v)| v.trim().trim_matches('"').to_string()).unwrap_or_default();
    if v.is_empty() {
        return Err("empty [workspace.package] version".into());
    }
    Ok(v)
}

/// `manifest` with the workspace version replaced; everything else is byte-for-byte unchanged.
pub fn replace(manifest: &str, new: &str) -> Result<String, String> {
    validate(new)?;
    let (idx, _) = find_line(manifest)?;
    let mut out = String::with_capacity(manifest.len() + 8);
    for (i, line) in manifest.split_inclusive('\n').enumerate() {
        if i == idx {
            let eol = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            out.push_str(&format!("version = \"{new}\"{eol}"));
        } else {
            out.push_str(line);
        }
    }
    Ok(out)
}

/// Semver without build metadata: `MAJOR.MINOR.PATCH` plus an optional `-pre.release` tag.
pub fn validate(v: &str) -> Result<(), String> {
    let bad = || Err(format!("`{v}` is not a version like 1.2.3 or 1.2.3-rc.1"));
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let nums: Vec<&str> = core.split('.').collect();
    if nums.len() != 3 || nums.iter().any(|n| n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) || (n.len() > 1 && n.starts_with('0'))) {
        return bad();
    }
    if let Some(p) = pre
        && (p.is_empty() || p.split('.').any(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')))
    {
        return bad();
    }
    Ok(())
}

/// The Android app's Gradle file, relative to the repo root.
const GRADLE: &str = "android/app/build.gradle.kts";

/// The `versionName = "…"` and `versionCode = N` values of the Android Gradle file.
pub fn read_android(gradle: &str) -> Result<(String, u32), String> {
    let (_, name_line) = find_gradle_line(gradle, "versionName")?;
    let (_, code_line) = find_gradle_line(gradle, "versionCode")?;
    let name = value_of(name_line).trim_matches('"').to_string();
    let code = value_of(code_line).parse::<u32>().map_err(|_| format!("`{}` is not a number", value_of(code_line)))?;
    Ok((name, code))
}

/// `gradle` with `versionName` set to `new` and `versionCode` one higher. Setting the version the
/// file already has changes nothing (re-running `set` must not burn version codes). Everything
/// else, line endings included, is byte-for-byte unchanged.
pub fn replace_android(gradle: &str, new: &str) -> Result<String, String> {
    validate(new)?;
    let (name, code) = read_android(gradle)?;
    if name == new {
        return Ok(gradle.to_string());
    }
    let next = code.checked_add(1).ok_or("versionCode overflow")?;
    let (name_idx, _) = find_gradle_line(gradle, "versionName")?;
    let (code_idx, _) = find_gradle_line(gradle, "versionCode")?;
    let mut out = String::with_capacity(gradle.len() + 8);
    for (i, line) in gradle.split_inclusive('\n').enumerate() {
        let replacement = if i == name_idx {
            Some(format!("versionName = \"{new}\""))
        } else if i == code_idx {
            Some(format!("versionCode = {next}"))
        } else {
            None
        };
        match replacement {
            Some(text) => {
                let indent = &line[..line.len() - line.trim_start().len()];
                let eol = &line[line.trim_end_matches(['\r', '\n']).len()..];
                out.push_str(&format!("{indent}{text}{eol}"));
            }
            None => out.push_str(line),
        }
    }
    Ok(out)
}

/// The text after `=` on a `key = value` line.
fn value_of(line: &str) -> &str {
    line.split_once('=').map(|(_, v)| v.trim()).unwrap_or_default()
}

/// Index and text of the line `key = …` (indented or not) in the Gradle file.
fn find_gradle_line<'a>(gradle: &'a str, key: &str) -> Result<(usize, &'a str), String> {
    for (i, line) in gradle.lines().enumerate() {
        if let Some(rest) = line.trim().strip_prefix(key)
            && rest.trim_start().starts_with('=')
        {
            return Ok((i, line));
        }
    }
    Err(format!("no `{key} = …` in {GRADLE}"))
}

/// Index and text of the `version` line in `[workspace.package]`.
fn find_line(manifest: &str) -> Result<(usize, &str), String> {
    let mut in_section = false;
    for (i, line) in manifest.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            in_section = t == "[workspace.package]";
            continue;
        }
        if in_section
            && let Some(rest) = t.strip_prefix("version")
            && rest.trim_start().starts_with('=')
        {
            return Ok((i, line));
        }
    }
    Err("no `version = \"…\"` in [workspace.package] of the root Cargo.toml".into())
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let path = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    match args {
        [] => {
            println!("{}", read(&text)?);
            Ok(())
        }
        ["set", new] => {
            let old = read(&text)?;
            let updated = replace(&text, new)?;
            let gradle_path = root.join(GRADLE);
            let gradle = std::fs::read_to_string(&gradle_path).map_err(|e| format!("read {}: {e}", gradle_path.display()))?;
            // Both files are computed before either is written: a bad Gradle file must not leave
            // the desktop bumped and the Android app behind.
            let gradle_updated = replace_android(&gradle, new)?;
            // Write atomically: a half-written root manifest breaks every build in the workspace.
            let tmp = root.join("target").join("Cargo.toml.xtask-version");
            std::fs::create_dir_all(root.join("target")).map_err(|e| format!("create target/: {e}"))?;
            std::fs::write(&tmp, updated).map_err(|e| format!("write {}: {e}", tmp.display()))?;
            std::fs::rename(&tmp, &path).map_err(|e| format!("replace {}: {e}", path.display()))?;
            if gradle_updated != gradle {
                std::fs::write(&gradle_path, &gradle_updated).map_err(|e| format!("write {}: {e}", gradle_path.display()))?;
            }
            let (_, code) = read_android(&gradle_updated)?;
            println!("android: versionName {new}, versionCode {code}");
            // Refresh the workspace members' entries in Cargo.lock (no dependency upgrades).
            let mut c = crate::cargo();
            c.args(["update", "--workspace", "--offline"]);
            if crate::run(c, "cargo update --workspace --offline").is_err() {
                let mut c = crate::cargo();
                c.args(["update", "--workspace"]);
                crate::run(c, "cargo update --workspace")?;
            }
            println!("version: {old} -> {new}");
            Ok(())
        }
        _ => Err("usage: cargo xtask version [set X.Y.Z[-pre]]".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "[workspace]\nmembers = [\"a\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace.dependencies]\nfoo = { version = \"1\" }\n";

    #[test]
    fn reads_the_workspace_version() {
        assert_eq!(read(MANIFEST).unwrap(), "0.1.0");
        assert!(read("[package]\nversion = \"1.0.0\"\n").is_err());
    }

    #[test]
    fn replaces_only_the_workspace_version() {
        let out = replace(MANIFEST, "1.2.3-rc.1").unwrap();
        assert_eq!(read(&out).unwrap(), "1.2.3-rc.1");
        assert_eq!(out, MANIFEST.replace("version = \"0.1.0\"", "version = \"1.2.3-rc.1\""));
        assert!(out.contains("foo = { version = \"1\" }"));
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let crlf = MANIFEST.replace('\n', "\r\n");
        let out = replace(&crlf, "2.0.0").unwrap();
        assert_eq!(out, crlf.replace("\"0.1.0\"", "\"2.0.0\""));
    }

    #[test]
    fn validates_versions() {
        for ok in ["0.1.0", "1.20.300", "1.0.0-rc.1", "1.0.0-alpha", "1.0.0-x-y.2"] {
            assert!(validate(ok).is_ok(), "{ok}");
        }
        for bad in ["1.0", "1.0.0.0", "v1.0.0", "01.0.0", "1.0.0-", "1.0.0-a..b", "1.0.0+meta", "1.a.0", ""] {
            assert!(validate(bad).is_err(), "{bad}");
        }
        assert!(replace(MANIFEST, "nope").is_err());
    }

    const GRADLE_FILE: &str = "android {\n    defaultConfig {\n        applicationId = \"x\"\n        versionCode = 22\n        versionName = \"3.6.0\"\n\n        minSdk = 28\n    }\n}\n";

    #[test]
    fn reads_the_android_version() {
        assert_eq!(read_android(GRADLE_FILE).unwrap(), ("3.6.0".to_string(), 22));
        assert!(read_android("android {}\n").is_err());
    }

    #[test]
    fn sets_the_android_version_name_and_raises_the_code_by_one() {
        let out = replace_android(GRADLE_FILE, "1.10.0").unwrap();
        assert_eq!(read_android(&out).unwrap(), ("1.10.0".to_string(), 23));
        assert_eq!(out, GRADLE_FILE.replace("= 22", "= 23").replace("\"3.6.0\"", "\"1.10.0\""));
    }

    #[test]
    fn setting_the_same_android_version_again_keeps_the_code() {
        let once = replace_android(GRADLE_FILE, "1.10.0").unwrap();
        assert_eq!(replace_android(&once, "1.10.0").unwrap(), once);
        let twice = replace_android(&once, "1.10.1").unwrap();
        assert_eq!(read_android(&twice).unwrap(), ("1.10.1".to_string(), 24));
    }

    #[test]
    fn android_edit_keeps_crlf_and_rejects_a_bad_version() {
        let crlf = GRADLE_FILE.replace('\n', "\r\n");
        let out = replace_android(&crlf, "2.0.0").unwrap();
        assert_eq!(out, crlf.replace("= 22", "= 23").replace("\"3.6.0\"", "\"2.0.0\""));
        assert!(replace_android(GRADLE_FILE, "v2").is_err());
    }

    #[test]
    fn the_real_gradle_file_matches_the_desktop_version() {
        let root = crate::root();
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        let gradle = std::fs::read_to_string(root.join(GRADLE)).unwrap();
        let (name, code) = read_android(&gradle).unwrap();
        assert_eq!(name, read(&manifest).unwrap(), "android versionName must equal the workspace version (cargo xtask version set X.Y.Z)");
        assert!(code > 0);
    }

    #[test]
    fn the_real_manifest_has_a_valid_version() {
        let text = std::fs::read_to_string(crate::root().join("Cargo.toml")).unwrap();
        validate(&read(&text).unwrap()).unwrap();
    }
}
