import { call } from './api.js';

const bare = (u) => u.replace(/^https?:\/\/(www\.)?/, '').replace(/\/$/, '');
const urlish = (q) => /^(\w+:\/\/|[\w-]+(\.[\w-]+)+(:\d+)?(\/|$))/.test(q);

/**
 * Autocomplete list under an address input. The list lives in its own overlay webview (page webviews
 * draw above this one's HTML), so this keeps the rows and the selection and pushes them over.
 * `anchor()` gives the input's {x, y, w} in window coordinates; `fill(text)` puts text in the input.
 */
export function createSuggest({ anchor = () => ({ x: 0, y: 0, w: 0 }), palette = false, fill }) {
  let rows = [], sel = -1, typed = '', seq = 0, timer;
  const push = () => call('suggest_show', { payload: { rows, sel }, palette, ...anchor() });

  function hide() {
    seq++;
    clearTimeout(timer);
    rows = [];
    call('suggest_hide');
  }

  async function input(q) {
    typed = q;
    sel = -1;
    const mine = ++seq;
    if (!q.trim()) return hide();
    const found = await call('history_search', { q });
    if (mine !== seq) return;
    const lq = q.toLowerCase(), seen = new Set(), hist = [];
    for (const r of found) {
      const k = bare(r.url);
      if (seen.has(k)) continue;
      seen.add(k);
      hist.push({ kind: 'hist', url: r.url, title: r.title, score: k.toLowerCase().startsWith(lq) ? 0 : r.url.toLowerCase().includes(lq) ? 1 : 2 });
    }
    hist.sort((a, b) => a.score - b.score);
    const head = { kind: urlish(q) ? 'go' : 'search', url: q, title: q };
    rows = [head, ...hist.slice(0, 3)];
    push();
    // the engine's completions arrive a moment later and slot in under the history matches
    clearTimeout(timer);
    timer = setTimeout(async () => {
      const words = await call('search_suggest', { q }).catch(() => []);
      if (mine !== seq || sel >= 0 || !words.length) return;
      const more = words.filter((w) => w.toLowerCase() !== lq).slice(0, 4).map((w) => ({ kind: 'search', url: w, title: w }));
      rows = [head, ...hist.slice(0, 3), ...more];
      push();
    }, 120);
  }

  /** Arrow keys move the highlight and fill the input; Escape closes. True when the key was used. */
  function key(e) {
    if (!rows.length) return false;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const n = rows.length + 1;
      sel = ((sel + 1 + (e.key === 'ArrowDown' ? 1 : -1) + n) % n) - 1;
      fill(sel < 0 ? typed : rows[sel].url);
      push();
      return true;
    }
    if (e.key === 'Escape') {
      hide();
      return true;
    }
    return false;
  }

  return { input, key, hide };
}
