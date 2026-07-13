+++
id = "F-ci-publish"
type = "chore"
effort = "M"
area = ["release"]
horizon = "shipped"
status = "done"
target = ["v0.5"]
shipped = { version = "v0.5.1", date = "2026-07-13" }
shipped_order = 9
+++

Automate the crates.io publish from CI via Trusted Publishing (OIDC), so a `v<semver>` tag ships the crate with no long-lived token stored anywhere — GitHub Actions authenticates to crates.io per-run and receives an ephemeral token. Removes the manual `cargo login` / `cargo publish` step.

Wired as a dist custom publish job (`publish-jobs = ["./publish-crates-io"]`): the dist-generated `release.yml` calls `.github/workflows/publish-crates-io.yml` after a successful `host`, dist grants it `id-token: write`, and `rust-lang/crates-io-auth-action` mints the ephemeral token for `cargo publish`. First proven by the v0.5.1 release, which published to crates.io through this path. Requires a one-time Trusted Publisher config on crates.io (repo `bastien-gallay/roadmark`, workflow `release.yml` — the OIDC JWT names the entry-point workflow, not the reusable `publish-crates-io.yml` it calls).
