//! A tab's own back/forward list. Pure data and rules, no engine types, so it is unit-testable.
//!
//! Why we keep a list at all: a tab's profile can change between two history entries, and a
//! profile swap rebuilds the view, which starts with an empty native (Chromium) history. The list
//! lets us notice such steps. Everything else goes through the native history (bfcache, scroll and
//! form restore, POST re-submit), which is only valid inside the window `[base, top]`: the entries
//! the *current view's* native history lines up with. A step outside that window is a plain load.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub url: String,
    pub profile: String,
}

/// How a back/forward step must be carried out.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Nothing before/after the current entry.
    None,
    /// The view's native history holds the target: `go_back` / `go_forward`.
    Native,
    /// Load the target (a fresh view for another profile, or a plain `load_url`).
    Load(Entry),
}

#[derive(Default)]
pub struct Hist {
    pub entries: Vec<Entry>,
    pub idx: usize,
    /// Native history of the current view covers entries `base..=top` (empty when `base > top`).
    base: usize,
    top: usize,
}

/// Saved with the session: enough to bring back/forward back after a restart.
const SAVE_MAX: usize = 30;
const SAVE_URL_MAX: usize = 2048;

impl Hist {
    pub fn seeded(url: &str, profile: &str) -> Self {
        let mut h = Self::default();
        h.push(url, profile);
        h
    }

    pub fn url(&self) -> String {
        self.entries.get(self.idx).map(|e| e.url.clone()).unwrap_or_default()
    }

    pub fn can_back(&self) -> bool {
        self.idx > 0
    }

    pub fn can_forward(&self) -> bool {
        self.idx + 1 < self.entries.len()
    }

    /// A new view was created: its native history is empty, so it only knows the current entry.
    pub fn fresh_view(&mut self) {
        self.base = self.idx;
        self.top = self.idx;
    }

    /// The current entry is a page our list cannot trust the native history around (an error page
    /// may have added one or two native entries): nothing before it is reachable natively.
    pub fn fence(&mut self) {
        self.base = self.idx + 1;
        self.top = self.idx;
    }

    /// A new page after the current one; forward entries are dropped.
    pub fn push(&mut self, url: &str, profile: &str) {
        if !self.entries.is_empty() {
            self.entries.truncate(self.idx + 1);
        }
        self.entries.push(Entry { url: url.into(), profile: profile.into() });
        self.idx = self.entries.len() - 1;
        self.top = self.idx;
        self.base = self.base.min(self.idx);
    }

    /// The current entry turned out to live at another address (redirect, `replaceState`).
    pub fn replace(&mut self, url: &str) {
        if let Some(e) = self.entries.get_mut(self.idx) {
            e.url = url.into();
        }
    }

    /// Move one step. `profile` is the one the view currently shows.
    pub fn step(&mut self, delta: i32, profile: &str) -> Step {
        let new = self.idx as i64 + delta as i64;
        if new < 0 || new as usize >= self.entries.len() || delta == 0 {
            return Step::None;
        }
        let new = new as usize;
        let target = self.entries[new].clone();
        let native = target.profile == profile && if new < self.idx { new >= self.base && self.idx <= self.top } else { new <= self.top && self.idx >= self.base };
        self.idx = new;
        if native {
            Step::Native
        } else {
            // Loading appends to the native history and drops what was after it.
            self.base = new;
            self.top = new;
            Step::Load(target)
        }
    }

    /// A main-frame page committed at `url`.
    /// - `traverse`: we caused this load by stepping/reloading; the entry exists, only its address may differ.
    /// - `fresh`: first load of a new view, which started at the current entry; a different address is a redirect.
    /// - `fwd_back`: the engine says this was a history traversal (a page called `history.back()`).
    pub fn commit(&mut self, url: &str, profile: &str, traverse: bool, fresh: bool, fwd_back: bool) {
        if self.entries.is_empty() {
            return self.push(url, profile);
        }
        if traverse || (fresh && self.entries[self.idx].url != url) {
            return self.replace(url);
        }
        if self.entries[self.idx].url == url {
            return;
        }
        if fwd_back {
            if self.idx > 0 && self.entries[self.idx - 1].url == url {
                return self.idx -= 1;
            }
            if self.idx + 1 < self.entries.len() && self.entries[self.idx + 1].url == url {
                return self.idx += 1;
            }
        }
        self.push(url, profile);
    }

    /// The part worth saving: at most `SAVE_MAX` entries around the current one, without huge addresses.
    pub fn for_save(&self) -> (Vec<Entry>, usize) {
        let start = self.idx.saturating_sub(SAVE_MAX * 4 / 5);
        let end = (start + SAVE_MAX).min(self.entries.len());
        let mut out = vec![];
        let mut idx = 0;
        for (i, e) in self.entries.iter().enumerate().take(end).skip(start) {
            if i == self.idx {
                idx = out.len();
            }
            if e.url.len() <= SAVE_URL_MAX {
                out.push(e.clone());
            } else if i == self.idx {
                return (vec![], 0);
            }
        }
        (out, idx)
    }

    /// Put a saved list back, if it still describes the tab (`url` is its saved address). `known` filters
    /// profile names that no longer exist.
    pub fn restore(&mut self, saved: Vec<Entry>, idx: usize, url: &str, tab_profile: &str, known: impl Fn(&str) -> bool) {
        if saved.get(idx).map(|e| e.url.as_str()) != Some(url) || saved.len() > SAVE_MAX * 2 {
            return;
        }
        self.entries = saved
            .into_iter()
            .map(|mut e| {
                if !known(&e.profile) {
                    e.profile = tab_profile.into();
                }
                e
            })
            .collect();
        self.idx = idx;
        self.fresh_view();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist(urls: &[&str]) -> Hist {
        let mut h = Hist::default();
        for u in urls {
            h.push(u, "A");
        }
        h.fresh_view();
        h.base = 0; // as if one view loaded them all
        h
    }

    #[test]
    fn native_inside_the_views_window() {
        let mut h = hist(&["a", "b", "c"]);
        assert_eq!(h.step(-1, "A"), Step::Native);
        assert_eq!(h.step(-1, "A"), Step::Native);
        assert_eq!(h.step(-1, "A"), Step::None);
        assert_eq!(h.step(1, "A"), Step::Native);
        assert_eq!(h.idx, 1);
    }

    #[test]
    fn fresh_view_cannot_go_back_natively() {
        let mut h = hist(&["a", "b", "c"]);
        h.fresh_view(); // e.g. restored after a restart or woken from sleep
        assert_eq!(h.step(-1, "A"), Step::Load(Entry { url: "b".into(), profile: "A".into() }));
        // the load rewrote the native list: "b" is now alone in it
        assert_eq!(h.step(-1, "A"), Step::Load(Entry { url: "a".into(), profile: "A".into() }));
        assert_eq!(h.step(1, "A"), Step::Load(Entry { url: "b".into(), profile: "A".into() }));
    }

    #[test]
    fn profile_change_is_never_native() {
        let mut h = hist(&["a", "b"]);
        h.entries[0].profile = "B".into();
        assert!(matches!(h.step(-1, "A"), Step::Load(e) if e.profile == "B"));
        assert_eq!(h.idx, 0);
    }

    #[test]
    fn push_drops_forward_entries() {
        let mut h = hist(&["a", "b", "c"]);
        h.step(-1, "A");
        h.step(-1, "A");
        h.push("d", "A");
        assert_eq!(h.entries.iter().map(|e| e.url.as_str()).collect::<Vec<_>>(), ["a", "d"]);
        assert_eq!(h.step(-1, "A"), Step::Native);
    }

    #[test]
    fn redirects_and_replace_do_not_drift() {
        let mut h = Hist::seeded("http://x", "A");
        h.fresh_view();
        h.commit("https://x/home", "A", false, true, false); // first load redirected
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.url(), "https://x/home");
        h.commit("https://x/home", "A", false, false, false); // reload
        assert_eq!(h.entries.len(), 1);
        h.commit("https://x/b", "A", false, false, false);
        assert_eq!(h.entries.len(), 2);
        h.replace("https://x/b2"); // replaceState
        assert_eq!((h.entries.len(), h.url().as_str()), (2, "https://x/b2"));
        h.commit("https://x/c", "A", true, false, false); // traverse load landed elsewhere
        assert_eq!((h.entries.len(), h.url().as_str()), (2, "https://x/c"));
    }

    #[test]
    fn page_driven_history_back_is_a_step_not_a_push() {
        let mut h = hist(&["a", "b", "c"]);
        h.commit("b", "A", false, false, true);
        assert_eq!((h.idx, h.entries.len()), (1, 3));
        h.commit("c", "A", false, false, true);
        assert_eq!(h.idx, 2);
        h.commit("z", "A", false, false, true); // unknown target: ordinary push
        assert_eq!((h.idx, h.entries.len()), (3, 4));
    }

    #[test]
    fn error_page_fences_native_history() {
        let mut h = hist(&["a", "b"]);
        h.fence();
        h.push("c", "A");
        assert!(matches!(h.step(-1, "A"), Step::Load(_)));
        let mut h = hist(&["a", "b"]);
        h.fence();
        assert!(matches!(h.step(-1, "A"), Step::Load(_)));
    }

    #[test]
    fn save_and_restore() {
        let mut h = Hist::default();
        for i in 0..100 {
            h.push(&format!("u{i}"), "A");
        }
        h.step(-3, "A");
        let (saved, idx) = h.for_save();
        assert!(saved.len() <= SAVE_MAX);
        assert_eq!(saved[idx].url, "u96");
        let mut r = Hist::default();
        r.restore(saved.clone(), idx, "u96", "A", |p| p == "A");
        assert_eq!(r.url(), "u96");
        assert!(r.can_forward() && r.can_back());
        assert!(matches!(r.step(-1, "A"), Step::Load(_)));
        // stale or mismatched data is ignored
        let mut r = Hist::default();
        r.restore(saved, idx, "other", "A", |_| true);
        assert!(r.entries.is_empty());
    }

    #[test]
    fn unknown_profile_falls_back_to_the_tabs() {
        let mut r = Hist::default();
        r.restore(vec![Entry { url: "a".into(), profile: "gone".into() }, Entry { url: "b".into(), profile: "A".into() }], 1, "b", "A", |p| p == "A");
        assert_eq!(r.entries[0].profile, "A");
    }

    #[test]
    fn huge_urls_are_not_saved() {
        let mut h = Hist::default();
        h.push("a", "A");
        h.push(&format!("data:{}", "x".repeat(5000)), "A");
        h.push("c", "A");
        let (saved, idx) = h.for_save();
        assert_eq!((saved.len(), saved[idx].url.as_str()), (2, "c"));
        h.step(-1, "A");
        assert!(h.for_save().0.is_empty());
    }
}
