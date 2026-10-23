# Aditus

A lightweight, open-source desktop browser with a vertical tab sidebar and per-site account isolation.

> Status: feature-complete v1, early. Verified on macOS (headless self-test + UI snapshots). Windows and Linux build in CI and pass unit tests, but runtime behavior there is untested. See [PLAN.md](PLAN.md).

## Run

```sh
npm install
npm run tauri dev      # needs Rust, Tauri prerequisites
npm run tauri build    # unsigned bundle
```

Unsigned builds: on macOS right-click the app and choose Open (or `xattr -cr Aditus.app`); on Windows choose "More info" then "Run anyway" in SmartScreen.

Shortcuts: Cmd/Ctrl+L address bar, Cmd/Ctrl+F find, Cmd/Ctrl+Shift+A search tabs.

## Why

Most browsers spend screen space and memory on things I don't use. Aditus aims for:

- A minimal UI with the tab bar on the side, collapsible to an icon strip.
- Small binary, fast startup, low memory.
- Different accounts per website in the same window. Example: Google Photos on account A, YouTube on account B, side by side, without separate windows.
- A built-in ad blocker. No extension system.

## How it works

- **Engine:** Aditus does not ship its own. It uses each OS's webview through [Tauri](https://tauri.app): WebKit on macOS and Linux, WebView2 (Chromium) on Windows. Rendering can differ slightly per OS.
- **Profiles:** a profile is an isolated storage area (cookies, localStorage, logins). Every tab runs in one profile. You map hosts to profiles (`photos.google.com` → A, `youtube.com` → B). Navigating a tab to a host mapped to another profile swaps the tab to that profile's webview, with an undo toast. Redirects, login flows, and OAuth popups never swap.
- **Ad blocker:** [`adblock-rust`](https://github.com/brave/adblock-rust) with EasyList, EasyPrivacy, and uBlock filters, refreshed in the background. Per-site allowlist from the top bar.
- **Memory:** background tabs idle for 15 minutes (configurable) are discarded and reloaded on click. Pinned and audio-playing tabs are exempt.

## Features (v1)

Address bar with search (DuckDuckGo default, Google, custom), back/forward/reload, history, downloads, find in page, native DevTools, session restore, settings, tab pinning/grouping/search, toggleable top bar with a Cmd/Ctrl+L palette.

Not in v1: bookmarks, password manager, extensions, mobile, sync.

## Platforms

macOS 14+, Windows, Linux. macOS 14 is required for per-profile storage on WebKit.

## Stack

Rust, Tauri 2 (multi-webview, `unstable` feature, version pinned), Svelte, SQLite for history and downloads, TOML for config.

## Known limits

- Ad blocking is good, not uBlock Origin parity, especially on WebKit.
- DevTools differ per OS: Chrome DevTools on Windows, Safari Web Inspector on macOS, WebKit inspector on Linux.
- Engine versions follow the OS. Old Linux distros may ship an outdated WebKitGTK.

## License

GPL-3.0. See [LICENSE](LICENSE).
