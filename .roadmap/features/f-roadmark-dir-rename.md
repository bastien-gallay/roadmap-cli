+++
id = "F-roadmark-dir-rename"
type = "chore"
effort = "M"
area = ["core", "cli"]
horizon = "parked"
status = "todo"
target = ["Later"]
+++

Rename the source directory `.roadmap/` → `.roadmark/` for brand coherence. Deferred and low priority while usage stays personal. If ever done, ship it non-breaking (option B): default to `.roadmark/`, fall back to `.roadmap/` with a deprecation warning — targeted at a future v1.0, not before. `.roadmap/` is arguably clearer and stays consistent with the `ROADMAP.md` output, so this may never be worth the churn.
