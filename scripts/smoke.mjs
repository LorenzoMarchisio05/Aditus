// Launches a debug build and checks the basics over the DevTools protocol.
// Usage: [SMOKE_PORT=9333] node scripts/smoke.mjs <path to the aditus binary>
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const bin = process.argv[2];
if (!bin) throw new Error('usage: node scripts/smoke.mjs <binary>');
const PORT = Number(process.env.SMOKE_PORT) || 9333;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const fails = [];
const check = (name, ok) => { console.log(ok ? 'ok  ' : 'FAIL', name); if (!ok) fails.push(name); };

const web = createServer((req, res) => res.end(`<title>${req.url === '/two' ? 'smoke2' : 'smoke'}</title><p>hello</p>`)).listen(0);
const webPort = web.address().port;
const app = spawn(bin, [], {
  env: { ...process.env, ADITUS_DEBUG_PORT: String(PORT), ADITUS_DATA_DIR: mkdtempSync(join(tmpdir(), 'aditus-')) },
  stdio: 'ignore',
});

const targets = async () => (await (await fetch(`http://127.0.0.1:${PORT}/json`)).json());
const until = async (fn, ms = 15000) => {
  for (const t = Date.now(); Date.now() - t < ms; await sleep(200)) {
    try { const v = await fn(); if (v) return v; } catch {}
  }
  return null;
};
async function evalIn(t, expression) {
  const ws = new WebSocket(t.webSocketDebuggerUrl);
  await new Promise((r) => (ws.onopen = r));
  const out = await new Promise((r) => {
    ws.onmessage = (m) => r(JSON.parse(m.data));
    ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true } }));
  });
  ws.close();
  return out.result?.result?.value;
}

try {
  const ui = await until(async () => (await targets()).find((t) => t.url.includes('index.html?win=main')));
  check('ui loads', !!ui);
  if (ui) {
    await evalIn(ui, `new Promise((s) => cefQuery({ request: JSON.stringify({ cmd: 'new_tab', args: { url: 'http://127.0.0.1:${webPort}/' } }), onSuccess: s, onFailure: s }))`);
    const page = await until(async () => (await targets()).find((t) => t.url.startsWith(`http://127.0.0.1:${webPort}`) && t.title === 'smoke'));
    check('http page loads in a tab', !!page);
    if (page) check('page script injected (cefQuery hidden from the page)', (await evalIn(page, 'typeof window.cefQuery')) === 'undefined');
    const q = (cmd, url) => evalIn(ui, `new Promise((s) => cefQuery({ request: JSON.stringify({ cmd: '${cmd}', args: { url: '${url}' } }), onSuccess: s, onFailure: s }))`);
    await q('new_tab', `http://127.0.0.1:${webPort}/two`);
    check('second tab loads', !!(await until(async () => (await targets()).find((t) => t.title === 'smoke2'))));
    check('first tab still alive', !!(await targets()).find((t) => t.title === 'smoke'));
    await q('new_tab', 'aditus://settings');
    check('settings page opens', !!(await until(async () => (await targets()).find((t) => t.url.startsWith('aditus://app/') && t.url.includes('view=settings')))));
  }
} finally {
  app.kill();
  web.close();
}
if (fails.length) { console.log(`${fails.length} failed`); process.exit(1); }
