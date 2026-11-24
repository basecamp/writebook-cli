# Releasing

A release takes two runs of the release script, with a PR merged in between:

```sh
scripts/release.sh 0.2.0   # opens the "Prepare v0.2.0 release" PR and stops
# merge the PR, pull main, then:
scripts/release.sh 0.2.0   # pushes the v0.2.0 tag
```

The tag must point at a commit whose `Cargo.toml` has the same version, so the
first run sets the version on a `release/v0.2.0` branch and opens a PR for it.
The second run finds the version set and pushes the tag. The script never
pushes to `main`.

For a release candidate, use a version like `0.2.0-rc.1`. To run only the
checks, add `--dry-run`.

## What the script checks

Before it pushes anything, the script:

- refuses a version that isn't `X.Y.Z` or `X.Y.Z-suffix`
- needs you on `main`, with no changes, and in sync with `origin`
- refuses a tag that already exists at another commit
- refuses a version older than the latest release
- runs clippy and the tests (the live tests only when `WRITEBOOK_LIVE_URL` is set)

If the tag push is rejected, it deletes the local tag.

## What the release workflow does

[`.github/workflows/release.yml`](.github/workflows/release.yml) runs on the
tag. It checks that the tag is on `main` and matches `Cargo.toml`, runs CI,
builds the binaries for Linux and macOS, attests their build provenance, and
publishes the GitHub release.

- A release candidate is marked as a prerelease and doesn't become the latest
  release, so `mise` and `writebook upgrade` don't pick it up.
- The release notes list the PRs since the previous release that isn't a
  release candidate. The release PR is left out by its `release` label.

## Build provenance

mise checks each release's build provenance. Once one version has it, mise
refuses later versions that don't. The workflow attests before it publishes, so
a failed attestation publishes nothing. If a release goes out without
attestations anyway, publish a new version rather than re-running the workflow.
