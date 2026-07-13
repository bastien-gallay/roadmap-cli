+++
id = "F-crates-io"
type = "chore"
effort = "S"
area = ["release", "docs"]
horizon = "shipped"
status = "done"
target = ["v0.5"]
shipped = { version = "v0.5.0", date = "2026-07-12" }
shipped_order = 8
+++

Published `roadmark` to crates.io — it now installs with `cargo install roadmark` — and added the crates.io version badge to the README. The published crate is trimmed to sources, README, changelog, and the license pair via an `include` allowlist.
