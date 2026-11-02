# Aditus

## Releasing

Before tagging a release you MUST add a `## <version>` section to `CHANGELOG.md` (bullets start with `**New**`, `**Improved**` or `**Fix**`). The release workflow fails without it and the app shows these notes to users. Also bump `backend/Cargo.toml` to match the tag and run `cargo fmt` in `backend/`. See README "Releasing".
