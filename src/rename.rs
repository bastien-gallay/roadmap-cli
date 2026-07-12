//! `rename` subcommand: rename a feature slug, moving its file and
//! rewriting cross-references in every feature body so anchors stay
//! consistent.
//!
//! The rewrite is token-based, not regex-based (see `replace_token`):
//! the old id (`F-old`), its anchor (`f-old`), and — when the filename
//! slug diverged from the id — the old slug are replaced only at
//! whole-token boundaries, so `[F-old](#f-old)` links, bare prose
//! mentions, and `f-old.md` path references all update while ids that
//! merely share a prefix (`F-old-widget`) are left alone.

use crate::add::{classify_slug, derive_id, SlugShape};
use crate::{feature_md_paths, parse_feature};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Outcome of `rename`. Mirrors `AddOutcome`: the lib reports what
/// happened, `main.rs` owns the user-facing text.
#[derive(Debug)]
pub struct RenameOutcome {
    pub old_path: PathBuf,
    pub new_path: PathBuf,
    /// Files (post-rename paths) whose content was rewritten — the
    /// renamed file itself plus any feature body holding a cross-reference.
    pub rewritten: Vec<PathBuf>,
    pub legacy_numeric_warning: bool,
}

pub fn rename(
    root: &Path,
    from: &str,
    to: &str,
    allow_legacy_numeric: bool,
) -> Result<RenameOutcome> {
    if from == to {
        bail!("`from` and `to` are both `{from}` — nothing to rename");
    }
    classify_slug(from).with_context(|| format!("invalid `from` slug `{from}`"))?;
    let to_shape = classify_slug(to).with_context(|| format!("invalid `to` slug `{to}`"))?;
    if to_shape == SlugShape::LegacyNumeric && !allow_legacy_numeric {
        bail!(
            "slug `{to}` is the legacy numeric form (`f<digits>`). Renames must target \
             `f-<kebab-name>`. If this is part of a one-shot migration, pass \
             `--allow-legacy-numeric`."
        );
    }

    let features_dir = root.join("features");
    let old_path = features_dir.join(format!("{from}.md"));
    let new_path = features_dir.join(format!("{to}.md"));
    if !old_path.is_file() {
        bail!("no such feature file: {}", old_path.display());
    }
    if new_path.exists() {
        bail!(
            "refusing to overwrite existing file: {}",
            new_path.display()
        );
    }

    // Read every feature file once; the same snapshot feeds the id
    // extraction, the collision scan, and the rewrite, so no file can
    // slip in (or change) between the check and the mutation.
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    for path in feature_md_paths(&features_dir)? {
        let src = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        files.push((path, src));
    }

    // Take the old id from the file itself, not from the filename — the
    // slug ↔ id convention holds for `add`-created files but rename must
    // not corrupt a file whose id diverged.
    let old_src = files
        .iter()
        .find(|(p, _)| *p == old_path)
        .map(|(_, s)| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("no such feature file: {}", old_path.display()))?;
    let old_id = parse_feature(old_src)
        .with_context(|| format!("parsing {}", old_path.display()))?
        .frontmatter
        .id;
    if old_id.is_empty() {
        bail!(
            "{} has an empty `id` — fix it before renaming",
            old_path.display()
        );
    }
    let new_id = derive_id(to);

    // Refuse a rename that would collide with another feature's anchor,
    // or whose old id another feature also carries (the blanket token
    // rewrite would silently change that feature's identity too).
    // Anchors are lowercased ids, so both checks are case-insensitive.
    // Files that don't parse are skipped — `validate` owns reporting them.
    for (path, src) in &files {
        if *path == old_path {
            continue;
        }
        let Ok(f) = parse_feature(src) else { continue };
        if f.frontmatter.id.to_lowercase() == new_id.to_lowercase() {
            bail!(
                "id `{new_id}` would collide with `{}` ({})",
                f.frontmatter.id,
                path.display()
            );
        }
        if f.frontmatter.id.to_lowercase() == old_id.to_lowercase() {
            bail!(
                "id `{old_id}` is also carried by {} — fix the duplicate \
                 (`roadmap validate`) before renaming",
                path.display()
            );
        }
    }

    // Compute every rewrite in memory before touching the disk, then
    // mutate in re-run-recoverable order: cross-referencing files first
    // (idempotent no-ops on a re-run), then the move, then the renamed
    // file's own content — so an interruption anywhere before the final
    // write leaves a tree that re-running the same command repairs.
    let mut rewritten = Vec::new();
    let mut renamed_out = None;
    let mut other_writes = Vec::new();
    for (path, src) in &files {
        let out = rewrite_refs(src, &old_id, &new_id, from, to);
        if out != *src {
            if *path == old_path {
                renamed_out = Some(out);
                rewritten.push(new_path.clone());
            } else {
                other_writes.push((path.clone(), out));
                rewritten.push(path.clone());
            }
        }
    }
    for (path, out) in &other_writes {
        std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
    }
    std::fs::rename(&old_path, &new_path)
        .with_context(|| format!("renaming {} → {}", old_path.display(), new_path.display()))?;
    if let Some(out) = renamed_out {
        std::fs::write(&new_path, out)
            .with_context(|| format!("writing {}", new_path.display()))?;
    }

    Ok(RenameOutcome {
        old_path,
        new_path,
        rewritten,
        legacy_numeric_warning: to_shape == SlugShape::LegacyNumeric,
    })
}

/// Rewrite all whole-token references to `old_id` (and its lowercased
/// anchor form) into `new_id` (and its anchor), plus references to the
/// old filename slug when it diverged from the id. Pure string → string
/// so it unit-tests without a filesystem.
pub fn rewrite_refs(src: &str, old_id: &str, new_id: &str, from: &str, to: &str) -> String {
    let old_anchor = old_id.to_lowercase();
    let new_anchor = new_id.to_lowercase();
    // When the old id is its own anchor form (already lowercase), every
    // occurrence may be a `#…` link target, and anchors are lowercased
    // ids — so substitute the anchor form of the new id, keeping links
    // alive (and the file's lowercase id style intact).
    let mut out = if old_id == old_anchor {
        replace_token(src, old_id, &new_anchor)
    } else {
        let pass = replace_token(src, old_id, new_id);
        replace_token(&pass, &old_anchor, &new_anchor)
    };
    // Filename references (`f-slug.md`) key on the slug, not the id;
    // when the two diverged the passes above never touch them.
    if from != old_anchor {
        out = replace_token(&out, from, to);
    }
    out
}

/// Slug/id characters — a match flanked by any of these is a longer
/// token (`F-old-widget`), not a reference to `F-old`.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

/// Replace `old` with `new` only where the match sits at token
/// boundaries. Manual scanner — the shape is fixed and narrow, doesn't
/// justify a regex dep.
fn replace_token(text: &str, old: &str, new: &str) -> String {
    if old.is_empty() {
        // `find("")` always matches at 0 without consuming — guard the
        // degenerate needle instead of spinning forever.
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(old) {
        // A match at the start of `rest` sits right after previously
        // consumed text — its true left neighbour is `out`'s last char,
        // not start-of-string.
        let prev = rest[..pos]
            .chars()
            .next_back()
            .or_else(|| out.chars().next_back());
        let before_ok = prev.is_none_or(|c| !is_token_char(c));
        let after = &rest[pos + old.len()..];
        let after_ok = after.chars().next().is_none_or(|c| !is_token_char(c));
        out.push_str(&rest[..pos]);
        out.push_str(if before_ok && after_ok { new } else { old });
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_tmp(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("roadmap-cli-rename-{label}-{pid}-{n}"))
    }

    fn feature_src(id: &str, body: &str) -> String {
        format!(
            "+++\n\
             id = \"{id}\"\n\
             type = \"feature\"\n\
             area = [\"core\"]\n\
             horizon = \"next\"\n\
             status = \"todo\"\n\
             target = [\"v0.3\"]\n\
             +++\n\n{body}\n"
        )
    }

    fn write_feature(root: &Path, slug: &str, id: &str, body: &str) {
        let dir = root.join("features");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{slug}.md")), feature_src(id, body)).unwrap();
    }

    #[test]
    fn replace_token_respects_boundaries() {
        assert_eq!(replace_token("see F-old.", "F-old", "F-new"), "see F-new.");
        assert_eq!(
            replace_token("F-old-widget stays", "F-old", "F-new"),
            "F-old-widget stays"
        );
        assert_eq!(
            replace_token("xF-old stays too", "F-old", "F-new"),
            "xF-old stays too"
        );
        assert_eq!(replace_token("F-old", "F-old", "F-new"), "F-new");
    }

    #[test]
    fn replace_token_keeps_context_across_adjacent_matches() {
        // The second occurrence sits mid-token (after the first one's
        // `d`) — losing the left context across iterations used to
        // rewrite it.
        assert_eq!(replace_token("F-oldF-old", "F-old", "F-new"), "F-oldF-old");
        assert_eq!(
            replace_token("aF-oldF-old", "F-old", "F-new"),
            "aF-oldF-old"
        );
        // Separated occurrences still both rewrite.
        assert_eq!(
            replace_token("F-old F-old", "F-old", "F-new"),
            "F-new F-new"
        );
    }

    #[test]
    fn replace_token_empty_needle_is_noop() {
        assert_eq!(replace_token("+++\nbody", "", "F-new"), "+++\nbody");
    }

    #[test]
    fn rewrite_refs_covers_links_prose_and_paths() {
        let src = "See [F-old](#f-old) and file f-old.md, unlike F-old-widget.";
        assert_eq!(
            rewrite_refs(src, "F-old", "F-new", "f-old", "f-new"),
            "See [F-new](#f-new) and file f-new.md, unlike F-old-widget."
        );
    }

    #[test]
    fn rewrite_refs_updates_frontmatter_id_line() {
        let src = feature_src("F-old", "Body.");
        let out = rewrite_refs(&src, "F-old", "F-new", "f-old", "f-new");
        assert!(out.contains("id = \"F-new\""));
        assert!(!out.contains("F-old"));
    }

    #[test]
    fn rewrite_refs_lowercase_id_stays_anchor_safe() {
        // A file whose id is already its anchor form must not have its
        // references rewritten to the capitalized new id — anchors are
        // lowercased ids, so `#F-new` would be a dead link.
        let src = "See [f-old](#f-old) in f-old.md.";
        assert_eq!(
            rewrite_refs(src, "f-old", "F-new", "f-old", "f-new"),
            "See [f-new](#f-new) in f-new.md."
        );
    }

    #[test]
    fn rewrite_refs_rewrites_diverged_slug_path_refs() {
        // Filename references key on the slug; when slug and id diverged
        // the id/anchor passes never contain them.
        let src = "See [F-actual](#f-actual), stored in f-slug.md.";
        assert_eq!(
            rewrite_refs(src, "F-actual", "F-new", "f-slug", "f-new"),
            "See [F-new](#f-new), stored in f-new.md."
        );
    }

    #[test]
    fn rename_moves_file_and_rewrites_cross_refs() {
        let root = unique_tmp("ok");
        write_feature(&root, "f-old", "F-old", "The old feature.");
        write_feature(&root, "f-other", "F-other", "Depends on [F-old](#f-old).");

        let out = rename(&root, "f-old", "f-new", false).unwrap();
        assert!(out.new_path.ends_with("features/f-new.md"));
        assert!(!out.old_path.exists());
        assert!(!out.legacy_numeric_warning);
        assert_eq!(out.rewritten.len(), 2);

        let renamed = std::fs::read_to_string(&out.new_path).unwrap();
        assert!(renamed.contains("id = \"F-new\""));
        let other = std::fs::read_to_string(root.join("features/f-other.md")).unwrap();
        assert!(other.contains("Depends on [F-new](#f-new)."));
    }

    #[test]
    fn rename_migrates_legacy_numeric_slug() {
        let root = unique_tmp("legacy-from");
        write_feature(&root, "f139", "F139", "Legacy feature.");
        write_feature(&root, "f-other", "F-other", "See [F139](#f139).");

        let out = rename(&root, "f139", "f-legacy-thing", false).unwrap();
        assert!(out.new_path.ends_with("features/f-legacy-thing.md"));
        let other = std::fs::read_to_string(root.join("features/f-other.md")).unwrap();
        assert!(other.contains("See [F-legacy-thing](#f-legacy-thing)."));
    }

    #[test]
    fn rename_rejects_legacy_target_without_flag() {
        let root = unique_tmp("legacy-to");
        write_feature(&root, "f-old", "F-old", "Body.");
        let err = rename(&root, "f-old", "f200", false).unwrap_err();
        assert!(format!("{err:#}").contains("--allow-legacy-numeric"));
        let out = rename(&root, "f-old", "f200", true).unwrap();
        assert!(out.legacy_numeric_warning);
    }

    #[test]
    fn rename_refuses_missing_source_and_existing_target() {
        let root = unique_tmp("guards");
        write_feature(&root, "f-a", "F-a", "A.");
        write_feature(&root, "f-b", "F-b", "B.");
        let err = rename(&root, "f-nope", "f-x", false).unwrap_err();
        assert!(format!("{err:#}").contains("no such feature file"));
        let err = rename(&root, "f-a", "f-b", false).unwrap_err();
        assert!(format!("{err:#}").contains("refusing to overwrite"));
    }

    #[test]
    fn rename_refuses_anchor_collision_with_other_id() {
        let root = unique_tmp("collision");
        write_feature(&root, "f-a", "F-a", "A.");
        // Same anchor as the rename target, different filename.
        write_feature(&root, "f-elsewhere", "F-taken", "B.");
        let err = rename(&root, "f-a", "f-taken", false).unwrap_err();
        assert!(format!("{err:#}").contains("would collide with `F-taken`"));
    }

    #[test]
    fn rename_rejects_noop_and_bad_slugs() {
        let root = unique_tmp("noop");
        write_feature(&root, "f-a", "F-a", "A.");
        assert!(rename(&root, "f-a", "f-a", false).is_err());
        assert!(rename(&root, "f-a", "F-Bad", false).is_err());
        assert!(rename(&root, "bad", "f-b", false).is_err());
    }

    #[test]
    fn rename_uses_file_id_not_filename() {
        let root = unique_tmp("diverged");
        // Filename and id diverged — the rewrite must key on the id for
        // anchors AND still fix path references to the old filename.
        write_feature(&root, "f-slug", "F-actual", "Body.");
        write_feature(
            &root,
            "f-other",
            "F-other",
            "See [F-actual](#f-actual) in f-slug.md.",
        );
        let out = rename(&root, "f-slug", "f-new", false).unwrap();
        let renamed = std::fs::read_to_string(&out.new_path).unwrap();
        assert!(renamed.contains("id = \"F-new\""));
        let other = std::fs::read_to_string(root.join("features/f-other.md")).unwrap();
        assert!(other.contains("See [F-new](#f-new) in f-new.md."));
    }

    #[test]
    fn rename_bails_on_empty_id() {
        let root = unique_tmp("empty-id");
        write_feature(&root, "f-old", "", "Body.");
        let err = rename(&root, "f-old", "f-new", false).unwrap_err();
        assert!(format!("{err:#}").contains("empty `id`"));
        // Nothing was mutated.
        assert!(root.join("features/f-old.md").is_file());
    }

    #[test]
    fn rename_bails_on_duplicate_old_id() {
        let root = unique_tmp("dup-old-id");
        write_feature(&root, "f-a", "F-dup", "A.");
        write_feature(&root, "f-copy", "F-dup", "B.");
        let err = rename(&root, "f-a", "f-new", false).unwrap_err();
        assert!(format!("{err:#}").contains("is also carried by"));
        // Neither file changed identity.
        let copy = std::fs::read_to_string(root.join("features/f-copy.md")).unwrap();
        assert!(copy.contains("id = \"F-dup\""));
        assert!(root.join("features/f-a.md").is_file());
    }
}
