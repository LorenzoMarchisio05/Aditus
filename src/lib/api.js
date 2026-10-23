import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

export const call = (cmd, args) => invoke(cmd, args);
export const on = (ev, fn) => listen(ev, (e) => fn(e.payload));
// State snapshots are broadcast per window; keep only this window's.
export const WIN = getCurrentWindow().label;
export const onState = (fn) => on('state', (s) => s.win === WIN && fn(s));

export function hue(name) {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}
// Default profile is a neutral dot; the rest get a stable hue.
export const color = (name) => (name === 'Default' ? 'var(--fg)' : `hsl(${hue(name)} 55% 62%)`);

const ACCENTS = { amber: ['#ffb020', '#d98e00'], blue: ['#6ea8fe', '#2f6fe0'], green: ['#5ad19a', '#1f9d63'], pink: ['#ff7eb6', '#d63a84'], violet: ['#b79cff', '#7a56e0'] };
export const ACCENT_NAMES = Object.keys(ACCENTS);

/** Applies theme and accent from settings to the current document. */
export function applyTheme(s) {
  const root = document.documentElement;
  const theme = s?.theme ?? 'system';
  root.dataset.theme = theme;
  const dark = theme === 'dark' || (theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  const pair = ACCENTS[s?.accent];
  if (pair && s.accent !== 'amber') root.style.setProperty('--accent', pair[dark ? 0 : 1]);
  else root.style.removeProperty('--accent');
}
