# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-07-12

### Added

- The generated `ROADMAP.md` now surfaces the schema-v2 fields: the
  catalog table gains Type, Class/Sev, Effort and Horizon columns, and
  a new Details section renders each feature's full markdown body plus
  a "Shipped in …" line from the `shipped` metadata.

## [0.2.0] - 2026-07-12

Schema v2 — **breaking change** to the feature-file frontmatter.

### Changed

- Frontmatter schema v2: two orthogonal axes replace the old flat
  `priority` — `class` (kind of leverage, feature-only) and `effort`
  (S/M/L). `topic` becomes the multi-valued `area`; `priority` becomes
  `horizon`; `type` (`feature | fix | chore`) and fix-only `severity`
  are new.
- Allowed values for `type`/`class`/`effort`/`area`/`horizon`/`severity`
  are no longer hardcoded: each project declares them in `config.toml`
  `[fields.*]` (closed value set, `multi` shape, `required_when`
  conditions). `validate` enforces the declarations.
- Sort key is now target bucket → status → horizon (declared value
  order) → `shipped_order` → id.

### Fixed

- `required_when` evaluates every condition key (AND semantics), not
  only `type`.
- A single-valued field given a TOML list is now a schema error (the
  `multi` flag is enforced).
- An unknown `[fields.*]` name in `config.toml` is rejected instead of
  silently disabling that field's validation.
- `[fields.horizon]` is required: every feature carries a horizon and
  it drives the sort order.

## [0.1.0] - 2026-07-11

Initial release.

### Added

- `roadmap generate` — render `ROADMAP.md` to stdout from a `.roadmap/`
  directory of TOML-frontmatter feature files (deterministic output).
- `roadmap validate` — schema errors, duplicate ids, anchor collisions,
  anchor drift against the committed `ROADMAP.md` (`--accept-drift`).
- `roadmap add` — scaffold a feature file (`f-<kebab-name>`; legacy
  `f<digits>` behind `--allow-legacy-numeric`).
- CRLF-authored feature files parse correctly.
- Prebuilt binaries for 5 targets plus shell/powershell installers
  (cargo-dist).

[0.3.0]: https://github.com/bastien-gallay/roadmap-cli/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/bastien-gallay/roadmap-cli/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/bastien-gallay/roadmap-cli/releases/tag/v0.1.0
