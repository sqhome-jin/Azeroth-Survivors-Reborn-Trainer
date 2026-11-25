# Trainer releases

The stable trainer is a macOS Apple Silicon app. The version lives in
`Cargo.toml` and must match `Cargo.lock`. For routine releases, increment the
patch from 0 through 9. After patch 9, increment the minor and reset the patch
to 0, for example `1.0.9` → `1.1.0`.

Keep the Rust backend, SwiftUI host, tests, assets, and build workflow in the
repository. GitHub Actions builds, tests, and publishes a single signed Apple
Silicon zip with the updater-compatible name
`Azeroth.Survivor.Reborn.Trainer.macos-arm64.zip`. The app checks GitHub for
newer releases and opens the download page; it does not replace itself in
place. Run builds and tests in GitHub Actions. Local formatting and lockfile
generation are fine.

When a requested change increments the version, verify the branch build,
commit the related changes, and push to `origin/main`. Verify that the matching
GitHub release and its single zip asset are available.

Preserve unrelated local changes. Keep saves, settings, diagnostics, caches,
and built executables out of commits.
