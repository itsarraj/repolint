//! Real OSS-hygiene checks over a repo directory: does a `LICENSE` file
//! exist and does its *actual text* match the license the manifest
//! (`Cargo.toml` or `package.json`) declares; does `README.md` have
//! recognizable Install and Usage sections; is there a CI config at all.
//!
//! Nothing here talks to a registry or the network — every check is a
//! read of a handful of files already sitting in the directory you point
//! it at.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The three states a single check can land in. `Warn` is not a failure —
/// it means "couldn't verify this one way or the other" (e.g. no manifest
/// found to compare the license against), which is a materially different
/// thing to report than "verified, and it's wrong."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Fail,
    Warn,
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub name: &'static str,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct AuditReport {
    pub root: PathBuf,
    pub checks: Vec<CheckResult>,
}

impl AuditReport {
    /// `true` if nothing came back `Fail`. `Warn`s don't block — they're
    /// surfaced so the caller knows something couldn't be checked, not so
    /// they fail CI over it.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.status != CheckStatus::Fail)
    }

    pub fn fail_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == CheckStatus::Fail)
            .count()
    }
}

// ---------------------------------------------------------------------
// License text detection
// ---------------------------------------------------------------------

/// Recognizes a fixed set of common OSS license texts by the distinctive
/// wording every real copy of that license carries, checked in an order
/// that resolves the real overlaps (e.g. BSD-3-Clause's text is
/// BSD-2-Clause's plus one more paragraph, so BSD-3 must be checked
/// first). Returns `None` for anything outside that fixed set — CC0,
/// WTFPL, a custom license, or just a LICENSE file that doesn't contain
/// license text at all — rather than guessing.
pub fn detect_license_type(text: &str) -> Option<&'static str> {
    // Real LICENSE files hard-wrap prose at ~79 columns, which routinely
    // splits the exact phrases below across a line break (this bit
    // dnscheck's own real MIT LICENSE in testing: "...obtaining a copy\nof
    // this software..."). Matching against a whitespace-collapsed copy
    // makes every check below robust to wrap width instead of relying on
    // a phrase never crossing a newline.
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = normalized.as_str();

    if text.contains("Mozilla Public License") && text.contains("Version 2.0") {
        return Some("MPL-2.0");
    }
    if text.contains("Apache License") && text.contains("Version 2.0") {
        return Some("Apache-2.0");
    }
    if text.contains("GNU GENERAL PUBLIC LICENSE") {
        if text.contains("Version 2") {
            return Some("GPL-2.0");
        }
        return Some("GPL-3.0");
    }
    if text.contains("GNU LESSER GENERAL PUBLIC LICENSE") {
        if text.contains("Version 2.1") {
            return Some("LGPL-2.1");
        }
        return Some("LGPL-3.0");
    }
    if text.contains("This is free and unencumbered software released into the public domain") {
        return Some("Unlicense");
    }
    if text.contains("Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files")
    {
        return Some("MIT");
    }
    if text.contains("Redistribution and use in source and binary forms") {
        if text.contains("Neither the name") {
            return Some("BSD-3-Clause");
        }
        return Some("BSD-2-Clause");
    }
    if text.contains(
        "Permission to use, copy, modify, and/or distribute this software for any purpose",
    ) {
        return Some("ISC");
    }
    None
}

/// Normalizes an SPDX-ish id for comparison: uppercase, and the
/// `-only`/`-or-later` GPL-family suffixes stripped so `GPL-3.0-only`
/// compares equal to the plain `GPL-3.0` this tool's detector produces.
fn normalize_license_id(id: &str) -> String {
    let upper = id.trim().to_uppercase();
    upper
        .trim_end_matches("-OR-LATER")
        .trim_end_matches("-ONLY")
        .to_string()
}

/// Splits a license *expression* (`"MIT OR Apache-2.0"`, Cargo's legacy
/// `"MIT/Apache-2.0"`, a bare `"MIT"`) into its individual candidate ids.
/// This is deliberately not a real SPDX expression parser (see the
/// README) — it can't tell `AND` from `OR`, so `MIT AND Apache-2.0`
/// (both required) is treated the same as `MIT OR Apache-2.0` (either
/// one satisfies it). For the common case of a single license or a dual
/// permissive `OR`, that distinction doesn't matter; it would for a
/// license that combines a permissive grant with a required additional
/// restriction.
fn declared_candidates(expr: &str) -> Vec<String> {
    expr.split(['/', ','])
        .flat_map(|s| s.split(" OR ").flat_map(|s| s.split(" AND ")))
        .map(|s| normalize_license_id(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn declared_matches_detected(declared: &str, detected: &str) -> bool {
    let detected_norm = normalize_license_id(detected);
    declared_candidates(declared)
        .iter()
        .any(|c| c == &detected_norm)
}

// ---------------------------------------------------------------------
// Manifest license field
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CargoManifest {
    package: Option<CargoPackage>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    license: Option<String>,
}

/// Reads the `license` field out of `Cargo.toml`, if present.
pub fn read_cargo_license(dir: &Path) -> Option<String> {
    let path = dir.join("Cargo.toml");
    let content = fs::read_to_string(path).ok()?;
    let manifest: CargoManifest = toml::from_str(&content).ok()?;
    manifest.package?.license
}

/// Reads the `license` field out of `package.json`, if present. Handles
/// both the modern SPDX-string form (`"license": "MIT"`) and the older
/// object form (`"license": {"type": "MIT"}`) npm still accepts.
pub fn read_package_json_license(dir: &Path) -> Option<String> {
    let path = dir.join("package.json");
    let content = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    match value.get("license")? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(o) => o.get("type")?.as_str().map(|s| s.to_string()),
        _ => None,
    }
}

/// Looks for a declared license in `Cargo.toml` first, then
/// `package.json`, returning `(source_file, declared_value)`.
pub fn find_declared_license(dir: &Path) -> Option<(&'static str, String)> {
    if let Some(l) = read_cargo_license(dir) {
        return Some(("Cargo.toml", l));
    }
    if let Some(l) = read_package_json_license(dir) {
        return Some(("package.json", l));
    }
    None
}

// ---------------------------------------------------------------------
// LICENSE file
// ---------------------------------------------------------------------

const LICENSE_FILENAMES: &[&str] = &["LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING"];

/// Finds a LICENSE-shaped file at the repo root, trying the common name
/// variants in order.
pub fn find_license_file(dir: &Path) -> Option<PathBuf> {
    LICENSE_FILENAMES
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

// ---------------------------------------------------------------------
// README sections
// ---------------------------------------------------------------------

const README_FILENAMES: &[&str] = &["README.md", "README.MD", "Readme.md", "README"];

pub fn find_readme(dir: &Path) -> Option<PathBuf> {
    README_FILENAMES
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

/// A line counts as a Markdown heading for this purpose if it starts
/// with one or more `#`. ATX-style only (`## Usage`) — the older
/// underline style (`Usage\n-----`) isn't recognized, which is a
/// deliberate simplification, not an oversight (see README).
fn heading_text(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    Some(trimmed.trim_start_matches('#').trim().to_lowercase())
}

fn has_heading_matching(content: &str, keywords: &[&str]) -> bool {
    content
        .lines()
        .filter_map(heading_text)
        .any(|h| keywords.iter().any(|k| h.contains(k)))
}

pub fn readme_has_install_section(content: &str) -> bool {
    has_heading_matching(content, &["install", "getting started", "quick start"])
}

pub fn readme_has_usage_section(content: &str) -> bool {
    has_heading_matching(
        content,
        &["usage", "getting started", "quick start", "example"],
    )
}

// ---------------------------------------------------------------------
// CI config
// ---------------------------------------------------------------------

/// True if a `.github/workflows/*.yml`/`*.yaml` file exists, or a
/// `.gitlab-ci.yml` sits at the repo root.
pub fn has_ci_config(dir: &Path) -> Option<PathBuf> {
    let gitlab = dir.join(".gitlab-ci.yml");
    if gitlab.is_file() {
        return Some(gitlab);
    }
    let workflows = dir.join(".github").join("workflows");
    if let Ok(entries) = fs::read_dir(&workflows) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext == "yml" || ext == "yaml" {
                    return Some(path);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------
// Full audit
// ---------------------------------------------------------------------

pub fn audit_repo(dir: &Path) -> AuditReport {
    let mut checks = Vec::new();

    // 1. LICENSE presence + type match against the manifest.
    match find_license_file(dir) {
        None => {
            checks.push(CheckResult {
                name: "license_present",
                status: CheckStatus::Fail,
                detail: format!(
                    "no LICENSE file found (looked for {})",
                    LICENSE_FILENAMES.join(", ")
                ),
            });
        }
        Some(license_path) => {
            checks.push(CheckResult {
                name: "license_present",
                status: CheckStatus::Pass,
                detail: format!(
                    "found {}",
                    license_path
                        .strip_prefix(dir)
                        .unwrap_or(&license_path)
                        .display()
                ),
            });

            let text = fs::read_to_string(&license_path).unwrap_or_default();
            let detected = detect_license_type(&text);

            match (find_declared_license(dir), detected) {
                (None, _) => {
                    checks.push(CheckResult {
                        name: "license_matches_manifest",
                        status: CheckStatus::Warn,
                        detail: "no `license` field found in Cargo.toml or package.json to compare against".to_string(),
                    });
                }
                (Some((source, declared)), None) => {
                    checks.push(CheckResult {
                        name: "license_matches_manifest",
                        status: CheckStatus::Warn,
                        detail: format!(
                            "{source} declares `{declared}` but the LICENSE file's text isn't one of the recognized license texts, so it can't be confirmed either way"
                        ),
                    });
                }
                (Some((source, declared)), Some(detected)) => {
                    if declared_matches_detected(&declared, detected) {
                        checks.push(CheckResult {
                            name: "license_matches_manifest",
                            status: CheckStatus::Pass,
                            detail: format!("{source} declares `{declared}`, LICENSE file text is {detected} — match"),
                        });
                    } else {
                        checks.push(CheckResult {
                            name: "license_matches_manifest",
                            status: CheckStatus::Fail,
                            detail: format!(
                                "{source} declares `{declared}` but the LICENSE file's actual text is {detected} — mismatch"
                            ),
                        });
                    }
                }
            }
        }
    }

    // 2. README presence + sections.
    match find_readme(dir) {
        None => {
            checks.push(CheckResult {
                name: "readme_present",
                status: CheckStatus::Fail,
                detail: "no README.md found".to_string(),
            });
        }
        Some(readme_path) => {
            checks.push(CheckResult {
                name: "readme_present",
                status: CheckStatus::Pass,
                detail: format!(
                    "found {}",
                    readme_path
                        .strip_prefix(dir)
                        .unwrap_or(&readme_path)
                        .display()
                ),
            });

            let content = fs::read_to_string(&readme_path).unwrap_or_default();
            let has_install = readme_has_install_section(&content);
            let has_usage = readme_has_usage_section(&content);

            let status = if has_install && has_usage {
                CheckStatus::Pass
            } else {
                CheckStatus::Fail
            };
            let detail = match (has_install, has_usage) {
                (true, true) => "found an Install and a Usage section".to_string(),
                (true, false) => {
                    "found an Install section but no recognizable Usage section".to_string()
                }
                (false, true) => {
                    "found a Usage section but no recognizable Install section".to_string()
                }
                (false, false) => {
                    "no recognizable Install or Usage section (looked for `#`-style headings)"
                        .to_string()
                }
            };
            checks.push(CheckResult {
                name: "readme_has_sections",
                status,
                detail,
            });
        }
    }

    // 3. CI config.
    match has_ci_config(dir) {
        Some(path) => checks.push(CheckResult {
            name: "ci_config_present",
            status: CheckStatus::Pass,
            detail: format!(
                "found {}",
                path.strip_prefix(dir).unwrap_or(&path).display()
            ),
        }),
        None => checks.push(CheckResult {
            name: "ci_config_present",
            status: CheckStatus::Fail,
            detail: "no .github/workflows/*.yml and no .gitlab-ci.yml found".to_string(),
        }),
    }

    AuditReport {
        root: dir.to_path_buf(),
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir(name: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "repolint-test-{}-{}-{}",
            std::process::id(),
            name,
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MIT_TEXT: &str = "MIT License\n\nCopyright (c) 2026 Someone\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software and associated documentation files (the \"Software\"), to deal\nin the Software without restriction...\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\nIMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY.\n";

    const APACHE_TEXT: &str = "                                 Apache License\n                           Version 2.0, January 2004\n                        http://www.apache.org/licenses/\n\n   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION\n";

    const GPL3_TEXT: &str = "                    GNU GENERAL PUBLIC LICENSE\n                       Version 3, 29 June 2007\n\n Copyright (C) 2007 Free Software Foundation, Inc. <https://fsf.org/>\n";

    const BSD3_TEXT: &str = "Redistribution and use in source and binary forms, with or without\nmodification, are permitted provided that the following conditions are met:\n\n3. Neither the name of the copyright holder nor the names of its\ncontributors may be used to endorse or promote products derived from\nthis software without specific prior written permission.\n";

    const BSD2_TEXT: &str = "Redistribution and use in source and binary forms, with or without\nmodification, are permitted provided that the following conditions are met:\n\n1. Redistributions of source code must retain the above copyright notice.\n2. Redistributions in binary form must reproduce the above copyright notice.\n";

    const ISC_TEXT: &str = "Permission to use, copy, modify, and/or distribute this software for any\npurpose with or without fee is hereby granted, provided that the above\ncopyright notice and this permission notice appear in all copies.\n";

    const UNLICENSE_TEXT: &str =
        "This is free and unencumbered software released into the public domain.\n";

    #[test]
    fn detects_mit() {
        assert_eq!(detect_license_type(MIT_TEXT), Some("MIT"));
    }

    #[test]
    fn detects_apache_2() {
        assert_eq!(detect_license_type(APACHE_TEXT), Some("Apache-2.0"));
    }

    #[test]
    fn detects_gpl3_not_gpl2() {
        assert_eq!(detect_license_type(GPL3_TEXT), Some("GPL-3.0"));
    }

    #[test]
    fn detects_bsd3_over_bsd2_when_endorsement_clause_present() {
        assert_eq!(detect_license_type(BSD3_TEXT), Some("BSD-3-Clause"));
        assert_eq!(detect_license_type(BSD2_TEXT), Some("BSD-2-Clause"));
    }

    #[test]
    fn detects_isc() {
        assert_eq!(detect_license_type(ISC_TEXT), Some("ISC"));
    }

    #[test]
    fn detects_unlicense() {
        assert_eq!(detect_license_type(UNLICENSE_TEXT), Some("Unlicense"));
    }

    #[test]
    fn unrecognized_text_returns_none() {
        assert_eq!(
            detect_license_type("Do whatever you want with this.\n"),
            None
        );
    }

    #[test]
    fn declared_matches_detected_handles_plain_id() {
        assert!(declared_matches_detected("MIT", "MIT"));
        assert!(!declared_matches_detected("MIT", "Apache-2.0"));
    }

    #[test]
    fn declared_matches_detected_handles_or_expression() {
        assert!(declared_matches_detected("MIT OR Apache-2.0", "Apache-2.0"));
        assert!(declared_matches_detected("MIT OR Apache-2.0", "MIT"));
        assert!(!declared_matches_detected("MIT OR Apache-2.0", "GPL-3.0"));
    }

    #[test]
    fn declared_matches_detected_handles_legacy_slash_form() {
        assert!(declared_matches_detected("MIT/Apache-2.0", "MIT"));
    }

    #[test]
    fn declared_matches_detected_normalizes_only_suffix() {
        assert!(declared_matches_detected("GPL-3.0-only", "GPL-3.0"));
        assert!(declared_matches_detected("GPL-3.0-or-later", "GPL-3.0"));
    }

    #[test]
    fn declared_matches_detected_is_case_insensitive() {
        assert!(declared_matches_detected("mit", "MIT"));
    }

    #[test]
    fn readme_sections_found_when_present() {
        let content =
            "# my-tool\n\n## Install\n\ncargo install my-tool\n\n## Usage\n\nmy-tool run\n";
        assert!(readme_has_install_section(content));
        assert!(readme_has_usage_section(content));
    }

    #[test]
    fn readme_sections_missing_when_absent() {
        let content = "# my-tool\n\nA cool tool.\n";
        assert!(!readme_has_install_section(content));
        assert!(!readme_has_usage_section(content));
    }

    #[test]
    fn readme_recognizes_installation_and_getting_started_variants() {
        let content = "# tool\n\n## Installation\n\nsteps\n\n## Getting Started\n\nsteps\n";
        assert!(readme_has_install_section(content));
        assert!(readme_has_usage_section(content));
    }

    #[test]
    fn read_cargo_license_parses_license_field() {
        let dir = scratch_dir("cargo-license");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n",
        )
        .unwrap();
        assert_eq!(read_cargo_license(&dir), Some("MIT".to_string()));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_package_json_license_handles_string_and_object_forms() {
        let dir = scratch_dir("pkg-json-string");
        fs::write(
            dir.join("package.json"),
            r#"{"name": "x", "license": "MIT"}"#,
        )
        .unwrap();
        assert_eq!(read_package_json_license(&dir), Some("MIT".to_string()));
        fs::remove_dir_all(&dir).ok();

        let dir2 = scratch_dir("pkg-json-object");
        fs::write(
            dir2.join("package.json"),
            r#"{"name": "x", "license": {"type": "Apache-2.0"}}"#,
        )
        .unwrap();
        assert_eq!(
            read_package_json_license(&dir2),
            Some("Apache-2.0".to_string())
        );
        fs::remove_dir_all(&dir2).ok();
    }

    #[test]
    fn audit_repo_flags_missing_license() {
        let dir = scratch_dir("missing-license");
        fs::write(
            dir.join("README.md"),
            "# x\n\n## Install\n\nx\n\n## Usage\n\nx\n",
        )
        .unwrap();
        let report = audit_repo(&dir);
        assert!(!report.passed());
        let license_check = report
            .checks
            .iter()
            .find(|c| c.name == "license_present")
            .unwrap();
        assert_eq!(license_check.status, CheckStatus::Fail);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn audit_repo_flags_license_mismatch() {
        let dir = scratch_dir("license-mismatch");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n",
        )
        .unwrap();
        fs::write(dir.join("LICENSE"), APACHE_TEXT).unwrap();
        fs::write(
            dir.join("README.md"),
            "# x\n\n## Install\n\nx\n\n## Usage\n\nx\n",
        )
        .unwrap();
        let report = audit_repo(&dir);
        assert!(!report.passed());
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "license_matches_manifest")
            .unwrap();
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.detail.contains("Apache-2.0"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn audit_repo_passes_a_fully_compliant_repo() {
        let dir = scratch_dir("compliant");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n",
        )
        .unwrap();
        fs::write(dir.join("LICENSE"), MIT_TEXT).unwrap();
        fs::write(
            dir.join("README.md"),
            "# x\n\n## Install\n\nx\n\n## Usage\n\nx\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join(".github").join("workflows")).unwrap();
        fs::write(
            dir.join(".github").join("workflows").join("ci.yml"),
            "name: CI\n",
        )
        .unwrap();
        let report = audit_repo(&dir);
        assert!(
            report.passed(),
            "expected a fully compliant repo to pass: {:?}",
            report.checks
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn has_ci_config_finds_gitlab_ci() {
        let dir = scratch_dir("gitlab-ci");
        fs::write(dir.join(".gitlab-ci.yml"), "stages: []\n").unwrap();
        assert!(has_ci_config(&dir).is_some());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn has_ci_config_none_when_absent() {
        let dir = scratch_dir("no-ci");
        assert!(has_ci_config(&dir).is_none());
        fs::remove_dir_all(&dir).ok();
    }
}
