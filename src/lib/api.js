import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export const call = (cmd, args) => invoke(cmd, args);
export const on = (ev, fn) => listen(ev, (e) => fn(e.payload));

export function hue(name) {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}
export const color = (name) => `hsl(${hue(name)} 65% 58%)`;
