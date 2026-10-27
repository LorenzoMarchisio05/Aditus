# Aditus audit

Scope: all Rust in `backend/src`, build/CI scripts, `Cargo.toml`, README. UI skimmed only for `@html`/`innerHTML` (none found). Not read: `App.svelte`, `Panel.svelte`, `Palette`, `Suggest`, `Mascot`, second half of `menu.rs`. Nothing was run or tested. Items marked **(verify)** are inferences from code, not confirmed behavior.

## A. Can lose data or break the app (fix first)

1. **A typo in `config.toml` wipes the config.** `lib.rs:72-73` calls `Config::load`, then immediately `cfg.save`.
   - Any parse error, or a missing required field (`search_template`, `idle_minutes`, `adblock`...), returns `Config::default()` with a new random profile UUID.
   - The broken file is then overwritten. Profiles, mappings and permissions are gone; old cookie stores are orphaned. Undercuts the "hand-editable TOML" feature.
   - Fix: on parse failure, back up the file, keep it, show a toast.
2. **Writes are not atomic.** `config.save`, `save_session` and `extensions/state.json` use plain `fs::write`. A crash mid-write corrupts them, and `panic = "abort"` makes a crash more likely. A corrupt session file silently becomes "no tabs". `save_session` also runs on every `emit_state`, on the UI thread.
3. **Updater rollback can delete the installed app.** Script at `update.rs:103-104`: `mv "$2" "$2.old" && … || { rm -rf "$2"; mv "$2.old" "$2"; … }`.
   - If the first `mv` fails (permissions, busy), the `||` branch deletes the current app, and there is no `.old` to restore.
   - Temp dir also leaks on every failed install.
4. **`panic = "abort"` plus `unwrap()` means one panic kills every tab.** Many `lock().unwrap()`, and `prepare(...).unwrap()` / `query_map(...).unwrap()` in `db.rs`. A locked, full or corrupt DB aborts the whole browser. Corrupt DB at startup means the app won't launch (`Db::open` error), no recovery path.
5. **Deleting a profile leaves its cookies and storage on disk** (`commands.rs:202-213` edits config only). Privacy leak, orphaned data.
6. **Profiles keyed by name, not id**, in mappings, tabs, history and session. A hand-edit rename silently breaks mappings and restore.

## B. Security and privacy

7. **Update integrity rests on GitHub alone.**
   - sha256 comes from the same API as the zip.
   - For ad-hoc/unsigned builds, `codesign --verify` only checks the bundle is self-consistent; the bundle-id comparison is trivially forgeable. Whoever controls the release gets code execution on every install.
   - Updater strips quarantine to get around Gatekeeper.
   - Once there is a Developer ID, pin the Team ID in the updater.
8. **Engine is a prebuilt binary from a third party.** `fetch-cef.sh` pulls `obs-rust/cef`, built "on a contributor's Mac", pinned by hash. Chromium security patches arrive only when that person rebuilds. The `cef-bump` PR does not run CI, and bumps the crate while the binary stays old. Biggest long-term risk for a browser.
9. **Hostile pages can forge "user gestures".** `cefQuery` exists in every page context and `page_ping` is allowed for untrusted pages. The `isTrusted` check lives only in the injected JS. A page can call `cefQuery` with `{"cmd":"page_ping","args":{"kind":"g"}}` directly. That:
   - defeats gesture-based popup and tab-under blocking (`GESTURE_WINDOW`);
   - allows fake back/forward (`b`/`f`) pings;
   - allows spam of fake "save password?" offers (`page_login`, own origin only).
   - Also, `window.cefQuery` and `window.__aditus` make every Aditus user fingerprintable.
10. **Chrome extensions: CRX signature not verified.** `split_crx3` checks the key hashes to the id, but never verifies the signature over the zip. Only TLS to Google protects it.
    - `unzip` limits total size by declared `f.size()`, which a hostile zip can understate.
    - Extensions are shared across all profiles, breaking profile isolation. MV3 only (uBlock Origin is MV2, excluded).
    - Silent auto-update when permissions are a subset.
11. **Saved passwords break profile isolation.** Keychain entries keyed by `origin + user`, not profile.
    - `login_fill` takes `.next()`, so no choice between multiple accounts on one origin.
    - Fill works in the top frame only.
    - Keychain reads/writes run synchronously on the UI thread; a large CSV import freezes the UI.
12. **Private-window leaks.**
    - `on_download` (`tabs.rs:716`) is unconditional, so private downloads (URL and path) are stored in SQLite.
    - Permissions keyed by host only, shared across profiles, never expire.
    - `clear_history_on_quit` clears the DB only. `session.json` still holds every open tab's URL and title; cookies, cache and downloads untouched.
13. **UI origin has no CSP.** `aditus://app` pages get the full command set (install_update, passwords import, reveal_path...). No `@html` found, so no known XSS today. A `Content-Security-Policy` header in `scheme.rs` is cheap defense in depth.
14. **Release entitlements.** `disable-library-validation` and `allow-unsigned-executable-memory` weaken hardened runtime. Check which are actually needed.
15. **Env vars still work in release.** `ADITUS_DIST`, `ADITUS_NETLOG` (captures "Everything") and `NOFLAGS` are not gated to debug builds, unlike `ADITUS_FLAGS` and `ADITUS_DATA_DIR`.
16. **Search suggestions on by default** (`search_suggestions: yes`). Every keystroke goes to DDG or Google. Off in private windows, but a privacy-minded user would expect opt-in.

## C. Architecture tradeoffs

17. **Own history stack replaces Chromium's.** `entries`/`idx` plus `go_history` navigate by `load_url`.
    - Back = full reload: no bfcache, no native scroll/form restore.
    - POST pages come back as GET.
    - Stack drifts on redirects, `location.replace`, iframes.
    - Not persisted, so back-history is lost on quit.
    - Reason: per-tab profile swapping. Core design tension.
18. **Popups become separate tabs, `window.opener` lost.** Reason: "never a native popup window". OAuth and payment popups using `postMessage` back to the opener (Sign in with Google, 3DS, Stripe) will likely break. `AUTH_HOSTS` is a hard-coded patch for this class.
19. **Profile swap is destroy-and-recreate.** Cancels the navigation and rebuilds the view; POST body, opener and in-page state are lost.
20. **Many views per window.** Palette, suggestion list and panels are each a full BrowserView. The suggestion browser is created and destroyed per typing session; heavy.
21. **Timing hacks instead of readiness signals.**
    - `defer_after` 250ms (palette-open), 60/400ms (suggest focus), 120ms (traffic lights), 50ms (hijack ping race).
    - Bounds "nudge" `(1.0,0.0)->(0.0,0.0)`.
    - DevTools "parked" forever (`PARK` leaks a browser per docked DevTools).
    - CEF-version-dependent workarounds that will fail on slow machines.
22. **Alloy runtime for pages.** Why the extension API is partial; CEF is believed to plan deprecating it **(verify)**.
23. **State broadcast.** Every `emit_state` serializes the whole config and all tabs per window and `eval`s it into every app view. Fires on each title, load and memory change, and runs SQLite queries under the global lock. Won't scale to hundreds of tabs.
24. **Registry duplication.** Same fact in `PAGES` (global Mutex) and `REG` (thread-local). "Never hold the lock across an engine call" enforced only by discipline.
25. **`ps -axo` forked every 10s on the UI thread**, forever, even idle (battery wakeups). Doesn't exist on Windows (silently returns 0). Memory limit sleeps one tab per tick, so converges slowly.
26. **`audible` inferred from `<video>` play/pause events from any frame**, not real audio state. Muted autoplay keeps tabs awake; an iframe pause can flip state. Idle sleep loses all JS state; only scroll, forms and media position restored.

## D. Ad blocker and YouTube

27. **Likely missing generic cosmetic filtering (verify).** Page script only uses `url_cosmetic_resources().hide_selectors`. Generic `##.ad` rules likely need `hidden_class_id_selectors(classes, ids)` called from the page. Scriptlets (`injected_script`) ignored too, which is why YouTube needed a hand-written script.
28. **YouTube script is fragile.** Patches `JSON.parse`, `JSON.stringify` and `Response.json` globally and relies on YouTube's internal field names. The `ponytail:` comment says so.
29. **`save_settings` runs `adblock::rebuild` synchronously on the UI thread** (`commands.rs:527`). Rebuilds ~200k rules; multi-second freeze when toggling a list.
30. **Allowlist is exact-host.** `example.com` does not cover `www.example.com`. Toggling reloads only the focused tab.
31. **Hijack heuristic.** `is_hijack` (third-party vs link-site) can misfire on legitimate cross-site flows.

## E. UX and correctness gaps

32. **Permission flow.** Request answered "deny" immediately, sidebar card asks later. Site sees `NotAllowedError`; "allow" reloads the page, killing an in-progress call. `perm_prompt` is a single global slot, so a second request overwrites the first.
33. **LAN addresses get `https`.** In `search::resolve` only `localhost` and `127.` get `http`. `192.168.1.1` and `10.x` become `https://...` and fail with no http fallback. Single-word intranet hosts become searches.
34. **No auth or certificate handlers (verify).** No `get_auth_credentials` or `on_certificate_error` seen. HTTP basic auth (routers, NAS) and self-signed local services probably unreachable.
35. **No download hardening.** `can_download` always 1 (drive-by downloads), no warning, `open_download` opens anything. Same-URL downloads matched by `MAX(id)`; stale `active` rows survive a crash. Whether files are quarantined: **(verify)**.
36. **Closing a window doesn't wait for `beforeunload`.** `can_close` fires `try_close_browser` and returns 1 at once. No "N tabs / active downloads" confirmation.
37. **History.** `LIKE '%q%'` full-scans an unbounded table on the UI thread and returns duplicate URLs; `_` not escaped. Default `history_days` is 0, never pruned. Re-importing Chrome history duplicates 5000 rows. No DB schema versioning. Importer described as supporting Brave, Edge etc., but path handling is Chrome-only.
38. **Failures are silent.** `let _ =` on nearly every I/O call and no log file. A GUI `.app` gives nowhere to see errors.

## F. Build, CI and tests

39. **Docs say Windows and Linux "build in CI". They don't.** `ci.yml` has one macOS job; `fetch-cef.sh` is macOS-arm64 only. Those code paths are uncompiled and will rot. `cef-port` still in push triggers.
40. **`bundle-mac.sh` hardcodes `-v 0.1.2`** while the crate is 0.1.4; bundle version string is stale. (`ui/package.json` says 0.1.0.)
41. **Release workflow runs no tests.** No `cargo test` or smoke test before publishing a tag; no clippy or fmt anywhere. Signing path documented as untested.
42. **Test coverage.** ~25 unit tests, all on pure modules. `tabs.rs` (1.6k lines, the state machine), `engine.rs` and `commands.rs` have zero. Smoke test only checks the UI loads and the page script is injected. One `scheme.rs` assertion is a tautology (`is_none() || starts_with`).
43. **arm64-only release asset**; updater hardcodes the asset name.

## Suggested order

| # | Fix | Cost |
|---|---|---|
| 1 | Config backup-on-parse-fail; atomic writes for config, session, state (#1, #2) | small |
| 2 | Updater rollback bug and temp cleanup (#3) | small |
| 3 | Drop untrusted `page_ping` gesture/back/forward, or have the renderer prove gestures (#9) | medium |
| 4 | Private downloads, profile-dir delete, CSP header (#12, #5, #13) | small |
| 5 | `http` for LAN IPs; `ps` off the UI thread (#33, #25) | small |
| 6 | Adblock rebuild off-thread; check generic cosmetic filters (#29, #27) | small to medium |
| 7 | Release runs tests; fix version string; fix README claim (#39-#41) | small |

Larger design calls to decide before coding: native history vs own stack (#17), `window.opener` popups (#18), who builds CEF (#8).
