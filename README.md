# roadmark

**Your roadmap as code — compiled, versioned, and validated in CI, so it
never rots.**

> **Naming:** the crate and the binary are both `roadmark`. The GitHub
> repository is still `bastien-gallay/roadmap-cli` until it is renamed, so
> the install URLs below keep that path. The `roadmark`-named release
> artifacts ship with the first release cut after the rename; until then,
> install with `cargo install --git …` or from a local checkout.

---

> **We believe** the hand-maintained `ROADMAP.md` is rotting debt: every
> sprint it drifts from the code, lies to contributors, and ends up
> abandoned in a corner of the repo.
> **We believe** planning deserves the same rigor as code — compiled,
> versioned, automatically validated — not held together by human goodwill.
> **That's why** roadmark compiles your roadmap from atomic feature files
> and **breaks your CI** the moment it becomes inconsistent. Discipline
> becomes mechanical, not moral.

Built to scratch my own itch — this repo dogfoods roadmark on its own
`.roadmap/`.

---

## What it is

roadmark is a **roadmap-as-code** tool for teams that live in their Git
repo. Instead of coordinating edits on one big roadmap file, each feature is
its own markdown file with TOML frontmatter. One command compiles them into
a `ROADMAP.md`; another **guarantees** the roadmap can't become incoherent —
enforced in CI, not by discipline.

It is **not** a task tracker (leave day-to-day tasks to tools like
Backlog.md), and **not** a hosted roadmap app (OpenProject / Productboard
style). It sits in the gap none of them fill: docs-as-code, at the roadmap
level, with a validation guarantee.

## Install

Prebuilt binaries (macOS, Linux, Windows — see the
[latest release](https://github.com/bastien-gallay/roadmap-cli/releases/latest)):

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/bastien-gallay/roadmap-cli/releases/latest/download/roadmark-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/bastien-gallay/roadmap-cli/releases/latest/download/roadmark-installer.ps1 | iex"
```

Or with cargo, from the Git repo or a local checkout:

```sh
cargo install --git https://github.com/bastien-gallay/roadmap-cli
cargo install --path .
```

All of these install a binary named `roadmark`.

## Quick start

```sh
roadmark add f-dark-mode        # scaffold a new feature file under .roadmap/features/
roadmark generate > ROADMAP.md  # compile features into ROADMAP.md
roadmark validate               # fail if the roadmap is inconsistent — run this in CI
```

`roadmark --root path/to/.roadmap generate` points at a non-default location.

---

## How it works — three layers

### 1. Author (the body): rich, Git-native roadmap management

One feature = one file. Two people never edit the same line, so the roadmap
has **zero merge conflicts**. Each feature carries structured frontmatter
plus a free markdown body whose first non-empty line becomes the catalog
summary. The taxonomy is **yours**: statuses, effort levels, horizons, and
areas are declared in `config.toml`, with no process religion baked in.

The tool reads a `.roadmap/` directory (override with `--root`):

```text
.roadmap/
├── config.toml
└── features/
    ├── f-dark-mode.md
    ├── f-another-thing.md
    └── ...
```

Each `features/*.md` is TOML frontmatter fenced by `+++`, followed by the
markdown body:

```markdown
+++
id = "F-dark-mode"
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

When a feature flips to `status = "done"`, record its shipping metadata so
historical order survives every regen:

```toml
shipped = { version = "v0.1", date = "2026-07-11", pr = 42 }
shipped_order = 3       # stable position within the shipped tier
```

The allowed values for every field are **config-owned, not hardcoded** —
the generator stays taxonomy-neutral so roadmark is reusable across
projects:

```toml
# .roadmap/config.toml
versions = ["v0.1", "v0.2", "v0.3", "Later"]   # sort buckets, earliest first
title = "My Project — Roadmap"                  # H1 of the generated doc

[fields.type]
values = ["feature", "fix", "chore"]

[fields.class]
values = ["differentiator", "enabler", "table-stakes", "polish", "bet"]
required_when = { type = "feature" }            # class only on features

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

### 2. Generate: the roadmap is a compiled artifact

`roadmark generate` compiles every feature file into a single, formatted
`ROADMAP.md` on stdout. The output has two parts — a **feature catalog**
(one table row per feature, ID linking to its detail section) and
**details** (each feature's full body, verbatim). It is **deterministic**:
the catalog is sorted by a total key (target bucket → status → horizon →
`shipped_order` → id), so regeneration is byte-stable and diffs stay clean.

### 3. Validate — the guarantee

This is the point. `roadmark validate` is read-only and reports:

- **schema errors** — malformed frontmatter, unknown field values, a
  single-valued field given a list, a missing `required_when` field
- **duplicate ids / anchor collisions** — two features that would produce
  the same `<a id="…">` anchor (checked case-insensitively)
- **anchor drift** — anchors the committed `ROADMAP.md` is missing or has
  stale, i.e. you forgot to regenerate (pass `--accept-drift` to downgrade
  to a warning)

Exit code is non-zero on hard errors, or on drift unless `--accept-drift`.
Wire it into CI and your roadmap **cannot** silently drift or lie:

```yaml
# .github/workflows/roadmap.yml  (sketch)
- run: roadmark validate    # the PR fails if the roadmap is inconsistent
```

`validate` silently passes when `.roadmap/` is absent, so the same recipe
runs on checkouts without the source tree.

## Other commands

```sh
roadmark rename f-old f-new      # move a feature file, rewriting every cross-link
```

`rename` moves `features/<from>.md` to `features/<to>.md`, updates its `id`,
and rewrites cross-references (`[F-old](#f-old)` links, bare id mentions,
`f-old.md` path references) in every feature body. Matching is whole-token,
so ids that merely share a prefix (`F-old-widget`) are untouched. It refuses
to overwrite an existing file, to collide with another feature's anchor, or
to run while the old id is duplicated. Regenerate `ROADMAP.md` afterwards.

New features use the `f-<kebab-name>` slug shape. The legacy `f<digits>`
form is rejected by `add` (and as a `rename` target) unless
`--allow-legacy-numeric` is passed (migration only).

---

## Reach — headless roadmap

Your roadmap's source of truth stays in Git; it **projects** to wherever
your team already works. Like a headless CMS, the canonical content lives in
one clean, versioned place and is rendered where it's needed.

| Projection | Direction | Status |
| --- | --- | --- |
| `ROADMAP.md` | files → doc | ✅ available |
| GitHub Projects | files → board | 🔭 planned (demand-driven) |
| Jira | files ↔ tool | 🔭 planned (demand-driven) |

**Design invariant:** the toml/md files are the single source of truth;
every backend is a projection reached through a **one-way** adapter, and no
external tool is ever co-authoritative. See
[`docs/adr/0001-single-source-of-truth.md`](docs/adr/0001-single-source-of-truth.md).
This is exactly what makes `validate`'s promise unconditional — the source
is guaranteed clean *before* it propagates anywhere.

---

## Why not just…

- **…a hand-edited `ROADMAP.md`?** It rots. It drifts from the code, causes
  merge conflicts, and no one trusts it after a month. roadmark makes
  coherence mechanical.
- **…a SaaS roadmap tool?** It lives outside the repo, invisible in code
  review, and disconnected from the code it describes. roadmark keeps the
  roadmap in the PR — and can still project *to* your SaaS if your team
  needs it.
- **…a markdown task manager (Backlog.md, etc.)?** Those track tasks;
  roadmark plans the roadmap above them. They're complementary, not
  competing.

## Status

Early and actively dogfooded. The core (`add` / `generate` / `validate` /
`rename`) is shipped and stable; external projections (GitHub Projects,
Jira) are planned and demand-driven. Issues and feedback welcome.

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) at your option.
