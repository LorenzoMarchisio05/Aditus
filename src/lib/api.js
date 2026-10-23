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
