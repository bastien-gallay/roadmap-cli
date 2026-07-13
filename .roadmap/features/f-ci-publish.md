+++
id = "F-ci-publish"
type = "chore"
effort = "M"
area = ["release"]
horizon = "next"
status = "todo"
target = ["Later"]
+++

Automate the crates.io publish from CI via Trusted Publishing (OIDC), so a `v<semver>` tag ships the crate with no long-lived token stored anywhere — GitHub Actions authenticates to crates.io per-run and receives an ephemeral token. Removes the manual `cargo login` / `cargo publish` step.
