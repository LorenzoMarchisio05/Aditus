# Aditus

A lightweight, open-source desktop browser with a vertical tab sidebar and per-site account isolation.

> Status: feature-complete v1, early. Runs on Chromium (CEF). Tested on macOS only; Windows and Linux are untested (keyboard shortcuts are wired but unproven). See [PLAN.md](PLAN.md).

## Run

```sh
cd ui && npm install && cd ..
# needs Rust, ninja, a prebuilt CEF in $CEF_PATH and bundle-cef-app (see backend/bundle-mac.sh)
backend/bundle-mac.sh            # debug Aditus.app in backend/target/bundle
backend/bundle-mac.sh release    # optimised
node scripts/smoke.mjs backend/target/bundle/Aditus.app/Contents/MacOS/aditus
```

Unsigned builds: right-click the app and choose Open (or `xattr -cr Aditus.app`). Release builds ask for Keychain access once (Chromium cookie key); a signed build keeps the answer.

Shortcuts: Cmd/Ctrl+L address bar, Cmd/Ctrl+F find, Cmd/Ctrl+Shift+A search tabs.

## Layout

```
ui/        Svelte 5 + Vite front end: sidebar, address bar, palette, settings (src/), static pages (public/)
backend/   Rust: CEF engine, tabs, profiles, adblock, storage, menus (src/), bundling scripts, app icons and resources
scripts/   smoke test
```

The UI is built to `ui/dist` and served inside the app as `aditus://app/...`; the backend talks to it over `cefQuery` commands and state events.

## Why

Most browsers spend screen space and memory on things I don't use. Aditus aims for:

- A minimal UI with the tab bar on the side, collapsible to an icon strip.
- Small binary, fast startup, low memory.
- Different accounts per website in the same window. Example: Google Photos on account A, YouTube on account B, side by side, without separate windows.
- A built-in ad blocker. No extension system.

## How it works

- **Engine:** Chromium through [CEF](https://bitbucket.org/chromiumembedded/cef) (raw `cef-rs`, no Tauri). Same engine on every OS. The prebuilt CEF has no H.264/AAC and no Widevine DRM: VP9/Opus work, Netflix-style DRM and some video sites don't.
- **Profiles:** a profile is an isolated storage area (cookies, localStorage, logins). Every tab runs in one profile. You map hosts to profiles (`photos.google.com` → A, `youtube.com` → B). Navigating a tab to a host mapped to another profile swaps the tab to that profile's browser, with an undo toast. Redirects, login flows, and OAuth popups never swap.
- **Ad blocker:** [`adblock-rust`](https://github.com/brave/adblock-rust) with EasyList, EasyPrivacy, and uBlock filters, refreshed in the background. Per-site allowlist from the top bar.
- **Memory:** background tabs idle for 15 minutes (configurable) are discarded and reloaded on click. Pinned and audio-playing tabs are exempt.

## Features (v1)

Address bar with search (DuckDuckGo default, Google, custom), back/forward/reload, history, downloads, find in page, native DevTools, session restore, settings, tab pinning/grouping/search, toggleable top bar with a Cmd/Ctrl+L palette.

Not in v1: bookmarks, password manager, extensions, mobile, sync.

## Platforms

macOS 14+ (tested), Windows, Linux (untested).

## Stack

Rust, CEF 156 via `cef-rs` (Views framework), Svelte, SQLite for history and downloads, TOML for config.

## Performance (macOS, Apple silicon, measured once)

- Speedometer 3.1: 52.3 (Aditus debug build, ad blocker on) vs 50.7 (Chrome 155).
- 10 loaded tabs, summed RSS: ~4.2 GB / 23 processes vs Chrome ~4.7 GB / 27 processes.
- Window up in ~135-260 ms warm (release).

## Known limits

- Ad blocking is good, not uBlock Origin parity.
- DevTools open in a separate window (docking crashes CEF).
- No H.264/AAC, no Widevine (needs a custom CEF build).
- WebKit-era profile data is not migrated.

## License

GPL-3.0. See [LICENSE](LICENSE).
