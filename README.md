# roadmap-cli

Generate a `ROADMAP.md` from a `.roadmap/` directory of TOML-frontmatter
feature files. The roadmap document becomes a **generated artifact**; the
source of truth is one small markdown file per feature, so roadmap edits
skip the usual "edit a big shared file" merge ceremony.

The binary is called `roadmap`.

## Install

Prebuilt binaries (macOS, Linux, Windows — see the
[latest release](https://github.com/bastien-gallay/roadmap-cli/releases/latest)):

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/bastien-gallay/roadmap-cli/releases/latest/download/roadmap-cli-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/bastien-gallay/roadmap-cli/releases/latest/download/roadmap-cli-installer.ps1 | iex"
```

Or with cargo:

```sh
cargo install --git https://github.com/bastien-gallay/roadmap-cli
```

Or from a local checkout:

```sh
cargo install --path .
```

## Source layout

The tool reads a `.roadmap/` directory (override with `--root`):

```text
.roadmap/
├── config.toml
└── features/
    ├── f-my-feature.md
    ├── f-another-thing.md
    └── ...
```

### `config.toml`

```toml
# Bucket order for sorting and section emission. Earliest cycle first.
versions = ["v0.1", "v0.2", "v0.3", "Later", "Speculative"]

# H1 heading of the generated ROADMAP.md. Optional, defaults to "Roadmap".
title = "My Project — Roadmap"

# Optional note appended to the generated "DO NOT EDIT" banner —
# e.g. a pointer to an ADR or design doc. Optional.
source_note = "See docs/adr/roadmap-pipeline.md for the design."

# Closed value-sets for the schema fields, owned by your project — the
# generator stays taxonomy-neutral. `validate` enforces membership.
[fields.type]
values = ["feature", "fix", "chore"]

[fields.class]
values = ["differentiator", "enabler", "table-stakes", "polish", "bet"]
required_when = { type = "feature" }   # class only on features

[fields.effort]
values = ["S", "M", "L"]

[fields.area]
values = ["core", "docs", "cli"]
multi = true

[fields.horizon]
values = ["now", "next", "later", "parked", "shipped"]   # order = sort rank

[fields.severity]
values = ["critical", "major", "minor"]
required_when = { type = "fix" }
```

### A feature file

Each `features/*.md` is TOML frontmatter fenced by `+++`, followed by a
markdown body whose first non-empty line becomes the catalog summary:

```markdown
+++
id = "F-my-feature"
type = "feature"        # feature | fix | chore
class = "enabler"       # feature-only leverage (see [fields.class])
effort = "M"            # S | M | L
area = ["core", "cli"]  # multi-valued taxonomy
horizon = "next"        # now | next | later | parked | shipped
status = "todo"         # wip | todo | done
target = ["v0.2"]       # first entry drives the sort bucket
+++

One-paragraph summary — the first non-empty line lands in the Summary column.
```

A fix carries a `severity` instead of a `class`:

```toml
id = "F-broken-anchor"
type = "fix"
severity = "major"      # fix-only (see [fields.severity])
area = ["core"]
horizon = "now"
status = "wip"
target = ["v0.2"]
```

Allowed values for `type`/`class`/`effort`/`area`/`horizon`/`severity` are
declared per-project in `config.toml` `[fields.*]` (above), not hardcoded in
the tool — so `roadmap` stays reusable across projects.

### Shipped entries

When a feature flips to `status = "done"`, record its shipping metadata so
historical order survives every regen:

```toml
shipped = { version = "v0.1", date = "2026-07-11", pr = 42 }
shipped_order = 3   # stable position within the shipped tier
```

### Generated output

`ROADMAP.md` has two parts:

- **Feature catalog** — one table row per feature: ID, Type, Class/Sev
  (`class` for features, `severity` for fixes — they share a column),
  Effort, Area, Horizon, Status, Target, Summary (the body's first
  non-empty line). The ID links to the feature's detail section.
- **Details** — one section per feature with the full markdown body,
  verbatim, prefixed by a `Shipped in <version> (<date>, PR #<n>).`
  line when the feature carries `shipped` metadata.

### Sort order

The catalog is sorted by a total key, so regeneration is byte-stable:
target bucket (order of `versions` in `config.toml`) → status
(wip → todo → done) → horizon (declared order of `[fields.horizon].values`)
→ `shipped_order` → id.

## Commands

```sh
roadmap generate > ROADMAP.md   # render to stdout
roadmap validate                # schema, slug uniqueness, anchor drift
roadmap add f-my-feature        # scaffold a new feature file
roadmap rename f-old f-new      # rename a slug, rewriting cross-links
```

`roadmap --root path/to/.roadmap generate` points at a non-default location.

### `validate`

Read-only. Reports:

- **schema errors** — malformed frontmatter, unknown field values, a
  single-valued field given a list, a missing `required_when` field, an
  unknown `[fields.*]` name, or a missing `[fields.horizon]` (does not abort
  on the first one)
- **duplicate ids** / **anchor collisions** — two features that would
  produce the same `<a id="…">` anchor
- **anchor drift** — anchors in the committed `ROADMAP.md` that a fresh
  regen would add or drop (catches "forgot to regenerate"). Pass
  `--accept-drift` to downgrade drift to a warning.

Exit code is non-zero on hard errors, or on drift unless `--accept-drift`.

### `rename`

`roadmap rename <from> <to>` moves `features/<from>.md` to
`features/<to>.md`, updates its `id`, and rewrites cross-references
(`[F-old](#f-old)` links, bare id mentions, `f-old.md` path references)
in every feature body. Matching is whole-token, so ids that merely share
a prefix (`F-old-widget`) are untouched. It refuses to overwrite an
existing file or collide with another feature's anchor. Regenerate
`ROADMAP.md` afterwards.

## Slug convention

New features use `f-<kebab-name>`. The legacy `f<digits>` form is rejected
by `add` (and as a `rename` target) unless `--allow-legacy-numeric` is
passed (migration only).

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) at your option.
