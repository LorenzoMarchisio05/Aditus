// Talks to the Rust side through CEF's message router (`window.cefQuery`).
const send = (cmd, args) =>
  new Promise((resolve, reject) =>
    window.cefQuery({
      request: JSON.stringify({ cmd, args: args ?? {} }),
      onSuccess: (r) => resolve(r ? JSON.parse(r) : null),
      onFailure: (_, m) => reject(m),
    }),
  );

// Native menus open where the pointer is; Rust has no other way to know.
let mouse = [0, 0];
addEventListener('mousedown', (e) => (mouse = [e.clientX, e.clientY]), true);
addEventListener('contextmenu', (e) => (mouse = [e.clientX, e.clientY]), true);
const AT = new Set(['tab_menu', 'profile_menu']);
export const call = (cmd, args) => send(cmd, AT.has(cmd) ? { ...args, _x: mouse[0], _y: mouse[1] } : args);

const listeners = new Map();
window.__aditus_emit = (ev, payload) => listeners.get(ev)?.forEach((fn) => fn(payload));
export const on = async (ev, fn) => {
  if (!listeners.has(ev)) listeners.set(ev, new Set());
  listeners.get(ev).add(fn);
  return () => listeners.get(ev).delete(fn);
};
// State snapshots are broadcast per window; keep only this window's.
export const WIN = new URLSearchParams(location.search).get('win') || 'main';
export const onState = (fn) => on('state', (s) => s.win === WIN && fn(s));

export function hue(name) {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}
// Default profile is a neutral dot; the rest get a stable hue.
let custom = {};
export const setProfileColors = (profiles) => (custom = Object.fromEntries(profiles.filter((p) => p.color).map((p) => [p.name, p.color])));
export const color = (name) => custom[name] ?? (name === 'Default' ? 'var(--fg)' : `hsl(${hue(name)} 55% 62%)`);

const ACCENTS = { amber: ['#ffb020', '#d98e00'], blue: ['#6ea8fe', '#2f6fe0'], green: ['#5ad19a', '#1f9d63'], pink: ['#ff7eb6', '#d63a84'], violet: ['#b79cff', '#7a56e0'] };
export const ACCENT_NAMES = Object.keys(ACCENTS);

/** Applies theme and accent from settings to the current document. */
export function applyTheme(s) {
  const root = document.documentElement;
  const theme = s?.theme ?? 'system';
  root.dataset.theme = theme;
  const dark = theme === 'dark' || (theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  const pair = ACCENTS[s?.accent];
  if (/^#[0-9a-f]{6}$/i.test(s?.accent ?? '')) root.style.setProperty('--accent', s.accent);
  else if (pair && s.accent !== 'amber') root.style.setProperty('--accent', pair[dark ? 0 : 1]);
  else root.style.removeProperty('--accent');
}
