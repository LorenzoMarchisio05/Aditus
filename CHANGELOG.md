# Changelog

One section per release, newest first. Each bullet starts with a bold tag: **New**, **Improved** or **Fix**.
The release workflow publishes the matching section as the GitHub release notes, and the app shows it.

## 0.1.17
- **New** The New tab button is now always at the top of the sidebar, and tabs are listed newest first.
- **Improved** The address bar autocompletes like other browsers: the top suggestion matches the completed address, Escape restores the page's address, and localhost shows as an address.
- **Improved** The sidebar is 250 px wide by default.
- **Fix** Much less flicker: the new tab page, suggestion list and palette no longer redraw whenever the window re-lays out, tab icons no longer blink while pages load, and the footer no longer jumps when the mascot comes and goes.
- **Fix** Typing in the address bar is no longer wiped when a single-page site changes its URL.

## 0.1.16
- **New** Settings > About has a Release notes button that opens the What's new page.

## 0.1.15
- **Improved** The "What's new" page is now a timeline of every release, grouped into New, Improved and Fix.

## 0.1.14
- **Fix** Pages that don't set their own background (parts of Apple's developer site, for example) showed dark text on a black canvas. Web pages now get the usual white.

## 0.1.13
- **Fix** The window opened empty, with no tabs, in 0.1.11.
- **Fix** Update checks no longer hammer GitHub, and still find new releases when GitHub's API rate limit is hit.

## 0.1.11
- **New** Release notes: a "What's new" page shows up after an update, and from the update notification.
- **Improved** Notifications (updates, permissions, sign-in, save password, undo) now appear bottom-right over the page, so they no longer cover the mascot.
- **Fix** The new-tab page no longer flashes when you open a tab.
- **Fix** Tabs now go to sleep after the idle time counted from when you leave them, not from when you opened them.

## 0.1.10
- **Fix** Navigation in private tabs.
- **Fix** Fullscreen video and pages.
- **Fix** Tabs restore correctly when you reopen the app.
- **Fix** The updater and extension checks.
- **Fix** Several small UI state bugs.
