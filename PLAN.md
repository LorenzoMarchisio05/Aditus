# Aditus build plan

Order: kill the riskiest unknowns first, then build features on proven foundations. Each phase ends with something that runs.

## Decisions (frozen)

- Desktop only: macOS 14+, Windows, Linux.
- Rust + CEF (Chromium) via raw `cef-rs`, Views framework. Ported from Tauri system webviews.
- UI: Svelte. Config: TOML. Bulk data: SQLite. License: GPL-3.0.
- No extensions. Engine sits behind `backend/src/engine.rs`.
- Profile = storage isolation. Mapping key = exact host, optional `*.` wildcard. Auth hosts exempt.
- Swap only on user-initiated top-level navigation. Silent, with undo toast.
- Ad blocker: `adblock-rust`, daily list refresh, per-site allowlist.
- Idle-tab discard: 15 min default, configurable, pinned and audio exempt.

## Phase 0: spikes (throwaway code)

Goal: prove the risky parts work before building around them.

1. **Multi-webview:** one window, a Svelte UI webview (sidebar + top bar) plus N page webviews positioned by Rust. Resize correctly. Pin the cef-rs version.
2. **Per-webview storage isolation** on all three OSes: log into the same site with two accounts in two webviews, confirm cookies don't leak. macOS: `WKWebsiteDataStore` by identifier. Windows: WebView2 profile or separate user-data folder. Linux: separate `WebKitWebsiteDataManager`.
3. **Navigation interception:** catch top-level navigations, distinguish user-initiated from redirect and JS-driven. Document what each engine exposes.
4. **Overlay palette:** draw a floating input above a page webview. Confirm focus and keyboard handling.
5. **Network block hook:** block one hardcoded host on each OS.

Exit: a written note per spike with what works per OS and any dealbreaker. If 1 or 2 fails on any OS, stop and reassess (drop to `wry`, or revisit CEF).

## Phase 1: browser core

- Tab model in Rust: tab slot = id, profile, history list, live webview or discarded state.
- Open, close, switch, reorder tabs. Sidebar in Svelte (collapsible, resizable).
- Top bar: URL, back, forward, reload, loading state. Toggle and Cmd/Ctrl+L palette.
- URL vs search parsing. DuckDuckGo default.
- Title and favicon updates.
- New-window requests (`target=_blank`, `window.open`) open as tabs; OAuth popups stay in the opener's profile.

Exit: usable single-profile browser.

## Phase 2: profiles

- Profile CRUD, per-profile storage directory/ID.
- Host→profile mapping store (TOML), exact host + wildcard, auth-host exempt list.
- First-visit prompt for unmapped hosts. Settings page to edit mappings.
- Profile swap in the same tab slot using the Phase 0 navigation rule. Undo toast. Profile badge on tab and top bar.
- App-level history per tab slot across swaps (back/forward spanning profiles).

Exit: Photos on A and YouTube on B in one window.

## Phase 3: ad blocker

- Integrate `adblock-rust`. Compile rules to per-platform form (WebKit content rule lists; WebView2 request interception).
- Cosmetic filtering via injected CSS/JS.
- Background list download and refresh (daily), cached on disk.
- Per-site allowlist, toggle in top bar.

Exit: measurable blocking on common ad-heavy sites on all three OSes.

## Phase 4: features

- History (SQLite) with search.
- Downloads manager (SQLite).
- Find in page.
- Native DevTools wired in (`devtools` Cargo feature for release builds).
- Session restore.
- Pinning, grouping, tab search.
- Settings: search engine (DDG, Google, custom template), idle timeout, sidebar behavior.

## Phase 5: memory

- Idle-tab discard: destroy webview, keep URL, title, favicon, history. Reload on click.
- Exempt pinned and audio-playing tabs. Configurable timeout, default 15 min.
- Measure RAM with 10, 30 tabs. Compare against Safari and Chrome. Record numbers in the README.

## Phase 6: release

- CI builds for macOS, Windows, Linux.
- Unsigned GitHub releases (`.dmg`, `.msi`, `.AppImage`).
- Document Gatekeeper and SmartScreen bypass in the README.
- Later: package managers (Homebrew, winget, AUR), signing if others adopt it.

## Risks to watch

1. Unstable multi-webview API breaking on upgrade. Pin the version.
2. Overlay palette over a native webview (focus, z-order).
3. Reliable user-initiated vs redirect detection across three engines.
4. Three platform backends for ad block and profiles behind one interface.
5. OS-dependent engine versions, especially old WebKitGTK.

## Testing

Keep it small: unit tests for the mapping matcher, URL/search parsing, and the swap decision function (pure Rust, no webview). Manual checklist per OS for webview behavior.
