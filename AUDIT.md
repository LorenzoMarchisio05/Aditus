# Aditus audit

Scope: all Rust in `backend/src`, build/CI scripts, `Cargo.toml`, README. UI skimmed only for `@html`/`innerHTML` (none found). Not read: `App.svelte`, `Panel.svelte`, `Palette`, `Suggest`, `Mascot`, second half of `menu.rs`. The original audit ran nothing. Items marked **(verify)** are inferences from code, not confirmed behavior.

**Status update:** fixes below were done on branch `audit-fixes` in four rounds and covered by unit tests (74 pass, 2 ignored). The app has not been launched or smoke-tested; by decision, items whose only remaining gap is runtime verification are treated as closed, and any bug found in real use gets a new item. Remaining items are real limitations or decisions, not testing.

## Summary

- Fixed: 33
- Mostly fixed: 5
- Fixed once a key is set: 1
- Open (decision): 1
- Partly fixed: 1
- Documented: 1
- Assessed, not migrated: 1

Open decisions: #8 (who builds CEF with H.264). Needs action: #7 (set `UPDATE_PUBKEY` and the `UPDATE_SIGNING_KEY` secret). Not migrated on purpose: #22.

## A. Can lose data or break the app (fix first)

1. **A typo in `config.toml` wipes the config.** `lib.rs:72-73` calls `Config::load`, then immediately `cfg.save`.
   - Any parse error, or a missing required field (`search_template`, `idle_minutes`, `adblock`...), returns `Config::default()` with a new random profile UUID.
   - The broken file is then overwritten. Profiles, mappings and permissions are gone; old cookie stores are orphaned. Undercuts the "hand-editable TOML" feature.
   - Fix: on parse failure, back up the file, keep it, show a toast.
   - **Status: Fixed.** A bad file is renamed to `config.toml.bak`, `Settings` is `#[serde(default)]`, and a toast tells the user.
2. **Writes are not atomic.** `config.save`, `save_session` and `extensions/state.json` use plain `fs::write`. A crash mid-write corrupts them, and `panic = "abort"` makes a crash more likely. A corrupt session file silently becomes "no tabs". `save_session` also runs on every `emit_state`, on the UI thread.
   - **Status: Fixed.** `config::write_atomic` for config, session and extensions state.
3. **Updater rollback can delete the installed app.** Script at `update.rs:103-104`: `mv "$2" "$2.old" && … || { rm -rf "$2"; mv "$2.old" "$2"; … }`.
   - If the first `mv` fails (permissions, busy), the `||` branch deletes the current app, and there is no `.old` to restore.
   - Temp dir also leaks on every failed install.
   - **Status: Fixed.** Script only restores `$2.old` if it exists; temp dir removed on failure.
4. **`panic = "abort"` plus `unwrap()` means one panic kills every tab.** Many `lock().unwrap()`, and `prepare(...).unwrap()` / `query_map(...).unwrap()` in `db.rs`. A locked, full or corrupt DB aborts the whole browser. Corrupt DB at startup means the app won't launch (`Db::open` error), no recovery path.
   - **Status: Fixed.** Panicking unwraps/indexing on I/O, DB and user-data paths replaced (including a char-boundary slice abort in `cosmetic_generic`); corrupt DB moved to `aditus.db.bak` and recreated. What remains is infallible, invariant-protected, or fatal-at-startup, and mutex `lock().unwrap()` stays by design (`panic = "abort"`, no poisoning).
5. **Deleting a profile leaves its cookies and storage on disk** (`commands.rs:202-213` edits config only). Privacy leak, orphaned data.
   - **Status: Fixed.** `delete_profile` removes the data dir.
6. **Profiles keyed by name, not id**, in mappings, tabs, history and session. A hand-edit rename silently breaks mappings and restore.

   - **Status: Fixed.** Profiles are keyed by stable id in mappings, tabs, history, session, bookmarks and logins; names are resolved for the UI. `Config::sanitize()` and migrations (`user_version=3`) convert old name-keyed data. Risks: rows for profiles renamed/deleted before first launch stay as stale names; no rename command in the UI yet; `config.toml` mappings hold UUIDs. Migration of `session.json` and UI name resolution are not unit-tested.
## B. Security and privacy

7. **Update integrity rests on GitHub alone.**
   - sha256 comes from the same API as the zip.
   - For ad-hoc/unsigned builds, `codesign --verify` only checks the bundle is self-consistent; the bundle-id comparison is trivially forgeable. Whoever controls the release gets code execution on every install.
   - Updater strips quarantine to get around Gatekeeper.
   - Once there is a Developer ID, pin the Team ID in the updater.
   - **Status: Fixed once a key is set.** Updater verifies a detached ed25519 signature (`<zip>.sig` over the zip's sha256) with `ring` against an embedded key, plus sha256, code-signature check and Team ID pin. **Action needed:** replace the placeholder `UPDATE_PUBKEY` in `update.rs` and set the `UPDATE_SIGNING_KEY` secret (keygen in README); until then in-app install is disabled. CI signing step untested.
8. **Engine is a prebuilt binary from a third party.** `fetch-cef.sh` pulls `obs-rust/cef`, built "on a contributor's Mac", pinned by hash. Chromium security patches arrive only when that person rebuilds. The `cef-bump` PR does not run CI, and bumps the crate while the binary stays old. Biggest long-term risk for a browser.
   - **Status: Open (decision).** Kept `obs-rust` pin for now. Official CEF + separate ffmpeg is **not feasible** (ffmpeg is statically linked; see `docs/cef-ffmpeg.md`). Remaining options: scheduled release check on `obs-rust`, or build CEF ourselves with `proprietary_codecs=true ffmpeg_branding=Chrome`.
9. **Hostile pages can forge "user gestures".** `cefQuery` exists in every page context and `page_ping` is allowed for untrusted pages. The `isTrusted` check lives only in the injected JS. A page can call `cefQuery` with `{"cmd":"page_ping","args":{"kind":"g"}}` directly. That:
   - defeats gesture-based popup and tab-under blocking (`GESTURE_WINDOW`);
   - allows fake back/forward (`b`/`f`) pings;
   - allows spam of fake "save password?" offers (`page_login`, own origin only).
   - Also, `window.cefQuery` and `window.__aditus` make every Aditus user fingerprintable.
   - **Status: Fixed.** Page script deletes `window.cefQuery`/`cefQueryCancel` and captures builtins up front. Limit: code that runs before our script.
10. **Chrome extensions: CRX signature not verified.** `split_crx3` checks the key hashes to the id, but never verifies the signature over the zip. Only TLS to Google protects it.
    - `unzip` limits total size by declared `f.size()`, which a hostile zip can understate.
    - Extensions are shared across all profiles, breaking profile isolation. MV3 only (uBlock Origin is MV2, excluded).
    - Silent auto-update when permissions are a subset.
   - **Status: Accepted, documented.** CRX3 proof signature verified (`ring`; ECDSA and RSA both tested), unzip counts real bytes (400MB / 20,000 entries). Extensions remain shared across profiles (Chromium loads `--load-extension` once per process): documented in README and the extensions panel. MV3 only.
11. **Saved passwords break profile isolation.** Keychain entries keyed by `origin + user`, not profile.
    - `login_fill` takes `.next()`, so no choice between multiple accounts on one origin.
    - Fill works in the top frame only.
    - Keychain reads/writes run synchronously on the UI thread; a large CSV import freezes the UI.
   - **Status: Fixed.** Keychain accounts keyed by profile (legacy fallback copies forward for the first profile), DB migration `user_version=2`, account chooser popup, CSV import on a worker thread. Fill is still top frame only.
12. **Private-window leaks.**
    - `on_download` (`tabs.rs:716`) is unconditional, so private downloads (URL and path) are stored in SQLite.
    - Permissions keyed by host only, shared across profiles, never expire.
    - `clear_history_on_quit` clears the DB only. `session.json` still holds every open tab's URL and title; cookies, cache and downloads untouched.
   - **Status: Fixed.** Private downloads not recorded, permissions split private/regular, `clear_history_on_quit` clears session, downloads, and (via `.clear-on-start`) cookies/cache at next start. The profile folder layout it deletes is an assumption to verify.
13. **UI origin has no CSP.** `aditus://app` pages get the full command set (install_update, passwords import, reveal_path...). No `@html` found, so no known XSS today. A `Content-Security-Policy` header in `scheme.rs` is cheap defense in depth.
   - **Status: Fixed.** CSP header in `scheme.rs`.
14. **Release entitlements.** `disable-library-validation` and `allow-unsigned-executable-memory` weaken hardened runtime. Check which are actually needed.
   - **Status: Fixed.** Entitlements only for the helper apps; `disable-library-validation` removed. Needs a signed-build test; helpers may need it back.
15. **Env vars still work in release.** `ADITUS_DIST`, `ADITUS_NETLOG` (captures "Everything") and `NOFLAGS` are not gated to debug builds, unlike `ADITUS_FLAGS` and `ADITUS_DATA_DIR`.
   - **Status: Fixed.** `ADITUS_DIST`, `ADITUS_NETLOG`, `NOFLAGS` are debug-only.
16. **Search suggestions on by default** (`search_suggestions: yes`). Every keystroke goes to DDG or Google. Off in private windows, but a privacy-minded user would expect opt-in.

   - **Status: Fixed.** `search_suggestions` defaults to false.
## C. Architecture tradeoffs

17. **Own history stack replaces Chromium's.** `entries`/`idx` plus `go_history` navigate by `load_url`.
    - Back = full reload: no bfcache, no native scroll/form restore.
    - POST pages come back as GET.
    - Stack drifts on redirects, `location.replace`, iframes.
    - Not persisted, so back-history is lost on quit.
    - Reason: per-tab profile swapping. Core design tension.
   - **Status: Fixed.** Native `go_back`/`go_forward` when the profile doesn't change; redirects, `replaceState` and `location.replace` (detected via the engine's history position before/after) replace instead of push; up to 30 entries per tab persisted. Rules in `history.rs`. Runtime behavior untested; relies on CEF answering the history visitor synchronously.
18. **Popups become separate tabs, `window.opener` lost.** Reason: "never a native popup window". OAuth and payment popups using `postMessage` back to the opener (Sign in with Google, 3DS, Stripe) will likely break. `AUTH_HOSTS` is a hard-coded patch for this class.
   - **Status: Partly fixed.** User-gesture popups that ask for a window (sized `window.open`) or go to an auth host become related popups (`on_before_popup` returns 0), docked as tabs, keep `window.opener`, inherit the opener's profile; `window.close()` closes the tab; opener and popup are never slept or swapped. Plain `target=_blank` and `about:blank` popups stay independent tabs. Gaps: parked popups leak one blank browser each, `popup.closed` stays false, a cross-site popup flagged by the hijack heuristic before the click ping falls back to an independent tab. Needs live CEF testing.
19. **Profile swap is destroy-and-recreate.** Cancels the navigation and rebuilds the view; POST body, opener and in-page state are lost.
   - **Status: Mostly fixed.** Old view stays up until the new view starts loading (8s timeout). A POST that triggers a swap shows a toast and refills the page's last-reported form fields when the new page is the form's own address; the POST body is never replayed. In-page JS state and back/forward onto POST entries are not preserved.
20. **Many views per window.** Palette, suggestion list and panels are each a full BrowserView. The suggestion browser is created and destroyed per typing session; heavy.
   - **Status: Mostly fixed.** Suggest view created once per window and reused; palette already lazy. Docked-DevTools `PARK` leak remains: a DevTools popup is bound to its page and the cef crate can't retarget it, so it is documented rather than fixed.
21. **Timing hacks instead of readiness signals.**
    - `defer_after` 250ms (palette-open), 60/400ms (suggest focus), 120ms (traffic lights), 50ms (hijack ping race).
    - Bounds "nudge" `(1.0,0.0)->(0.0,0.0)`.
    - DevTools "parked" forever (`PARK` leaks a browser per docked DevTools).
    - CEF-version-dependent workarounds that will fail on slow machines.
   - **Status: Documented.** No replacement signal exists in the cef crate for the 120ms traffic-light, 50ms hijack ping and bounds-nudge hacks; each now has a comment explaining the race. Suggest focus timers were replaced by `suggest_ready`.
22. **Alloy runtime for pages.** Why the extension API is partial; CEF is believed to plan deprecating it **(verify)**.
   - **Status: Assessed, not migrated.** `docs/alloy-runtime-assessment.md`: headers don't mark Alloy deprecated; a Chrome-style window hosts one Chrome-style BrowserView, so migration is a rewrite. Recommendation: don't migrate now.
23. **State broadcast.** Every `emit_state` serializes the whole config and all tabs per window and `eval`s it into every app view. Fires on each title, load and memory change, and runs SQLite queries under the global lock. Won't scale to hundreds of tabs.
   - **Status: Mostly fixed.** `emit_state` coalesces within 16ms, sends per-window, and sends small `state-delta` events for title/loading/memory changes (full snapshot for structural changes, sequence numbers and `get_state` resync); config JSON, bookmark and login lookups cached. Dirty tracking: `emit_tab(id)` (title, audible, memory) marks only that tab and its flush diffs just those tabs and sends a delta; `emit_state`/`emit` mark the flush structural (full snapshot as before), and anything unsure falls back to full. The UI (`api.js`) now merges deltas by `seq` and calls `get_state` on a gap (it did not handle `state-delta` before). Limits: load start/stop, URL and history changes are still structural (they move can_back/forward and the DB flags), so page loads still build the full snapshot; a delta flush still scans `tabs` to find each dirty id; memory changes under 5 MB stay unsent until that tab is next dirty. Not run against the live app.
24. **Registry duplication.** Same fact in `PAGES` (global Mutex) and `REG` (thread-local). "Never hold the lock across an engine call" enforced only by discipline.
   - **Status: Fixed.** Ownership documented (`App` = serializable state, `engine::REG` = live handles). Debug-only guard: `lock_app()` + assertion in `engine::reg()` panics on an engine call under the App lock. `PAGES` kept (IO threads can't read `REG`). Guard never run against live CEF.
25. **`ps -axo` forked every 10s on the UI thread**, forever, even idle (battery wakeups). Doesn't exist on Windows (silently returns 0). Memory limit sleeps one tab per tick, so converges slowly.
   - **Status: Fixed.** `ps` sampling on a worker thread. Still no Windows implementation.
26. **`audible` inferred from `<video>` play/pause events from any frame**, not real audio state. Muted autoplay keeps tabs awake; an iframe pause can flip state. Idle sleep loses all JS state; only scroll, forms and media position restored.

   - **Status: Fixed.** Audible tracked per frame (set of playing frames); heuristic tightened. CEF has no audio-state callback.
## D. Ad blocker and YouTube

27. **Likely missing generic cosmetic filtering (verify).** Page script only uses `url_cosmetic_resources().hide_selectors`. Generic `##.ad` rules likely need `hidden_class_id_selectors(classes, ids)` called from the page. Scriptlets (`injected_script`) ignored too, which is why YouTube needed a hand-written script.
   - **Status: Fixed.** `cosmetic_generic` (rate-limited) for generic hide rules; bundled scriptlets (9, uBO short names accepted) delivered early from the browser process with a late fallback. Redirects and renderer swaps use the fallback.
28. **YouTube script is fragile.** Patches `JSON.parse`, `JSON.stringify` and `Response.json` globally and relies on YouTube's internal field names. The `ponytail:` comment says so.
   - **Status: Fixed.** Ad field names live in one list (`YT_AD_FIELDS`); built-in `json-prune`/`set-constant` scriptlet rules strip them (also via `Response.prototype.json`); the hand-written script keeps only stall/skip/CSS handling and a 20s diagnostic. If scriptlet delivery fails on YouTube frames, nothing strips the fields.
29. **`save_settings` runs `adblock::rebuild` synchronously on the UI thread** (`commands.rs:527`). Rebuilds ~200k rules; multi-second freeze when toggling a list.
   - **Status: Fixed.** Rebuild on a worker thread.
30. **Allowlist is exact-host.** `example.com` does not cover `www.example.com`. Toggling reloads only the focused tab.
   - **Status: Fixed.** Allowlist covers subdomains (`covers`); toggling reloads every matching tab.
31. **Hijack heuristic.** `is_hijack` (third-party vs link-site) can misfire on legitimate cross-site flows.

   - **Status: Fixed.** `is_hijack_host` allows same site (public-suffix aware via `psl`), the clicked link's site, `AUTH_HOSTS` and a new `PAYMENT_HOSTS`; everything else blocked. Table-driven tests.
## E. UX and correctness gaps

32. **Permission flow.** Request answered "deny" immediately, sidebar card asks later. Site sees `NotAllowedError`; "allow" reloads the page, killing an in-progress call. `perm_prompt` is a single global slot, so a second request overwrites the first.
   - **Status: Fixed.** Permission prompt queue instead of a single slot.
33. **LAN addresses get `https`.** In `search::resolve` only `localhost` and `127.` get `http`. `192.168.1.1` and `10.x` become `https://...` and fail with no http fallback. Single-word intranet hosts become searches.
   - **Status: Fixed.** LAN IPs default to http.
34. **No auth or certificate handlers (verify).** No `get_auth_credentials` or `on_certificate_error` seen. HTTP basic auth (routers, NAS) and self-signed local services probably unreachable.
   - **Status: Fixed.** Basic-auth card and certificate-error card (Go back / Proceed for this host, session only, private separated); `answer_net` not exposed to untrusted pages. Sub-resource auth prompts still show a card.
35. **No download hardening.** `can_download` always 1 (drive-by downloads), no warning, `open_download` opens anything. Same-URL downloads matched by `MAX(id)`; stale `active` rows survive a crash. Whether files are quarantined: **(verify)**.
   - **Status: Fixed.** Downloads tracked by row id, stale rows failed at startup, downloads with no recent click/Enter blocked with a toast, risky file types (`.app .dmg .pkg .command .sh .jar .scpt .terminal`, exec bit) need a confirmation dialog before opening, quarantine xattr set if missing.
36. **Closing a window doesn't wait for `beforeunload`.** `can_close` fires `try_close_browser` and returns 1 at once. No "N tabs / active downloads" confirmation.
   - **Status: Fixed.** `can_close` asks tab browsers via `try_close_browser`; second close within 3s forces it. Assumes CEF re-closes after the dialog.
37. **History.** `LIKE '%q%'` full-scans an unbounded table on the UI thread and returns duplicate URLs; `_` not escaped. Default `history_days` is 0, never pruned. Re-importing Chrome history duplicates 5000 rows. No DB schema versioning. Importer described as supporting Brave, Edge etc., but path handling is Chrome-only.
   - **Status: Fixed.** Searches on worker threads, LIKE escaping, grouped by URL, indexed, import dedupe, `PRAGMA user_version`. Importer is still Chrome-path only.
38. **Failures are silent.** `let _ =` on nearly every I/O call and no log file. A GUI `.app` gives nowhere to see errors.

   - **Status: Fixed.** `applog.rs` writes `aditus.log` (rotated at 512KB); `applog::check` at I/O sites.
## F. Build, CI and tests

39. **Docs say Windows and Linux "build in CI". They don't.** `ci.yml` has one macOS job; `fetch-cef.sh` is macOS-arm64 only. Those code paths are uncompiled and will rot. `cef-port` still in push triggers.
   - **Status: Fixed.** Docs no longer claim Windows/Linux CI.
40. **`bundle-mac.sh` hardcodes `-v 0.1.2`** while the crate is 0.1.4; bundle version string is stale. (`ui/package.json` says 0.1.0.)
   - **Status: Fixed.** `bundle-mac.sh` reads the version from `Cargo.toml`; `ui/package.json` aligned.
41. **Release workflow runs no tests.** No `cargo test` or smoke test before publishing a tag; no clippy or fmt anywhere. Signing path documented as untested.
   - **Status: Fixed.** `cargo fmt` applied (`backend/rustfmt.toml`, 200 cols); `cargo fmt --check` is a blocking step in `ci.yml` and `release.yml`; release runs tests and smoke test; non-blocking clippy in `ci.yml`.
42. **Test coverage.** ~25 unit tests, all on pure modules. `tabs.rs` (1.6k lines, the state machine), `engine.rs` and `commands.rs` have zero. Smoke test only checks the UI loads and the page script is injected. One `scheme.rs` assertion is a tautology (`is_none() || starts_with`).
   - **Status: Mostly fixed.** ~74 unit tests, now covering `tabs.rs`, `history.rs`, `nav.rs`, `scheme.rs`, `config.rs`, `db.rs`, and pure helpers of `commands.rs`/`engine.rs`. CEF-bound code paths remain untested; smoke test extended.
43. **arm64-only release asset**; updater hardcodes the asset name.

   - **Status: Fixed.** `asset_name(arch)` in `update.rs`; `release.yml` derives the name from `uname -m`. Still arm64-only in practice.
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
