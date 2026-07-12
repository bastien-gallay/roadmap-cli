//! Pure functions for the `roadmap` generator.
//!
//! Wired by `main.rs`. Kept fs-free so unit tests can pass strings
//! and snapshot the rendered output via `insta`.

pub mod add;
pub mod validate;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// One feature: TOML frontmatter + raw markdown body.
///
/// Body stays an unparsed `String` — a markdown parser would round-trip
/// poorly (loses author intent on edge cases), and the renderer only
/// needs the first paragraph for the catalog summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feature {
    pub frontmatter: Frontmatter,
    pub body: String,
}

/// Schema v2 frontmatter. Two orthogonal axes replace the old flat
/// `priority`: `class` (kind of leverage) and `effort`. The taxonomy
/// (`area`) is multi-valued. Allowed values for `type`/`class`/`effort`/
/// `area`/`horizon`/`severity` are **not** hardcoded here — they are
/// declared per-project in `config.toml` `[fields.*]` and enforced by
/// `validate`, so this generator stays reusable across projects.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Frontmatter {
    pub id: String,
    /// `feature | fix | chore`. Only features carry a `class`; only
    /// fixes carry a `severity`. `type` is a Rust keyword → renamed.
    #[serde(rename = "type")]
    pub item_type: String,
    /// Kind of leverage (feature-only): differentiator/enabler/… .
    #[serde(default)]
    pub class: Option<String>,
    /// S / M / L. Optional during migration (backfilled by triage).
    #[serde(default)]
    pub effort: Option<String>,
    /// Multi-valued taxonomy (renamed from the old single `topic`).
    pub area: Vec<String>,
    /// Ordering horizon (renamed from the old `priority`). Sort rank
    /// comes from the declared order of `[fields.horizon].values`.
    pub horizon: String,
    pub status: Status,
    pub target: Vec<String>,
    /// Fix-only severity: critical/major/minor.
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub shipped: Shipped,
    /// Stable position within the "shipped" tier — set at flip-time
    /// (status: todo → done) so historical order survives regen.
    /// Optional; only required once the catalog includes shipped entries.
    #[serde(default)]
    pub shipped_order: Option<u32>,
}

impl Frontmatter {
    /// The schema fields this generator models. A config `[fields.*]` name
    /// outside this set is a typo that would otherwise silently disable
    /// validation, so `validate` rejects it.
    pub const FIELD_NAMES: &'static [&'static str] =
        &["type", "class", "effort", "area", "horizon", "severity"];

    /// Values a named schema field currently holds, for config-driven
    /// validation. `None` = this generator does not model a field of that
    /// name (config references something unknown → the caller skips it).
    /// `Some(vec)` = the present values (empty when an optional field is
    /// unset), so the caller can enforce `required_when` and membership.
    pub fn field_values(&self, name: &str) -> Option<Vec<String>> {
        let one = |s: &str| vec![s.to_string()];
        let opt = |o: &Option<String>| o.iter().cloned().collect::<Vec<_>>();
        match name {
            "type" => Some(one(&self.item_type)),
            "class" => Some(opt(&self.class)),
            "effort" => Some(opt(&self.effort)),
            "area" => Some(self.area.clone()),
            "horizon" => Some(one(&self.horizon)),
            "severity" => Some(opt(&self.severity)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Wip,
    Todo,
    Done,
}

impl Status {
    const fn rank(self) -> u8 {
        match self {
            Self::Wip => 0,
            Self::Todo => 1,
            Self::Done => 2,
        }
    }

    const fn glyph(self) -> &'static str {
        match self {
            Self::Wip => "🚧",
            Self::Todo => "☐",
            Self::Done => "✅",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Shipped {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub pr: u32,
}

/// `.roadmap/config.toml` contents.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Bucket order for sorting and section emission. Earliest cycle first.
    pub versions: Vec<String>,
    /// H1 heading for the generated `ROADMAP.md`. Defaults to `"Roadmap"`.
    #[serde(default = "default_title")]
    pub title: String,
    /// Optional project-specific note appended to the generated
    /// "DO NOT EDIT" banner — e.g. a pointer to an ADR or design doc.
    #[serde(default)]
    pub source_note: Option<String>,
    /// Per-field allowed-value declarations, keyed by field name
    /// (`type`, `class`, `effort`, `area`, `horizon`, `severity`).
    /// `BTreeMap` so validation errors emit in a stable order.
    #[serde(default)]
    pub fields: BTreeMap<String, FieldSpec>,
}

/// Declares the allowed values (and shape) of one schema field, so the
/// project — not this binary — owns its taxonomy.
#[derive(Debug, Clone, Deserialize)]
pub struct FieldSpec {
    /// The closed set of accepted values.
    pub values: Vec<String>,
    /// Whether the frontmatter field is an array (e.g. `area`).
    #[serde(default)]
    pub multi: bool,
    /// Conditional presence: e.g. `{ type = "feature" }` makes the field
    /// required only when the feature's `type` equals `"feature"`.
    #[serde(default)]
    pub required_when: Option<HashMap<String, String>>,
}

fn default_title() -> String {
    "Roadmap".to_string()
}

/// Split a `+++`-fenced frontmatter doc into TOML head + markdown body.
///
/// Accepts trailing newline after closing fence. Body is returned
/// trimmed of one leading blank line if present.
pub fn split_frontmatter(src: &str) -> Result<(&str, &str)> {
    let rest = src
        .strip_prefix("+++\n")
        .ok_or_else(|| anyhow!("missing opening `+++` fence"))?;
    let end = rest
        .find("\n+++")
        .ok_or_else(|| anyhow!("missing closing `+++` fence"))?;
    let toml_block = &rest[..end];
    // Skip the closing fence + its trailing newline if present.
    let after = &rest[end + "\n+++".len()..];
    let body = after.strip_prefix('\n').unwrap_or(after);
    let body = body.strip_prefix('\n').unwrap_or(body);
    Ok((toml_block, body))
}

pub fn parse_feature(src: &str) -> Result<Feature> {
    // Normalize CRLF so Windows-authored (or autocrlf-checked-out) files
    // parse identically; the renderer emits LF-only output either way.
    let normalized: std::borrow::Cow<'_, str> = if src.contains('\r') {
        std::borrow::Cow::Owned(src.replace("\r\n", "\n"))
    } else {
        std::borrow::Cow::Borrowed(src)
    };
    let (toml_block, body) = split_frontmatter(&normalized)?;
    let frontmatter: Frontmatter =
        toml::from_str(toml_block).context("invalid TOML frontmatter")?;
    Ok(Feature {
        frontmatter,
        body: body.to_string(),
    })
}

/// Sort key: target[0] (via config bucket order) → status → horizon
/// (via config `[fields.horizon]` order) → shipped_order → id.
///
/// `shipped_order` (set at flip-time so historical order survives regen)
/// must sit *before* `id` in the key: `id` is unique, so any tiebreak
/// placed after it would never run. Features without a `shipped_order`
/// sort last within their tier (via `u32::MAX`), then break ties by `id`.
/// Unknown targets and unknown horizons sort last; missing target arrays
/// are an upstream schema error caught at parse time.
fn sort_key<'a>(
    f: &'a Feature,
    version_index: &HashMap<&str, usize>,
    horizon_index: &HashMap<&str, usize>,
) -> (usize, u8, usize, u32, &'a str) {
    let target_idx = f
        .frontmatter
        .target
        .first()
        .and_then(|t| version_index.get(t.as_str()).copied())
        .unwrap_or(usize::MAX);
    let horizon_idx = horizon_index
        .get(f.frontmatter.horizon.as_str())
        .copied()
        .unwrap_or(usize::MAX);
    (
        target_idx,
        f.frontmatter.status.rank(),
        horizon_idx,
        f.frontmatter.shipped_order.unwrap_or(u32::MAX),
        &f.frontmatter.id,
    )
}

/// Build a value → declaration-order index for stable ranking.
fn index_of(values: &[String]) -> HashMap<&str, usize> {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_str(), i))
        .collect()
}

pub fn sort_features(features: &mut [Feature], config: &Config) {
    let version_index = index_of(&config.versions);
    let horizon_index = config
        .fields
        .get("horizon")
        .map(|s| index_of(&s.values))
        .unwrap_or_default();
    features.sort_by(|a, b| {
        sort_key(a, &version_index, &horizon_index).cmp(&sort_key(
            b,
            &version_index,
            &horizon_index,
        ))
    });
}

/// First non-empty line of the body, used as the catalog summary.
fn summary(body: &str) -> &str {
    body.lines()
        .find(|l| !l.trim().is_empty())
        .map(str::trim)
        .unwrap_or("")
}

/// HTML id for the anchor: lowercase the feature id.
/// Matches GitHub's `<a id="f46">` / `<a id="f-foo">` convention.
fn anchor_id(id: &str) -> String {
    id.to_lowercase()
}

/// Escape free text destined for a `|`-delimited markdown table cell:
/// a literal `|` would open a spurious column and a newline would break
/// the row, so escape the former and fold the latter to a space.
fn escape_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

pub fn render(features: &[Feature], config: &Config) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(8 * 1024);
    let _ = writeln!(out, "# {}\n", config.title);
    out.push_str(
        "<!-- DO NOT EDIT — generated by `roadmap generate`. Source of truth: `.roadmap/`.",
    );
    if let Some(note) = &config.source_note {
        // A literal `-->` in the note would close the banner comment early,
        // leaking the remainder as visible text — neutralise it.
        let _ = write!(out, " {}", note.replace("-->", "--&gt;"));
    }
    out.push_str(" -->\n\n");
    out.push_str("## Feature catalog\n\n");
    out.push_str("| ID | Area | Status | Target | Summary |\n");
    out.push_str("|---|---|---|---|---|\n");
    for f in features {
        let fm = &f.frontmatter;
        let aid = anchor_id(&fm.id);
        let target = fm.target.join(" → ");
        let area = fm.area.join(", ");
        let _ = writeln!(
            out,
            "| <a id=\"{aid}\"></a>[{id}](#{aid}) | {area} | {status} | {target} | {summary} |",
            id = fm.id,
            area = escape_cell(&area),
            status = fm.status.glyph(),
            target = escape_cell(&target),
            summary = escape_cell(summary(&f.body)),
        );
    }
    out
}

/// List `*.md` files directly under `dir`, in filename order.
///
/// Single source of the "which files are feature files" rule, shared by
/// `load_features` (generate) and `validate` so the two can never drift
/// on the walk depth, sort, or extension filter.
pub fn feature_md_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .min_depth(1)
        .max_depth(1)
        .sort_by_file_name()
    {
        let entry = entry.context("walking features dir")?;
        if entry.file_type().is_file() && entry.path().extension().is_some_and(|e| e == "md") {
            paths.push(entry.path().to_path_buf());
        }
    }
    Ok(paths)
}

/// Read all `.roadmap/features/*.md` files under `root`, parse each,
/// and return them in load order (caller sorts).
pub fn load_features(root: &Path) -> Result<Vec<Feature>> {
    let dir = root.join("features");
    if !dir.is_dir() {
        bail!("expected directory: {}", dir.display());
    }
    let mut out = Vec::new();
    for path in feature_md_paths(&dir)? {
        let src = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let feature = parse_feature(&src).with_context(|| format!("parsing {}", path.display()))?;
        out.push(feature);
    }
    Ok(out)
}

pub fn load_config(root: &Path) -> Result<Config> {
    let path = root.join("config.toml");
    let src =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&src).with_context(|| format!("parsing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        let mut fields = BTreeMap::new();
        fields.insert(
            "horizon".to_string(),
            FieldSpec {
                values: ["now", "next", "later", "parked", "shipped"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                multi: false,
                required_when: None,
            },
        );
        Config {
            versions: vec!["v0.2.x".into(), "v0.3".into(), "v0.4".into()],
            title: "Roadmap".into(),
            source_note: None,
            fields,
        }
    }

    fn feat(id: &str, status: Status, horizon: &str, target: &str) -> Feature {
        Feature {
            frontmatter: Frontmatter {
                id: id.into(),
                item_type: "feature".into(),
                class: None,
                effort: None,
                area: vec!["arch".into()],
                horizon: horizon.into(),
                status,
                target: vec![target.into()],
                severity: None,
                shipped: Shipped::default(),
                shipped_order: None,
            },
            body: "Summary line.".into(),
        }
    }

    #[test]
    fn split_frontmatter_basic() {
        let src = "+++\nid = \"f1\"\n+++\n\nbody text\n";
        let (toml, body) = split_frontmatter(src).unwrap();
        assert_eq!(toml, "id = \"f1\"");
        assert_eq!(body, "body text\n");
    }

    #[test]
    fn parse_minimal() {
        let src = "+++\n\
id = \"F-foo\"\n\
type = \"feature\"\n\
area = [\"arch\"]\n\
horizon = \"next\"\n\
status = \"todo\"\n\
target = [\"v0.2.x\"]\n\
+++\n\nThe summary.\n";
        let f = parse_feature(src).unwrap();
        assert_eq!(f.frontmatter.id, "F-foo");
        assert_eq!(f.frontmatter.item_type, "feature");
        assert_eq!(f.frontmatter.area, vec!["arch".to_string()]);
        assert_eq!(f.frontmatter.horizon, "next");
        assert_eq!(f.frontmatter.status, Status::Todo);
        assert_eq!(f.body, "The summary.\n");
    }

    #[test]
    fn parse_accepts_crlf_line_endings() {
        let src = "+++\r\n\
id = \"F-foo\"\r\n\
type = \"feature\"\r\n\
area = [\"arch\"]\r\n\
horizon = \"next\"\r\n\
status = \"todo\"\r\n\
target = [\"v0.2.x\"]\r\n\
+++\r\n\r\nThe summary.\r\n";
        let f = parse_feature(src).unwrap();
        assert_eq!(f.frontmatter.id, "F-foo");
        assert_eq!(f.body, "The summary.\n");
    }

    #[test]
    fn sort_target_then_status_then_horizon_then_id() {
        let mut fs = vec![
            feat("f-z", Status::Todo, "next", "v0.3"),
            feat("f-a", Status::Todo, "next", "v0.2.x"),
            feat("f-b", Status::Wip, "next", "v0.2.x"),
            feat("f-c", Status::Todo, "later", "v0.2.x"),
        ];
        sort_features(&mut fs, &cfg());
        let ids: Vec<&str> = fs.iter().map(|f| f.frontmatter.id.as_str()).collect();
        assert_eq!(ids, vec!["f-b", "f-a", "f-c", "f-z"]);
    }

    #[test]
    fn anchor_lowercases_id() {
        assert_eq!(anchor_id("F-Roadmap-TOML"), "f-roadmap-toml");
        assert_eq!(anchor_id("F22"), "f22");
    }

    #[test]
    fn title_defaults_when_omitted() {
        let config: Config = toml::from_str("versions = [\"v1\"]\n").unwrap();
        assert_eq!(config.title, "Roadmap");
        assert!(config.source_note.is_none());
        assert!(config.fields.is_empty());
    }

    #[test]
    fn fields_parse_from_config() {
        let src = "versions = [\"v1\"]\n\
[fields.class]\n\
values = [\"differentiator\", \"enabler\"]\n\
[fields.area]\n\
values = [\"rules\", \"docs\"]\n\
multi = true\n\
[fields.class.required_when]\n\
type = \"feature\"\n";
        let config: Config = toml::from_str(src).unwrap();
        assert_eq!(config.fields["area"].values, vec!["rules", "docs"]);
        assert!(config.fields["area"].multi);
        assert!(!config.fields["class"].multi);
        assert_eq!(
            config.fields["class"].required_when.as_ref().unwrap()["type"],
            "feature"
        );
    }

    #[test]
    fn render_uses_title_and_source_note() {
        let config = Config {
            versions: vec!["v1".into()],
            title: "My Project — Roadmap".into(),
            source_note: Some("See docs/adr.".into()),
            fields: BTreeMap::new(),
        };
        let out = render(&[], &config);
        assert!(out.starts_with("# My Project — Roadmap\n\n"));
        assert!(out.contains("generated by `roadmap generate`"));
        assert!(out.contains("Source of truth: `.roadmap/`. See docs/adr. -->"));
    }

    #[test]
    fn shipped_order_breaks_ties_before_id() {
        // Same target/status/horizon, distinct ids — the alphabetically
        // later id (f-zeta) must still sort first because its shipped_order
        // is lower. Regression guard: this only works when shipped_order
        // sits before id in the sort key.
        let mut a = feat("f-alpha", Status::Done, "shipped", "v0.2.x");
        a.frontmatter.shipped_order = Some(3);
        let mut z = feat("f-zeta", Status::Done, "shipped", "v0.2.x");
        z.frontmatter.shipped_order = Some(1);
        let mut fs = vec![a, z];
        sort_features(&mut fs, &cfg());
        let ids: Vec<&str> = fs.iter().map(|f| f.frontmatter.id.as_str()).collect();
        assert_eq!(ids, vec!["f-zeta", "f-alpha"]);
    }

    #[test]
    fn features_without_shipped_order_fall_back_to_id() {
        let mut fs = vec![
            feat("f-b", Status::Done, "shipped", "v0.2.x"),
            feat("f-a", Status::Done, "shipped", "v0.2.x"),
        ];
        sort_features(&mut fs, &cfg());
        let ids: Vec<&str> = fs.iter().map(|f| f.frontmatter.id.as_str()).collect();
        assert_eq!(ids, vec!["f-a", "f-b"]);
    }

    #[test]
    fn escape_cell_escapes_pipes_and_folds_newlines() {
        assert_eq!(escape_cell("CLI | TUI"), "CLI \\| TUI");
        assert_eq!(escape_cell("line1\nline2"), "line1 line2");
    }

    #[test]
    fn render_escapes_pipe_in_free_text_columns() {
        let mut f = feat("f-x", Status::Todo, "next", "v0.2.x");
        f.frontmatter.area = vec!["CLI | TUI".into()];
        f.body = "Support `a | b` operator.".into();
        let out = render(&[f], &cfg());
        // The row must carry exactly the 5 intended column separators plus
        // the two escaped literals — never a raw unescaped `|` in the text.
        assert!(out.contains("CLI \\| TUI"));
        assert!(out.contains("Support `a \\| b` operator."));
    }

    #[test]
    fn render_neutralises_comment_terminator_in_source_note() {
        let config = Config {
            versions: vec!["v1".into()],
            title: "T".into(),
            source_note: Some("see foo --> bar".into()),
            fields: BTreeMap::new(),
        };
        let out = render(&[], &config);
        // The only `-->` in the output is the banner's own closing fence.
        assert_eq!(out.matches("-->").count(), 1);
        assert!(out.contains("see foo --&gt; bar"));
    }
}
