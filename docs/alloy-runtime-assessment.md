# #22 assessment: Alloy runtime for pages

Status: research only, nothing migrated. Sources: CEF 156 headers in `~/.local/share/cef/include`
(`internal/cef_types_runtime.h`, `cef_browser.h`, `cef_client.h`, `cef_life_span_handler.h`,
`cef_permission_handler.h`, `views/*`) and the `cef` 156.1.0 crate.

## Where Aditus uses which style (verified in `engine.rs`)

- Window: `window_runtime_style` returns CHROME.
- UI view, tab pages, overlays: `browser_runtime_style` returns ALLOY (CHROME only for DevTools popups).
- Extensions: `--load-extension` switch (`app.rs:147`). The "partial extension API" in the audit is
  what Alloy offers; Aditus cannot fill it in itself.

## Is Alloy deprecated?

Not provable from what ships here. The 156 headers do not mark `CEF_RUNTIME_STYLE_ALLOY` or any
Alloy-only callback deprecated. The enum doc still presents Alloy as supported ("adds additional
client callbacks and support for windowless rendering") and says windowless rendering always uses
Alloy. The headers do show the direction of travel: newer APIs are annotated "Chrome style only"
(`OnChromeCommand`, `GetChromeToolbar`, `chrome_app_icon_id`, `use_views_default_popup`, policy
dir, status bubble), and Alloy defaults are the weaker ones (downloads cancelled without a handler,
permission prompts denied or ignored by default). Any actual deprecation plan lives in the CEF
forum and issue tracker, which I could not check offline. Treat "Alloy eventually goes" as
plausible with no date and no header-level warning in 156. Re-check at each CEF bump by grepping
the headers for `deprecated` near `ALLOY`.

## The blocker

`cef_types_runtime.h`: "a Chrome style Window can host at most one Chrome style BrowserView but
potentially multiple Alloy style BrowserViews." Aditus has one Window holding the UI view, one
BrowserView per tab (moved by bounds in `HOST`) and overlay views. Making tab pages Chrome style
would allow only one of them per window. Flipping `browser_runtime_style` is therefore not a
migration; the second tab would break (exact failure mode unverified, needs a run, do not try blind).

## What changes under Chrome style (from header docs)

- Default handling flips: no download handler means downloads proceed (ours is set); permission
  prompts show Chrome UI unless handled (check our `OnShowPermissionPrompt`/media handlers).
- Close flow: `DoClose` is Alloy only; `close()` relies on dropping the view to destroy the browser.
- Popups: default is a Chrome UI popup unless `OnPopupBrowserViewCreated` handles it.
- Fullscreen exit is handled internally; our Esc/traffic-light handling must be rechecked.
- Chrome UI leaks in: accelerators, context menu (we have a custom one), find bar, status bubble,
  new-window commands (`OnChromeCommand`). Each needs suppression.
- Possible gain: `chrome.tabs` and tab-aware extension popups, since extensions need a Chrome tab
  model. CEF does not guarantee that, and the tab model would be Chrome's, not our sidebar's.
- `Focus::on_set_focus` (suggest view never takes focus) was written against Alloy behaviour.

## Plan

1. Decide the goal. If it is only "do not get stranded when Alloy goes", wait: nothing in 156 warns,
   and the per-tab BrowserView architecture cannot use Chrome style anyway.
2. Spike on a throwaway branch: a window with two Chrome BrowserViews, to record the failure mode.
3. If the limit holds, the only Chrome-style designs are one Chrome BrowserView per window using
   Chrome's tab model (drop our tab view, use `GetChromeToolbarType`) or one window per tab. Both
   are product rewrites, not a flag.
4. Keep UI and overlays Alloy (a Chrome window may host many Alloy views); only pages move.
5. Re-audit each handler above against Chrome defaults; gate behind a setting for A/B.
6. Re-test extensions (content scripts, DNR, service workers), the YouTube script and ad blocking.

Recommendation: do not migrate now. Re-read CEF release notes at each bump and revisit if Alloy gets
tagged deprecated.
