// Reader view: runs inside the page after Readability.js. Re-running it (or "Exit") reloads the page.
(() => {
  if (window.__aditusReader) return location.reload();
  const art = new Readability(document.cloneNode(true)).parse();
  if (!art || !art.content) return;
  window.__aditusReader = 1;
  const dark = matchMedia('(prefers-color-scheme: dark)').matches;
  const esc = (s) => String(s || '').replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));
  document.head.innerHTML = '<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>' + esc(art.title) + '</title>' +
    '<style>:root{color-scheme:light dark}body{margin:0;background:' + (dark ? '#14161a' : '#faf8f4') + ';color:' + (dark ? '#d9dce1' : '#222') + ';font:19px/1.7 Georgia,"Iowan Old Style",serif}' +
    'main{max-width:680px;margin:0 auto;padding:48px 24px 96px}h1{font:700 34px/1.25 -apple-system,system-ui,sans-serif;margin:0 0 8px}' +
    '.by{font:14px -apple-system,system-ui,sans-serif;opacity:.6;margin:0 0 32px}img,video{max-width:100%;height:auto}a{color:#d98a00}' +
    'pre,code{font-size:15px}pre{overflow:auto;padding:12px;border-radius:8px;background:rgba(128,128,128,.15)}blockquote{margin:0;padding-left:16px;border-left:3px solid rgba(128,128,128,.4);opacity:.85}' +
    '.exit{position:fixed;top:14px;right:14px;font:600 13px -apple-system,system-ui,sans-serif;padding:6px 12px;border-radius:8px;border:1px solid rgba(128,128,128,.4);background:transparent;color:inherit;cursor:pointer}</style>';
  document.body.innerHTML = '<button class="exit">Exit reader</button><main><h1></h1><p class="by"></p><article></article></main>';
  document.querySelector('h1').textContent = art.title || '';
  document.querySelector('.by').textContent = [art.byline, art.siteName].filter(Boolean).join(' · ');
  document.querySelector('article').innerHTML = art.content;
  document.querySelector('.exit').onclick = () => location.reload();
  scrollTo(0, 0);
})();
