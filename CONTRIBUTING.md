# Contributing to Aditus

Thanks for helping. Aditus is a small, daily-usable browser, so keep changes small and focused.

## Before you start

- **Bugs and small fixes:** open a PR directly.
- **New features or bigger changes:** open an issue first so we agree on the idea before you spend time on it. Aditus aims for a minimal UI and a small footprint; features that add weight or settings may be declined.
- **Not planned:** sync, mobile.

## Setup

You need Rust, Node 22, ninja, a prebuilt CEF in `$CEF_PATH` and `bundle-cef-app`. The README's [Run](README.md#run) section has the commands; CI (`.github/workflows/ci.yml`) shows the exact CEF version and install steps.

```sh
cd ui && npm install && cd ..
backend/bundle-mac.sh            # debug build: backend/target/bundle/Aditus.app
```

macOS 14+ is the only tested platform. Windows and Linux build in CI but are untested; reports and fixes for them are welcome.

## Layout

```
ui/        Svelte 5 + Vite front end (src/), static pages (public/)
backend/   Rust: CEF engine, tabs, profiles, adblock, storage, menus (src/)
scripts/   smoke test
```

The UI is built to `ui/dist` and served inside the app as `aditus://app/...`. It talks to the backend through `cefQuery` commands and state events. Rebuild the bundle to see UI changes in the app.

## Checks

Run these before opening a PR. CI runs the same.

```sh
cd backend && cargo test --lib
backend/bundle-mac.sh
node scripts/smoke.mjs backend/target/bundle/Aditus.app/Contents/MacOS/aditus
```

Add a unit test for new logic in the backend when it can be tested without CEF. For UI changes, run the app and try the change; say what you checked in the PR.

## Code style

- Match the surrounding code: naming, comment density, idiom. Run `cargo fmt` on Rust code.
- Prefer the standard library and existing dependencies. Justify any new dependency in the PR.
- Comments explain why, not what.
- No unrelated refactors or formatting churn in the same PR.

## Commits and pull requests

- Use [Conventional Commits](https://www.conventionalcommits.org/) as in the history: `feat(extensions): ...`, `fix(youtube): ...`, `docs: ...`. One line, lowercase, says what changed.
- One topic per PR. Describe what changed and why, and link the issue.
- Keep `README.md` accurate if you change behavior it describes (features, known limits).
- Chromium upgrades come from the weekly `cef-bump` workflow; don't bump the `cef` crate by hand.

## Reporting bugs

Include macOS/OS version, Aditus version, steps to reproduce, and what you expected. For site-specific problems, give the URL and say whether the ad blocker or an extension was on.

Security issues: don't file a public issue. Contact the maintainer privately through the email on their GitHub profile.

## License

By contributing you agree your work is licensed under [GPL-3.0](LICENSE).
