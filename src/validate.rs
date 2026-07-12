//! `validate` subcommand: schema, slug uniqueness, anchor drift.
//!
//! Pure read-only — never mutates the source tree. Collects all
//! issues into a `ValidationReport` instead of bailing on the first
//! parse error, so a single run surfaces every problem.

use crate::{
    feature_md_paths, load_config, parse_feature, render, sort_features, Config, Frontmatter,
};
use anyhow::{bail, Context, Result};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct ValidationReport {
    /// `.roadmap/` source tree is absent on this checkout (e.g. CI, or a
    /// worktree where the source lives elsewhere). Skipped, not failed.
    pub source_missing: bool,
    pub schema_errors: Vec<SchemaError>,
    pub duplicate_ids: Vec<String>,
    pub anchor_collisions: Vec<AnchorCollision>,
    /// Anchors present in `ROADMAP.md` but absent from a fresh regen
    /// — inbound links to the roadmap would 404 after the next regen.
    pub anchors_missing_from_regen: Vec<String>,
    /// Anchors present in regen but absent from on-disk `ROADMAP.md`
    /// — release-prep regen never ran (or wasn't committed).
    pub anchors_missing_from_disk: Vec<String>,
}

#[derive(Debug)]
pub struct SchemaError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug)]
pub struct AnchorCollision {
    pub anchor: String,
    pub ids: Vec<String>,
}

impl ValidationReport {
    pub fn is_clean(&self) -> bool {
        self.source_missing
            || (self.schema_errors.is_empty()
                && self.duplicate_ids.is_empty()
                && self.anchor_collisions.is_empty()
                && !self.has_drift())
    }

    pub fn has_drift(&self) -> bool {
        !self.anchors_missing_from_regen.is_empty() || !self.anchors_missing_from_disk.is_empty()
    }

    pub fn has_hard_errors(&self) -> bool {
        !self.schema_errors.is_empty()
            || !self.duplicate_ids.is_empty()
            || !self.anchor_collisions.is_empty()
    }

    pub fn to_text(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        if self.source_missing {
            out.push_str("validate: skipped (no `.roadmap/` source on this checkout)\n");
            return out;
        }
        if self.is_clean() {
            out.push_str("validate: clean\n");
            return out;
        }
        if !self.schema_errors.is_empty() {
            let _ = writeln!(out, "schema errors ({}):", self.schema_errors.len());
            for e in &self.schema_errors {
                let _ = writeln!(out, "  {}: {}", e.path.display(), e.message);
            }
        }
        if !self.duplicate_ids.is_empty() {
            let _ = writeln!(out, "duplicate ids ({}):", self.duplicate_ids.len());
            for id in &self.duplicate_ids {
                let _ = writeln!(out, "  {id}");
            }
        }
        if !self.anchor_collisions.is_empty() {
            let _ = writeln!(out, "anchor collisions ({}):", self.anchor_collisions.len());
            for c in &self.anchor_collisions {
                let _ = writeln!(out, "  anchor `{}` ← ids {:?}", c.anchor, c.ids);
            }
        }
        if !self.anchors_missing_from_regen.is_empty() {
            let _ = writeln!(
                out,
                "anchors on disk but not in regen ({}) — broken inbound links after regen:",
                self.anchors_missing_from_regen.len()
            );
            for a in &self.anchors_missing_from_regen {
                let _ = writeln!(out, "  {a}");
            }
        }
        if !self.anchors_missing_from_disk.is_empty() {
            let _ = writeln!(
                out,
                "anchors in regen but not on disk ({}) — `ROADMAP.md` needs regen:",
                self.anchors_missing_from_disk.len()
            );
            for a in &self.anchors_missing_from_disk {
                let _ = writeln!(out, "  {a}");
            }
        }
        out
    }
}

pub fn validate(root: &Path, roadmap_md: &Path) -> Result<ValidationReport> {
    let mut report = ValidationReport::default();

    let features_dir = root.join("features");
    if !features_dir.is_dir() {
        // No source on this checkout — silent-pass. Lets the recipe
        // run on checkouts where `.roadmap/` is absent (e.g. CI)
        // without manufacturing an error.
        report.source_missing = true;
        return Ok(report);
    }

    let config = load_config(root).context("loading config.toml")?;
    check_config_fields(&root.join("config.toml"), &config, &mut report);

    let mut features = Vec::new();
    for path in feature_md_paths(&features_dir)? {
        match std::fs::read_to_string(&path) {
            Ok(src) => match parse_feature(&src) {
                Ok(f) => {
                    check_feature_fields(&path, &f.frontmatter, &config, &mut report);
                    features.push(f);
                },
                Err(e) => report.schema_errors.push(SchemaError {
                    path: path.clone(),
                    message: format!("{e:#}"),
                }),
            },
            Err(e) => report.schema_errors.push(SchemaError {
                path: path.clone(),
                message: format!("read failed: {e}"),
            }),
        }
    }

    let mut id_counts: HashMap<String, usize> = HashMap::new();
    for f in &features {
        *id_counts.entry(f.frontmatter.id.clone()).or_default() += 1;
    }
    for (id, n) in &id_counts {
        if *n > 1 {
            report.duplicate_ids.push(id.clone());
        }
    }
    report.duplicate_ids.sort();

    let mut anchor_to_ids: HashMap<String, BTreeSet<String>> = HashMap::new();
    for f in &features {
        anchor_to_ids
            .entry(f.frontmatter.id.to_lowercase())
            .or_default()
            .insert(f.frontmatter.id.clone());
    }
    for (anchor, ids) in anchor_to_ids {
        if ids.len() > 1 {
            report.anchor_collisions.push(AnchorCollision {
                anchor,
                ids: ids.into_iter().collect(),
            });
        }
    }
    report
        .anchor_collisions
        .sort_by(|a, b| a.anchor.cmp(&b.anchor));

    if !roadmap_md.is_file() {
        bail!("ROADMAP.md not found at: {}", roadmap_md.display());
    }
    let on_disk = std::fs::read_to_string(roadmap_md)
        .with_context(|| format!("reading {}", roadmap_md.display()))?;
    let on_disk_anchors = extract_anchors(&on_disk);

    let mut sorted = features;
    sort_features(&mut sorted, &config);
    let regen = render(&sorted, &config);
    let regen_anchors = extract_anchors(&regen);

    report.anchors_missing_from_regen = on_disk_anchors
        .difference(&regen_anchors)
        .cloned()
        .collect();
    report.anchors_missing_from_disk = regen_anchors
        .difference(&on_disk_anchors)
        .cloned()
        .collect();

    Ok(report)
}

/// Config-driven per-feature schema checks: every declared field's value(s)
/// must be in its allow-list, `required_when` conditionals must hold, and
/// `area` must carry at least one value. One `SchemaError` per breach, in
/// stable (`BTreeMap`) field order so runs are reproducible.
fn check_feature_fields(
    path: &Path,
    fm: &Frontmatter,
    config: &Config,
    report: &mut ValidationReport,
) {
    let mut err = |message: String| {
        report.schema_errors.push(SchemaError {
            path: path.to_path_buf(),
            message,
        });
    };
    for (name, spec) in &config.fields {
        // `None` = a field the generator doesn't model. The config-level
        // typo is surfaced once by `check_config_fields`; skip it here.
        let Some(values) = fm.field_values(name) else {
            continue;
        };
        if let Some(required_when) = &spec.required_when {
            // ALL declared conditions must hold (AND) for the field to be
            // required — a condition matches when the referenced field
            // currently carries the expected value. Honours every key, not
            // just `type`.
            let all_match = required_when.iter().all(|(cond_field, cond_val)| {
                fm.field_values(cond_field)
                    .is_some_and(|vals| vals.iter().any(|v| v == cond_val))
            });
            if all_match && values.is_empty() {
                err(format!(
                    "`{name}` is required when {}",
                    describe_condition(required_when)
                ));
            }
        }
        if !spec.multi && values.len() > 1 {
            err(format!(
                "`{name}` accepts a single value but {} were given",
                values.len()
            ));
        }
        for v in &values {
            if !spec.values.iter().any(|allowed| allowed == v) {
                err(format!(
                    "unknown `{name}` value {v:?} (allowed: {})",
                    spec.values.join(", ")
                ));
            }
        }
    }
    if fm.area.is_empty() {
        err("`area` must list at least one value".to_string());
    }
}

/// Sorted, human-readable rendering of a `required_when` condition set, so
/// error messages are deterministic regardless of `HashMap` iteration order.
fn describe_condition(cond: &HashMap<String, String>) -> String {
    let mut parts: Vec<String> = cond.iter().map(|(k, v)| format!("{k} = \"{v}\"")).collect();
    parts.sort();
    parts.join(", ")
}

/// One-time config sanity, independent of any feature: reject a `[fields.*]`
/// name the generator doesn't model (a typo silently disables that field's
/// validation), and require `[fields.horizon]` — every feature carries a
/// `horizon` that drives sort rank, so omitting it silently degrades
/// within-tier ordering to id order.
fn check_config_fields(config_path: &Path, config: &Config, report: &mut ValidationReport) {
    for name in config.fields.keys() {
        if !Frontmatter::FIELD_NAMES.contains(&name.as_str()) {
            report.schema_errors.push(SchemaError {
                path: config_path.to_path_buf(),
                message: format!(
                    "unknown `[fields.{name}]` — not a recognized schema field (known: {})",
                    Frontmatter::FIELD_NAMES.join(", ")
                ),
            });
        }
    }
    if !config.fields.contains_key("horizon") {
        report.schema_errors.push(SchemaError {
            path: config_path.to_path_buf(),
            message: "missing `[fields.horizon]` — every feature carries a `horizon` that drives \
                      sort rank; declare its values (order = rank) or rows fall back to id order"
                .to_string(),
        });
    }
}

/// Extract the contents of every `<a id="…">` in markdown.
/// Manual scanner — the shape is fixed and narrow, doesn't justify a regex dep.
pub fn extract_anchors(md: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let needle = "<a id=\"";
    let mut rest = md;
    while let Some(start) = rest.find(needle) {
        let after = &rest[start + needle.len()..];
        match after.find('"') {
            Some(end) => {
                out.insert(after[..end].to_string());
                rest = &after[end + 1..];
            },
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_anchors_basic() {
        let md = r#"<a id="f22"></a> ... <a id="f-foo"></a> ..."#;
        let got = extract_anchors(md);
        let want: BTreeSet<String> = ["f22", "f-foo"].iter().map(|s| s.to_string()).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn extract_anchors_ignores_other_html() {
        let md = r##"<div id="x"></div> <a href="#y">z</a> <a id="ok"></a>"##;
        let got = extract_anchors(md);
        let want: BTreeSet<String> = ["ok"].iter().map(|s| s.to_string()).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn extract_anchors_unterminated_is_safe() {
        let md = r#"<a id="oops"#;
        assert!(extract_anchors(md).is_empty());
    }

    #[test]
    fn report_clean_when_empty() {
        let r = ValidationReport::default();
        assert!(r.is_clean());
        assert!(!r.has_drift());
        assert!(!r.has_hard_errors());
    }

    #[test]
    fn validate_skips_when_source_missing() {
        // Pointing `root` at any non-existent `features/` parent should
        // silent-pass — the recipe runs on source-less checkouts too.
        let tmp = std::env::temp_dir().join("roadmap-cli-skip-missing");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let roadmap_md = tmp.join("ROADMAP.md");
        std::fs::write(&roadmap_md, "").unwrap();
        let r = validate(&tmp, &roadmap_md).unwrap();
        assert!(r.source_missing);
        assert!(r.is_clean());
        assert!(r.to_text().contains("skipped"));
    }

    #[test]
    fn report_drift_only() {
        let mut r = ValidationReport::default();
        r.anchors_missing_from_disk.push("f-new".into());
        assert!(!r.is_clean());
        assert!(r.has_drift());
        assert!(!r.has_hard_errors());
    }

    fn fm(item_type: &str, class: Option<&str>, area: Vec<&str>, horizon: &str) -> Frontmatter {
        Frontmatter {
            id: "F-x".into(),
            item_type: item_type.into(),
            class: class.map(Into::into),
            effort: None,
            area: area.into_iter().map(Into::into).collect(),
            horizon: horizon.into(),
            status: crate::Status::Todo,
            target: vec!["v0.2.x".into()],
            severity: None,
            shipped: crate::Shipped::default(),
            shipped_order: None,
        }
    }

    fn cfg_with_fields() -> Config {
        use crate::FieldSpec;
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "class".to_string(),
            FieldSpec {
                values: vec!["differentiator".into(), "enabler".into()],
                multi: false,
                required_when: Some(std::collections::HashMap::from([(
                    "type".to_string(),
                    "feature".to_string(),
                )])),
            },
        );
        fields.insert(
            "area".to_string(),
            FieldSpec {
                values: vec!["rules".into(), "docs".into()],
                multi: true,
                required_when: None,
            },
        );
        Config {
            versions: vec!["v0.2.x".into()],
            title: "T".into(),
            source_note: None,
            fields,
        }
    }

    #[test]
    fn field_check_flags_unknown_value() {
        let mut r = ValidationReport::default();
        let feature = fm("feature", Some("enabler"), vec!["nope"], "next");
        check_feature_fields(Path::new("f.md"), &feature, &cfg_with_fields(), &mut r);
        assert!(r
            .schema_errors
            .iter()
            .any(|e| e.message.contains("unknown `area` value \"nope\"")));
    }

    #[test]
    fn field_check_requires_class_for_features() {
        let mut r = ValidationReport::default();
        let feature = fm("feature", None, vec!["rules"], "next");
        check_feature_fields(Path::new("f.md"), &feature, &cfg_with_fields(), &mut r);
        assert!(r.schema_errors.iter().any(|e| e
            .message
            .contains("`class` is required when type = \"feature\"")));
    }

    #[test]
    fn field_check_allows_missing_class_for_non_features() {
        let mut r = ValidationReport::default();
        let feature = fm("chore", None, vec!["rules"], "next");
        check_feature_fields(Path::new("f.md"), &feature, &cfg_with_fields(), &mut r);
        assert!(
            r.schema_errors.is_empty(),
            "chore without class must pass: {:?}",
            r.schema_errors
        );
    }

    #[test]
    fn field_check_flags_empty_area() {
        let mut r = ValidationReport::default();
        let feature = fm("feature", Some("enabler"), vec![], "next");
        check_feature_fields(Path::new("f.md"), &feature, &cfg_with_fields(), &mut r);
        assert!(r
            .schema_errors
            .iter()
            .any(|e| e.message.contains("`area` must list at least one value")));
    }

    /// Regression for the "`required_when` only honours `type`" bug: a
    /// condition keyed on a non-`type` field must still be evaluated.
    #[test]
    fn field_check_required_when_honours_non_type_key() {
        use crate::FieldSpec;
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "effort".to_string(),
            FieldSpec {
                values: vec!["S".into(), "M".into(), "L".into()],
                multi: false,
                required_when: Some(std::collections::HashMap::from([(
                    "horizon".to_string(),
                    "now".to_string(),
                )])),
            },
        );
        let config = Config {
            versions: vec!["v0.2.x".into()],
            title: "T".into(),
            source_note: None,
            fields,
        };
        // effort is unset and horizon == "now" → the rule must fire.
        let feature = fm("feature", None, vec!["rules"], "now");
        let mut r = ValidationReport::default();
        check_feature_fields(Path::new("f.md"), &feature, &config, &mut r);
        assert!(
            r.schema_errors.iter().any(|e| e
                .message
                .contains("`effort` is required when horizon = \"now\"")),
            "got: {:?}",
            r.schema_errors
        );

        // horizon != "now" → the rule must NOT fire.
        let other = fm("feature", None, vec!["rules"], "next");
        let mut r2 = ValidationReport::default();
        check_feature_fields(Path::new("f.md"), &other, &config, &mut r2);
        assert!(r2.schema_errors.is_empty(), "got: {:?}", r2.schema_errors);
    }

    /// Regression for the dead `multi` knob: `multi = false` must reject a
    /// field carrying more than one value.
    #[test]
    fn field_check_enforces_multi_false() {
        use crate::FieldSpec;
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "area".to_string(),
            FieldSpec {
                values: vec!["rules".into(), "docs".into()],
                multi: false,
                required_when: None,
            },
        );
        let config = Config {
            versions: vec!["v0.2.x".into()],
            title: "T".into(),
            source_note: None,
            fields,
        };
        let feature = fm("feature", None, vec!["rules", "docs"], "next");
        let mut r = ValidationReport::default();
        check_feature_fields(Path::new("f.md"), &feature, &config, &mut r);
        assert!(
            r.schema_errors.iter().any(|e| e
                .message
                .contains("`area` accepts a single value but 2 were given")),
            "got: {:?}",
            r.schema_errors
        );
    }

    #[test]
    fn config_check_flags_unknown_field_name() {
        use crate::FieldSpec;
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "bogus".to_string(),
            FieldSpec {
                values: vec!["x".into()],
                multi: false,
                required_when: None,
            },
        );
        fields.insert(
            "horizon".to_string(),
            FieldSpec {
                values: vec!["next".into()],
                multi: false,
                required_when: None,
            },
        );
        let config = Config {
            versions: vec!["v0.2.x".into()],
            title: "T".into(),
            source_note: None,
            fields,
        };
        let mut r = ValidationReport::default();
        check_config_fields(Path::new("config.toml"), &config, &mut r);
        assert!(
            r.schema_errors
                .iter()
                .any(|e| e.message.contains("unknown `[fields.bogus]`")),
            "got: {:?}",
            r.schema_errors
        );
    }

    #[test]
    fn config_check_flags_missing_horizon() {
        use crate::FieldSpec;
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "type".to_string(),
            FieldSpec {
                values: vec!["feature".into()],
                multi: false,
                required_when: None,
            },
        );
        let config = Config {
            versions: vec!["v0.2.x".into()],
            title: "T".into(),
            source_note: None,
            fields,
        };
        let mut r = ValidationReport::default();
        check_config_fields(Path::new("config.toml"), &config, &mut r);
        assert!(
            r.schema_errors
                .iter()
                .any(|e| e.message.contains("missing `[fields.horizon]`")),
            "got: {:?}",
            r.schema_errors
        );
    }
}
