+++
id = "F-ci-publish"
type = "chore"
effort = "M"
area = ["release"]
horizon = "next"
status = "done"
target = ["Later"]
+++

Automate the crates.io publish from CI via Trusted Publishing (OIDC), so a `v<semver>` tag ships the crate with no long-lived token stored anywhere — GitHub Actions authenticates to crates.io per-run and receives an ephemeral token. Removes the manual `cargo login` / `cargo publish` step.

Wired as a dist custom publish job (`publish-jobs = ["./publish-crates-io"]`): the dist-generated `release.yml` calls `.github/workflows/publish-crates-io.yml` after a successful `host`, dist grants it `id-token: write`, and `rust-lang/crates-io-auth-action` mints the ephemeral token for `cargo publish`. Kept `next` (not `shipped`) until a release actually publishes through this path; requires a one-time Trusted Publisher config on crates.io (repo `bastien-gallay/roadmark`, workflow `publish-crates-io.yml`).
