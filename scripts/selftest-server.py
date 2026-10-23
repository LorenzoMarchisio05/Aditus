import http.server, sys, urllib.parse, datetime
PORT = int(sys.argv[1])
LOG = open('/tmp/aditus-server.log', 'a', buffering=1)
def log(*a): print(datetime.datetime.now().strftime('%H:%M:%S'), *a, file=LOG)
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, body, headers=()):
        self.send_response(200)
        self.send_header('Content-Type', getattr(self, 'ctype', 'text/html'))
        for k, v in headers: self.send_header(k, v)
        self.end_headers(); self.wfile.write(body.encode())
    def do_GET(self):
        u = urllib.parse.urlparse(self.path); q = urllib.parse.parse_qs(u.query)
        host = self.headers.get('Host'); ck = self.headers.get('Cookie')
        if u.path == '/set':
            log('SET', host); self.send('<h1>set</h1>', [('Set-Cookie', 'sid=secret-from-' + host.split(':')[0] + '; Path=/')])
        elif u.path == '/report':
            log('REPORT', q.get('step'), 'host=' + host, 'cookie=' + str(ck)); self.send('<h1>report</h1>')
        elif u.path == '/ipc':
            self.send('''<h1>ipc</h1><script>(async()=>{const r={};
            for (const c of ['new_tab','get_state','close_tab','save_settings','history_clear','page_ping','cosmetic_css']) {
              try { await window.__TAURI_INTERNALS__.invoke(c, c==='page_ping'?{kind:'x'}:(c==='close_tab'?{id:999}:(c==='save_settings'?{settings:{}}:{}))); r[c]='ALLOWED'; } catch(e) { r[c]='denied: '+String(e).slice(0,70); } }
            fetch('/ipcresult?'+new URLSearchParams(r)); })()</script>''')
        elif u.path == '/ipcresult':
            log('IPC', {k: v[0] for k, v in q.items()}); self.send('ok')
        elif u.path == '/link':
            self.send('<a id=l href="http://localhost:%d/report?step=link_target">go</a><script>setTimeout(()=>document.getElementById("l").click(),800)</script>' % PORT)
        elif u.path == '/redirect':
            self.send_response(302); self.send_header('Location', 'http://localhost:%d/report?step=redirect_target' % PORT); self.end_headers()
        elif u.path == '/download':
            log('DOWNLOAD'); self.ctype = 'application/octet-stream'; self.send('file-body', [('Content-Disposition', 'attachment; filename="aditus-selftest.txt"')])
        elif u.path == '/popup':
            self.send('<a id=l target=_blank href="/report?step=popup_target">x</a><script>setTimeout(()=>document.getElementById("l").click(),600)</script>')
        elif u.path == '/adpage':
            log('ADPAGE')
            self.send('<script src="/ads/banner.js"></script><script src="/ok.js"></script><div class="sponsor" id="s">ad</div><script>setTimeout(()=>fetch("/adresult?hidden="+(getComputedStyle(document.getElementById("s")).display==="none")),1500)</script>')
        elif u.path == '/adresult':
            log('ADRESULT', {k: v[0] for k, v in q.items()}); self.send('ok')
        else:
            log('OTHER', self.path); self.send('ok')
http.server.ThreadingHTTPServer(('127.0.0.1', PORT), H).serve_forever()
