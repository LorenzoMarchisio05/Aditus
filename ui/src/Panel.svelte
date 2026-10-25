<script>
  const mod = navigator.platform.includes('Mac') ? '⌘' : 'Ctrl';
  const hints = [
    ['Search or type an address', [mod, 'L']],
    ['New tab', [mod, 'T']],
    ['Search tabs', ['⇧', mod, 'A']],
    ['Private window', ['⇧', mod, 'N']],
  ];
  import { onMount } from 'svelte';
  import Mascot from './Mascot.svelte';
  import { call, on, onState, color, applyTheme, setProfileColors, ACCENT_NAMES } from './lib/api.js';

  let { view } = $props();
  let st = $state(null);
  let q = $state('');
  let only = $state(null); // history profile filter
  let rows = $state([]);
  let downloads = $state([]);
  let nav = $state('General');
  let newProfile = $state('');
  let newHost = $state('');
  let newHostProfile = $state('');
  let hostQuery = $state('');
  let ab = $state(null);
  let exts = $state([]);
  let extInput = $state('');
  let extStaged = $state(null);
  let extMsg = $state('');
  let extBusy = $state(false);
  const loadExts = async () => (exts = await call('ext_list', {}));
  async function extStage() {
    extBusy = true;
    extMsg = '';
    try {
      extStaged = await call('ext_stage', { input: extInput });
    } catch (e) {
      extMsg = String(e);
    }
    extBusy = false;
  }
  async function extAdd() {
    try {
      await call('ext_commit', { id: extStaged.id });
      extStaged = null;
      extInput = '';
      extMsg = 'Added. Restart Aditus to load it.';
      loadExts();
    } catch (e) {
      extMsg = String(e);
    }
  }
  async function extCancel() {
    await call('ext_discard', { id: extStaged.id });
    extStaged = null;
  }
  async function extDo(cmd, args) {
    await call(cmd, args);
    if (cmd !== 'ext_open') extMsg = 'Restart Aditus to apply this change.';
    loadExts();
  }
  let upd = $state(null); // null | 'checking' | result

  const NAV = ['General', 'Tabs', 'Startup and downloads', 'Profiles and sites', 'Ad blocker', 'Search', 'Privacy', 'Extensions', 'Shortcuts', 'About'];
  const slug = (n) => n.toLowerCase().replaceAll(' ', '-');
  let mainEl, jumping = false;
  function jump(n) {
    nav = n;
    jumping = true;
    document.getElementById('sec-' + slug(n))?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    setTimeout(() => (jumping = false), 700);
  }
  function spy() {
    if (jumping || !mainEl) return;
    const end = mainEl.scrollTop + mainEl.clientHeight >= mainEl.scrollHeight - 4;
    if (end) return void (nav = NAV[NAV.length - 1]);
    let cur = NAV[0];
    for (const n of NAV) if (document.getElementById('sec-' + slug(n)).offsetTop <= mainEl.scrollTop + 60) cur = n;
    nav = cur;
  }
  const SHORTCUTS = [['New tab', '⌘T'], ['New window', '⌘N'], ['Close tab', '⌘W'], ['Reopen closed tab', '⇧⌘T'], ['Open location', '⌘L'], ['Find in page', '⌘F'], ['Search tabs', '⇧⌘A'], ['Reload', '⌘R'], ['Back / forward', '⌘[ / ⌘]'], ['Toggle sidebar', '⌘\\'], ['Toggle top bar', '⇧⌘L'], ['Developer tools', '⌥⌘I'], ['Bookmark page', '⌘D'], ['Bookmarks', '⌥⌘B'], ['History', '⌘Y'], ['Downloads', '⇧⌘J'], ['Settings', '⌘,']];
  const newtab = $derived(view === 'newtab');
  $effect(() => {
    if (!newtab) document.title = { history: 'History', downloads: 'Downloads', bookmarks: 'Bookmarks', settings: 'Settings' }[view] ?? 'Aditus';
  });
  const closeTab = () => call('close_tab', { id: st.active });
  const library = $derived(view === 'history' || view === 'downloads');
  const bookmarksView = $derived(view === 'bookmarks');
  let marks = $state([]);
  async function loadMarks() {
    marks = await call('bookmark_search', { q });
  }

  async function loadHistory() {
    rows = await call('history_search', { q });
  }
  async function loadDownloads() {
    downloads = await call('downloads_list');
  }

  onMount(() => {
    loadExts();
    loadLogins();
    const offs = [];
    (async () => {
      offs.push(
        await onState((s) => {
          setProfileColors(s.cfg.profiles);
          st = s;
          newHostProfile ||= s.cfg.profiles[0].name;
        })
      );
      offs.push(await on('downloads-changed', loadDownloads));
      call('get_state'); // after listeners exist, or the reply is lost
    })();
    if (bookmarksView) loadMarks();
    if (library || newtab) {
      loadHistory();
      loadDownloads();
    }
    return () => offs.forEach((f) => f());
  });

  $effect(() => {
    applyTheme(st?.cfg.settings);
  });

  $effect(() => {
    if (view === 'settings') call('adblock_info').then((i) => (ab = i));
  });
  let updMsg = $state('');
  async function installUpdate() {
    updMsg = 'Downloading…';
    try {
      await call('install_update');
      updMsg = 'Restarting…';
    } catch (e) {
      updMsg = String(e);
    }
  }
  async function checkUpdate() {
    upd = 'checking';
    upd = await call('check_update');
  }
  const toggleList = (name, on) => {
    const off = new Set(st.cfg.settings.disabled_lists ?? []);
    on ? off.delete(name) : off.add(name);
    set({ disabled_lists: [...off] });
    setTimeout(() => call('adblock_info').then((i) => (ab = i)), 1500);
  };
  const ago = (h) => (h == null ? 'never' : h < 1 ? 'just now' : h < 48 ? `${h}h ago` : `${Math.floor(h / 24)}d ago`);
  const NAMES = { easylist: ['EasyList', 'ads'], easyprivacy: ['EasyPrivacy', 'trackers'], ublock: ['uBlock filters', 'annoyances'] };

  let clr = $state({ history: true, downloads: false, cookies: false, cache: true, hours: 24, profile: '' });
  let clrArmed = $state(false), clrMsg = $state(''), clrTimer;
  async function clearData() {
    if (!clrArmed) {
      clrArmed = true;
      clearTimeout(clrTimer);
      clrTimer = setTimeout(() => (clrArmed = false), 4000);
      return;
    }
    clrArmed = false;
    await call('clear_data', { ...clr });
    clrMsg = 'Cleared.';
    setTimeout(() => (clrMsg = ''), 3000);
  }
  let logins = $state([]);
  const loadLogins = () => call('login_list').then((l) => (logins = l)).catch(() => {});
  async function removeLogin(l) {
    await call('login_remove', { origin: l[0], username: l[1] });
    loadLogins();
  }
  async function importCsv(e) {
    const f = e.currentTarget.files[0];
    if (!f) return;
    try {
      impMsg = `Imported ${await call('import_passwords_csv', { text: await f.text() })} passwords. Delete the CSV file now.`;
    } catch (x) {
      impMsg = String(x);
    }
    e.currentTarget.value = '';
    loadLogins();
  }
  let impMsg = $state('');
  async function importChrome() {
    try {
      impMsg = `Imported ${await call('import_bookmarks', {})} bookmarks and ${await call('import_history', {})} history entries.`;
    } catch (e) {
      impMsg = String(e);
    }
    setTimeout(() => (impMsg = ''), 4000);
  }
  const set = (patch) => call('save_settings', { settings: { ...st.cfg.settings, ...patch } });
  const engine = $derived(
    !st ? 'ddg' : st.cfg.settings.search_template.includes('duckduckgo') ? 'ddg' : st.cfg.settings.search_template.includes('google.com/search') ? 'google' : 'custom'
  );
  const pick = (v) => {
    if (v === 'ddg') set({ search_template: 'https://duckduckgo.com/?q=%s' });
    if (v === 'google') set({ search_template: 'https://www.google.com/search?q=%s' });
    if (v === 'custom') set({ search_template: 'https://example.com/search?q=%s' });
  };
  const ENGINES = [['ddg', 'DuckDuckGo', 'default'], ['google', 'Google', 'built in'], ['custom', 'Custom', 'your own URL']];

  const shown = $derived(only ? rows.filter((r) => r.profile === only) : rows);
  const hm = (t) => new Date(t * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const dayOf = (t) => {
    const d = new Date(t * 1000).toDateString();
    const today = new Date();
    if (d === today.toDateString()) return 'Today';
    if (d === new Date(today - 864e5).toDateString()) return 'Yesterday';
    return d;
  };
  const days = $derived(
    shown.reduce((acc, r) => {
      const d = dayOf(r.visited_at);
      (acc.at(-1)?.day === d ? acc.at(-1) : acc[acc.push({ day: d, rows: [] }) - 1]).rows.push(r);
      return acc;
    }, [])
  );
  const host = (u) => { try { return new URL(u).host; } catch { return ''; } };
  const fname = (d) => d.path.split(/[\\/]/).pop() || d.url;
  const ext = (d) => (fname(d).split('.').pop() || '').slice(0, 4).toUpperCase();
  const tint = { done: '#5dffb8', failed: '#ff7a8a' };
  const mapped = $derived(
    (st?.cfg.mappings ?? []).filter((m) => m.pattern.includes(hostQuery.trim().toLowerCase()))
  );
  // Most visited sites, one per domain; sign-in and consent hosts are noise here.
  const topSites = $derived.by(() => {
    const m = new Map();
    for (const r of rows) {
      const h = host(r.url);
      if (!h || /^(127\.|localhost)/.test(h) || /^(accounts|consent|gds|auth|sso|login|id|myaccount)\./.test(h)) continue;
      const root = h.split('.').slice(-2).join('.');
      if (!m.has(root)) m.set(root, { host: h.replace(/^www\./, ''), url: r.url, profile: r.profile, n: 0 });
      m.get(root).n++;
    }
    return [...m.values()].sort((a, b) => b.n - a.n).slice(0, 8);
  });
  const now = new Date();
  const greeting = now.getHours() < 5 ? 'Still up' : now.getHours() < 12 ? 'Good morning' : now.getHours() < 18 ? 'Good afternoon' : 'Good evening';
  const today = now.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
  const count = (name) => st.tabs.filter((t) => t.profile === name).length;
  const sites = (name) => st.cfg.mappings.filter((m) => m.profile === name).length;
</script>

<div class="panel">
  {#if st}<Mascot incognito={st.incognito} roam={newtab || view === 'settings' || view === 'history'} side={view === 'settings'} tabs={st.tabs} />{/if}
  {#if bookmarksView}
    <div class="lib" style="grid-template-columns: minmax(0, 1fr)">
      <section class="hist">
        <header>
          <h1>Bookmarks</h1>
          <span class="grow"></span>
          <button class="x" onclick={closeTab} aria-label="Close">✕</button>
        </header>
        <input class="search" placeholder="Search bookmarks" bind:value={q} oninput={loadMarks} />
        <div class="list">
          {#each marks as r}
            <div style="display:flex;align-items:center;gap:8px">
              <button class="hrow" style="flex:1;min-width:0" onclick={() => call('new_tab', { url: r.url })}>
                <span class="pb" style="background:{color(r.profile)}"></span>
                <span class="t">{r.title || r.url}</span>
                <span class="meta">{host(r.url)}</span>
              </button>
              <button class="ghost" onclick={async () => { await call('bookmark_remove', { url: r.url }); loadMarks(); }}>Remove</button>
            </div>
          {:else}
            <div class="meta empty">No bookmarks yet. Press ⌘D on a page to add one.</div>
          {/each}
        </div>
      </section>
    </div>
  {:else if library}
    <div class="lib">
      <section class="hist">
        <header>
          <h1>History</h1>
          <span class="grow"></span>
          <button class="ghost" onclick={async () => { await call('history_clear'); loadHistory(); }}>Clear…</button>
          <button class="x" onclick={closeTab} aria-label="Close">✕</button>
        </header>
        <input class="search" placeholder="Search history" bind:value={q} oninput={loadHistory} />
        {#if st}
          <div class="chips">
            <button class:on={!only} onclick={() => (only = null)}>All profiles</button>
            {#each st.cfg.profiles as p}
              <button class:on={only === p.name} onclick={() => (only = p.name)}><i style="background:{color(p.name)}"></i>{p.name}</button>
            {/each}
          </div>
        {/if}
        <div class="list">
          {#each days as d}
            <div class="day">{d.day}</div>
            {#each d.rows as r}
              <button class="hrow" onclick={() => call('new_tab', { url: r.url })}>
                <span class="time">{hm(r.visited_at)}</span>
                <span class="pb" style="background:{color(r.profile)}"></span>
                <span class="t">{r.title || r.url}</span>
                <span class="meta">{host(r.url)}</span>
              </button>
            {/each}
          {:else}
            <div class="meta empty">Nothing here.</div>
          {/each}
        </div>
      </section>
      <section class="dl">
        <header>
          <h1>Downloads</h1>
          <span class="grow"></span>
        </header>
        {#each downloads as d}
          <div class="card dcard">
            <span class="ext">{ext(d)}</span>
            <span class="grow">
              <span class="name">{fname(d)}</span>
              <span class="meta">{new Date(d.started_at * 1000).toLocaleString()}</span>
            </span>
            <span class="state" style="color:{tint[d.status] ?? 'var(--accent)'}">{d.status}</span>
            {#if d.path && d.status === 'done'}<button class="ghost" onclick={() => call('open_download', { path: d.path })}>Open</button>{/if}
            {#if d.path}<button class="ghost" onclick={() => call('reveal_path', { path: d.path })}>Show</button>{/if}
          </div>
        {:else}
          <div class="meta">No downloads yet.</div>
        {/each}
      </section>
    </div>
  {:else if newtab && st}
    <div class="nt">
      <div class="hero">
        <svg width="96" height="96" viewBox="21 21 62 62" aria-hidden="true"><defs><linearGradient id="gc" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFE3A3"/><stop offset="1" stop-color="#FFB020"/></linearGradient></defs><path d="M28 74V26" stroke="currentColor" stroke-width="7" stroke-linecap="round"/><path d="M28 74H74A46 46 0 0 0 28 28z" fill="url(#gc)"/><path d="M28 74H74" stroke="currentColor" stroke-width="7" stroke-linecap="round"/></svg>
        {#if st.incognito}
          <h1>Private window</h1>
          <p class="sub">Nothing from this window is saved: no history, cookies, logins or session. Downloads stay.</p>
        {:else}
          <span class="when">{today}</span>
          <h1>{greeting}.</h1>
        {/if}
      </div>
      {#if !st.incognito && topSites.length}
        <div class="sites">
          {#each topSites as s}
            <button class="site" onclick={() => { call('new_tab', { url: s.url }); call('hide_panel'); }}>
              <span class="ico"><b>{s.host[0].toUpperCase()}</b><img src="https://{s.host}/favicon.ico" alt="" onerror={(e) => (e.currentTarget.style.display = 'none')} /><i style="background:{color(s.profile)}"></i></span>
              <span class="hn">{s.host}</span>
            </button>
          {/each}
        </div>
      {/if}
      <div class="keys">
        {#each hints as [label, keys]}
          <span class="hint"><b>{label}</b><span class="caps">{#each keys as k}<kbd class:glyph={k === '⇧' || k === '⌘'} class:shift={k === '⇧'}>{k}</kbd>{/each}</span></span>
        {/each}
      </div>

    </div>
  {:else if st}
    <div class="settings">
      <aside>
        <div class="title"><span class="logo"><svg viewBox="0 0 100 100" width="70%" height="70%" fill="currentColor" aria-hidden="true"><path d="M28 74V26" stroke="currentColor" stroke-width="7" stroke-linecap="round"/><path d="M28 74H74A46 46 0 0 0 28 28z"/><path d="M28 74H74" stroke="currentColor" stroke-width="7" stroke-linecap="round"/></svg></span><span><b>Settings</b><small>Aditus 0.1.1</small></span></div>
        {#each NAV as n}
          <button class="nav" class:on={nav === n} onclick={() => jump(n)}>{n}</button>
        {/each}
        <span class="grow"></span>
        <button class="nav" onclick={closeTab}>Close</button>
      </aside>

      <main bind:this={mainEl} onscroll={spy}>
        <section id="sec-general" class="sec">
          <h1>General</h1>
          <div class="card list2">
            <label class="item">
              <span class="grow"><b>Show top bar</b><small>Back, forward, address and profile. Cmd+L opens the palette when hidden.</small></span>
              <input type="checkbox" class="sw" checked={st.cfg.settings.topbar_visible} onchange={(e) => set({ topbar_visible: e.currentTarget.checked })} />
            </label>
            <label class="item">
              <span class="grow"><b>Collapse sidebar to icons</b><small>Toggle any time from the View menu.</small></span>
              <input type="checkbox" class="sw" checked={st.cfg.settings.sidebar_collapsed} onchange={(e) => set({ sidebar_collapsed: e.currentTarget.checked })} />
            </label>
            <div class="item">
              <span class="grow"><b>Theme</b><small>Follows the system unless you pick one.</small></span>
              <span class="segc">
                {#each [['system', 'System'], ['dark', 'Dark'], ['light', 'Light']] as [v, l]}
                  <button class:on={st.cfg.settings.theme === v} onclick={() => set({ theme: v })}>{l}</button>
                {/each}
              </span>
            </div>
            <label class="item">
              <span class="grow"><b>Sidebar width</b><small>Set it here, or drag the edge when dragging is on.</small></span>
              <span><input type="number" min="160" max="480" value={st.cfg.settings.sidebar_width} onchange={(e) => set({ sidebar_width: Math.min(480, Math.max(160, +e.currentTarget.value)) })} /> px</span>
            </label>
            <div class="item">
              <span class="grow"><b>Developer tools</b><small>Where they open. A tab whose tools are already open keeps its position; "Window" applies to the next ones.</small></span>
              <span class="segc">
                {#each [['bottom', 'Bottom'], ['right', 'Right'], ['left', 'Left'], ['top', 'Top'], ['window', 'Window']] as [v, l]}
                  <button class:on={st.cfg.settings.devtools_dock === v} onclick={() => set({ devtools_dock: v })}>{l}</button>
                {/each}
              </span>
            </div>
            <label class="item">
              <span class="grow"><b>Resize sidebar by dragging</b><small>Drag the sidebar edge to change its width.</small></span>
              <input type="checkbox" class="sw" checked={st.cfg.settings.sidebar_resizable} onchange={(e) => set({ sidebar_resizable: e.currentTarget.checked })} />
            </label>
            <div class="item">
              <span class="grow"><b>Accent colour</b><small>Highlights, switches and the active tab.</small></span>
              <span class="swatches">
                {#each ACCENT_NAMES as a}
                  <button class="swatch" class:on={st.cfg.settings.accent === a} data-a={a} aria-label={a} onclick={() => set({ accent: a })}></button>
                {/each}
                <label class="swatch custom" class:on={st.cfg.settings.accent.startsWith('#')} title="Custom colour" style={st.cfg.settings.accent.startsWith('#') ? `background:${st.cfg.settings.accent}` : ''}>
                  <input type="color" value={st.cfg.settings.accent.startsWith('#') ? st.cfg.settings.accent : '#ffb020'} onchange={(e) => set({ accent: e.currentTarget.value })} />
                </label>
              </span>
            </div>
            <label class="item">
              <span class="grow"><b>Page zoom</b><small>Applied to every site.</small></span>
              <span><input type="number" min="50" max="300" step="10" value={st.cfg.settings.page_zoom} onchange={(e) => set({ page_zoom: Math.min(300, Math.max(50, +e.currentTarget.value)) })} /> %</span>
            </label>
          </div>
        </section>
        <section id="sec-tabs" class="sec">
          <h1>Tabs</h1>
          <div class="card list2">
            <label class="item">
              <span class="grow"><b>Compact tab list</b><small>Shorter rows, more tabs in view.</small></span>
              <input type="checkbox" class="sw" checked={st.cfg.settings.compact_tabs} onchange={(e) => set({ compact_tabs: e.currentTarget.checked })} />
            </label>
            <label class="item">
              <span class="grow"><b>Open links in the background</b><small>Links that open a new tab load behind the current one.</small></span>
              <input type="checkbox" class="sw" checked={st.cfg.settings.links_background} onchange={(e) => set({ links_background: e.currentTarget.checked })} />
            </label>
            <label class="item">
              <span class="grow"><b>Search suggestions</b><small>Send what you type in the address bar to your search engine for completions. Never from private windows.</small></span>
              <input type="checkbox" class="sw" checked={st.cfg.settings.search_suggestions} onchange={(e) => set({ search_suggestions: e.currentTarget.checked })} />
            </label>
            <label class="item">
              <span class="grow"><b>Sleep idle tabs after</b><small>Pinned and playing tabs stay awake.</small></span>
              <span><input type="number" min="1" max="1440" value={st.cfg.settings.idle_minutes} onchange={(e) => set({ idle_minutes: Math.max(1, +e.currentTarget.value) })} /> min</span>
            </label>
            <label class="item">
              <span class="grow"><b>Memory limit</b><small>Over this, the least recently used background tabs sleep. 0 turns it off.</small></span>
              <span><input type="number" min="0" max="65536" step="256" value={st.cfg.settings.memory_limit_mb} onchange={(e) => set({ memory_limit_mb: Math.max(0, +e.currentTarget.value) })} /> MB</span>
            </label>
          </div>
        </section>
        <section id="sec-startup-and-downloads" class="sec">
          <h1>Startup and downloads</h1>
          <div class="card list2">
            <div class="item">
              <span class="grow"><b>When Aditus starts</b><small>Pick up where you left off, or start fresh.</small></span>
              <span class="segc">
                {#each [['restore', 'Last session'], ['newtab', 'New tab'], ['home', 'Home page']] as [v, l]}
                  <button class:on={st.cfg.settings.startup === v} onclick={() => set({ startup: v })}>{l}</button>
                {/each}
              </span>
            </div>
            {#if st.cfg.settings.startup === 'home'}
              <label class="item">
                <span class="grow"><b>Home page</b><small>Opened on start, https://… only.</small></span>
                <input class="wide" placeholder="https://" value={st.cfg.settings.home_url} onchange={(e) => set({ home_url: e.currentTarget.value.trim() })} />
              </label>
            {/if}
            <label class="item">
              <span class="grow"><b>Download folder</b><small>Leave empty for your system Downloads folder.</small></span>
              <input class="wide" placeholder="/path/to/folder" value={st.cfg.settings.download_dir} onchange={(e) => set({ download_dir: e.currentTarget.value.trim() })} />
            </label>
          </div>
        </section>
        <section id="sec-profiles-and-sites" class="sec">
          <h1>Profiles <small>Separate cookies and logins. Each tab runs in one profile.</small></h1>
          <div class="pgrid">
            {#each st.cfg.profiles as p}
              <div class="card pcard">
                <div class="phead"><label class="pi" title="Change colour" style="background:{color(p.name)}">
                    <input type="color" value={p.color ?? '#999999'} onchange={(e) => call('set_profile_color', { name: p.name, color: e.currentTarget.value })} />
                  </label><b class="grow">{p.name}</b>
                  {#if p.color}<button class="x" title="Reset colour" onclick={() => call('set_profile_color', { name: p.name, color: null })}>Reset</button>{/if}</div>
                <div class="stats"><span><b>{sites(p.name)}</b>sites</span><span><b>{count(p.name)}</b>tabs</span></div>
                <button class="ghost" onclick={() => call('delete_profile', { name: p.name })} disabled={st.cfg.profiles.length < 2 || count(p.name) > 0}>Delete</button>
              </div>
            {/each}
            <form class="card add" onsubmit={(e) => { e.preventDefault(); if (newProfile.trim()) call('add_profile', { name: newProfile.trim() }); newProfile = ''; }}>
              <input placeholder="＋ Add profile" bind:value={newProfile} />
            </form>
          </div>

          <h1>Sites <small>A mapped site opens in its profile. Login pages never switch.</small></h1>
          <div class="row">
            <input class="grow" placeholder="Filter sites" bind:value={hostQuery} />
          </div>
          <div class="table">
            {#each mapped as m}
              <div class="trow">
                <span class="mono grow">{m.pattern}</span>
                <span class="who"><i style="background:{color(m.profile)}"></i>{m.profile}</span>
                <span class="meta w">{m.pattern.startsWith('*.') ? 'subdomains' : 'exact'}</span>
                <button class="ghost" onclick={() => call('delete_mapping', { pattern: m.pattern })}>Remove</button>
              </div>
            {:else}
              <div class="trow meta">None yet. Pick a profile in the address bar to map a site.</div>
            {/each}
          </div>
          <form class="row" onsubmit={(e) => { e.preventDefault(); if (newHost.trim()) call('assign_mapping', { host: newHost.trim(), profile: newHostProfile }); newHost = ''; }}>
            <input class="grow" placeholder="youtube.com or *.google.com" bind:value={newHost} />
            <select bind:value={newHostProfile}>{#each st.cfg.profiles as p}<option>{p.name}</option>{/each}</select>
            <button class="solid">＋ Add site</button>
          </form>
          <h2>Site permissions</h2>
          <div class="table">
            {#each st.cfg.permissions ?? [] as r}
              <div class="trow">
                <span class="mono grow">{r.host}</span>
                <span class="meta w">{r.kind}</span>
                <button class="ghost" title="Click to switch" onclick={() => call('set_permission', { host: r.host, kind: r.kind, allow: !r.allow })}>{r.allow ? 'Allowed' : 'Blocked'}</button>
                <button class="ghost" onclick={() => call('delete_permission', { host: r.host, kind: r.kind })}>Remove</button>
              </div>
            {:else}
              <div class="trow meta">None yet. Sites ask when they want your camera, microphone or location.</div>
            {/each}
          </div>
          {#if st.cfg.permissions?.length}<button class="ghost" onclick={() => call('clear_permissions')}>Reset all permissions</button>{/if}
          <p class="meta">Login and OAuth hosts (accounts.*, auth.*, sso.*) are exempt automatically, so sign-in flows are never interrupted.</p>
        </section>
        <section id="sec-ad-blocker" class="sec">
          <h1>Ad blocker</h1>
          <div class="card list2">
            <label class="item">
              <span class="grow"><b>Block ads and trackers</b><small>Lists refresh daily.</small></span>
              <input type="checkbox" class="sw" checked={st.cfg.settings.adblock} onchange={(e) => set({ adblock: e.currentTarget.checked })} />
            </label>
          </div>
          {#if ab}
            <div class="tiles">
              <div class="card tile"><b>{ab.rules.toLocaleString()}</b>rules loaded</div>
              <div class="card tile"><b>{ab.lists.filter((l) => l.enabled).length}/{ab.lists.length}</b>lists on</div>
              <div class="card tile"><b>{ago(Math.min(...ab.lists.filter((l) => l.age_hours != null).map((l) => l.age_hours)))}</b>since refresh</div>
              <div class="card tile"><b>{st.cfg.allowlist.length}</b>sites paused</div>
            </div>
            <h2>Filter lists</h2>
            <div class="card list2">
              {#each ab.lists as l}
                <label class="item">
                  <span class="grow"><b>{NAMES[l.name][0]}</b><small>{NAMES[l.name][1]} · updated {ago(l.age_hours)}</small></span>
                  <input type="checkbox" class="sw" checked={l.enabled} onchange={(e) => toggleList(l.name, e.currentTarget.checked)} />
                </label>
              {/each}
            </div>
          {/if}
          <h2>Paused on</h2>
          <div class="chips">
            {#each st.cfg.allowlist as h}
              <button class="mono" onclick={() => call('toggle_allowlist', { host: h })}>{h} <span class="meta">×</span></button>
            {:else}<span class="meta">No sites.</span>{/each}
          </div>
          <p class="meta">The shield in the address bar pauses blocking for the current site only.</p>
        </section>
        <section id="sec-search" class="sec">
          <h1>Search</h1>
          <div class="engines">
            {#each ENGINES as [id, name, sub]}
              <button class="engine" class:on={engine === id} onclick={() => pick(id)}>{name}<small>{sub}</small></button>
            {/each}
          </div>
          <div class="card tpl">
            <h2>Template</h2>
            <input class="mono" value={st.cfg.settings.search_template} disabled={engine !== 'custom'} onchange={(e) => set({ search_template: e.currentTarget.value })} />
          </div>
          <p class="meta">Searches follow the same site rules: if the search site is mapped to a profile, it opens there.</p>
        </section>
        <section id="sec-privacy" class="sec">
          <h1>Privacy</h1>
          <div class="card list2">
            <label class="item">
              <span class="grow"><b>Clear history when Aditus quits</b><small>Downloads list is kept.</small></span>
              <input type="checkbox" class="sw" checked={st.cfg.settings.clear_history_on_quit} onchange={(e) => set({ clear_history_on_quit: e.currentTarget.checked })} />
            </label>
            <label class="item">
              <span class="grow"><b>Delete history older than</b><small>0 keeps everything. Runs at launch.</small></span>
              <span><input type="number" min="0" max="3650" value={st.cfg.settings.history_days} onchange={(e) => set({ history_days: Math.max(0, +e.currentTarget.value) })} /> days</span>
            </label>
          </div>
          <h2>Saved passwords</h2>
          <div class="card list2">
            {#each logins as l}
              <div class="item"><span class="grow"><b>{l[1] || '(no username)'}</b><small>{l[0]}</small></span><button onclick={() => removeLogin(l)}>Remove</button></div>
            {:else}
              <div class="item"><span class="grow"><small>None yet. Aditus offers to save a password when you sign in. They are kept in your system keychain.</small></span></div>
            {/each}
          </div>
          <h2>Import</h2>
          <div class="card list2">
            <div class="item"><span class="grow"><b>Bookmarks and history from Chrome</b><small>{impMsg || 'Reads Chrome’s default profile. Nothing is changed there.'}</small></span><button onclick={importChrome}>Import</button></div>
            <div class="item"><span class="grow"><b>Passwords from a CSV file</b><small>Chrome, Edge or Brave export (Settings → Passwords). The file holds plain text, delete it afterwards.</small></span><input type="file" accept=".csv,text/csv" onchange={importCsv} /></div>
          </div>
          <h2>Clear browsing data</h2>
          <div class="card list2">
            <label class="item"><span class="grow"><b>Browsing history</b></span><input type="checkbox" class="sw" bind:checked={clr.history} /></label>
            <label class="item"><span class="grow"><b>Download list</b><small>The files themselves stay on disk.</small></span><input type="checkbox" class="sw" bind:checked={clr.downloads} /></label>
            <label class="item"><span class="grow"><b>Cookies</b><small>Signs you out of sites.</small></span><input type="checkbox" class="sw" bind:checked={clr.cookies} /></label>
            <label class="item"><span class="grow"><b>Cached files</b></span><input type="checkbox" class="sw" bind:checked={clr.cache} /></label>
            <div class="item">
              <span class="grow"><b>History from</b><small>Cookies and cache are always cleared in full.</small></span>
              <select bind:value={clr.hours}><option value={1}>Last hour</option><option value={24}>Last 24 hours</option><option value={168}>Last 7 days</option><option value={0}>All time</option></select>
            </div>
            <div class="item">
              <span class="grow"><b>Profile</b><small>Applies to cookies and cache.</small></span>
              <select bind:value={clr.profile}><option value="">All profiles</option>{#each st.cfg.profiles as p}<option value={p.name}>{p.name}</option>{/each}</select>
            </div>
            <div class="item">
              <span class="grow"><small>{clrMsg}</small></span>
              <button class="solid" disabled={!clr.history && !clr.downloads && !clr.cookies && !clr.cache} onclick={clearData}>{clrArmed ? 'Click again to confirm' : 'Clear data'}</button>
            </div>
          </div>
          <p class="meta">Local storage and other site data live in each profile's folder. Delete a profile to wipe all of it.</p>
        </section>
        <section id="sec-extensions" class="sec">
          <h1>Extensions</h1>
          <div class="card list2">
            <div class="item"><span class="grow"><b>Add from the Chrome Web Store</b><small>{extMsg || 'Paste an extension page link or its id. Manifest V3 only.'}</small></span><input placeholder="Link or id" bind:value={extInput} onkeydown={(e) => e.key === 'Enter' && extInput && extStage()} /><button disabled={extBusy || !extInput} onclick={extStage}>{extBusy ? 'Fetching…' : 'Fetch'}</button></div>
            {#if extStaged}
              <div class="item"><span class="grow"><b>{extStaged.name} {extStaged.version}</b><small>Can use: {[...extStaged.permissions, ...extStaged.hosts].join(', ') || 'no special permissions'}</small></span><button onclick={extAdd}>Add</button><button onclick={extCancel}>Cancel</button></div>
            {/if}
            {#each exts as x (x.id)}
              <div class="item"><span class="grow"><b>{x.name} {x.version}</b><small>{x.enabled ? 'On' : 'Off'}</small></span>
                {#if x.popup}<button onclick={() => extDo('ext_open', { id: x.id, kind: 'popup' })}>Popup</button>{/if}
                {#if x.options}<button onclick={() => extDo('ext_open', { id: x.id, kind: 'options' })}>Options</button>{/if}
                <button onclick={() => extDo('ext_toggle', { id: x.id, enabled: !x.enabled })}>{x.enabled ? 'Turn off' : 'Turn on'}</button>
                <button onclick={() => extDo('ext_remove', { id: x.id })}>Remove</button></div>
            {/each}
          </div>
          <p class="meta">Extensions can read the pages you visit, so add only ones you trust. They are shared by all profiles, don't run in private windows and don't update by themselves (remove and add again). Popups that need the current tab, like password autofill, won't work yet.</p>
        </section>
        <section id="sec-shortcuts" class="sec">
          <h1>Shortcuts</h1>
          <div class="card list2">
            {#each SHORTCUTS as [name, keys]}
              <div class="kv"><span>{name}</span><kbd>{keys}</kbd></div>
            {/each}
          </div>
          <p class="meta">Shortcuts follow the menu bar and aren't rebindable yet. Ctrl replaces ⌘ on Windows and Linux.</p>
        </section>
        <section id="sec-about" class="sec">
          <h1>About</h1>
          <div class="card about">
            <span class="logo big"><svg viewBox="0 0 100 100" width="70%" height="70%" fill="currentColor" aria-hidden="true"><path d="M28 74V26" stroke="currentColor" stroke-width="7" stroke-linecap="round"/><path d="M28 74H74A46 46 0 0 0 28 28z"/><path d="M28 74H74" stroke="currentColor" stroke-width="7" stroke-linecap="round"/></svg></span>
            <span class="grow"><b>Aditus 0.1.1</b><small>Open source, GPL-3.0</small></span>
            {#if upd && upd !== 'checking' && upd.newer}
              {#if upd.can_install}
                <button class="solid" onclick={installUpdate} disabled={updMsg === 'Downloading…'}>Install {upd.latest} and restart</button>
              {:else}
                <button class="solid" onclick={() => call('new_tab', { url: upd.url })}>Get {upd.latest}</button>
              {/if}
            {:else}
              <button class="ghost" onclick={checkUpdate} disabled={upd === 'checking'}>{upd === 'checking' ? 'Checking…' : 'Check for updates'}</button>
            {/if}
          </div>
          {#if upd && upd !== 'checking'}
            <p class="meta">{updMsg || (upd.latest === null ? 'No published release found.' : upd.newer ? `Version ${upd.latest} is available.` : "You're on the latest version.")}</p>
          {/if}
          <div class="card list2">
            <label class="item">
              <span class="grow"><b>Check for updates on launch</b><small>Looks at GitHub releases. Installing is always your click.</small></span>
              <input type="checkbox" class="sw" checked={st.cfg.settings.auto_update} onchange={(e) => set({ auto_update: e.currentTarget.checked })} />
            </label>
            <div class="item">
              <span class="grow"><b>Default browser</b><small>Your system decides; this opens its setting.</small></span>
              <button class="ghost" onclick={() => call('default_browser_settings')}>Open settings</button>
            </div>
          </div>
          <div class="card list2">
            <div class="kv"><span class="meta">Engine</span><span class="mono">Chromium (CEF)</span></div>
            <div class="kv"><span class="meta">Interface</span><span class="mono">Rust + Svelte</span></div>
            <div class="kv"><span class="meta">Data</span><span class="mono">config.toml · aditus.db</span></div>
          </div>
        </section>
      </main>
    </div>
  {/if}
</div>

<style>
  .panel {
    --card: var(--bg2);
    --hi: var(--bg3);
    height: 100vh; overflow: hidden; background: var(--bg); color: var(--fg);
  }
  .grow { flex: 1; min-width: 0; }
  small { display: block; color: var(--dim); font-size: 13px; font-weight: 400; margin-top: 2px; }
  .meta { color: var(--dim); font-size: 13px; }
  .mono { font-family: ui-monospace, 'SF Mono', monospace; font-size: 12px; }
  h1 { font-family: 'Bricolage Grotesque Variable', system-ui, sans-serif; font-weight: 800; font-size: 44px; line-height: 1; letter-spacing: -.03em; margin: 0; display: flex; align-items: baseline; gap: 14px; }
  h1 small { display: inline; margin: 0; font-family: 'Instrument Sans Variable', system-ui, sans-serif; font-size: 14px; font-weight: 400; letter-spacing: 0; }
  h2 { font: 500 10.5px ui-monospace, 'SF Mono', monospace; text-transform: uppercase; letter-spacing: .1em; color: var(--dim); margin: 18px 0 8px; }
  .card { background: var(--card); border: 1px solid var(--line); border-radius: 18px; }
  .ghost { border: 1px solid var(--line); color: var(--dim); font-weight: 500; height: 34px; border-radius: 10px; padding: 0 14px; }
  .solid { background: var(--accent); color: #111114; font-weight: 600; height: 34px; border-radius: 10px; padding: 0 14px; }
  .solid:hover:not(:disabled) { filter: brightness(1.08); background: var(--accent); }
  .x { color: var(--dim); }
  input.search, .row input, .add input { height: 38px; background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 0 14px; }

  .nt { height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 0 32px 40px; gap: 56px; overflow-y: auto; }
  .hero { display: flex; flex-direction: column; align-items: center; gap: 10px; text-align: center; }
  .hero svg { margin-bottom: 8px; }
  .hero h1 { font-size: 64px; letter-spacing: -.035em; line-height: 1; display: block; }
  .when, .lbl { font: 500 11px/1 'JetBrains Mono Variable', ui-monospace, monospace; letter-spacing: .12em; text-transform: uppercase; color: var(--dim); }
  .sub { max-width: 440px; color: var(--dim); font-size: 15px; line-height: 1.5; margin-top: 6px; }
  kbd { font: 11px ui-monospace, 'SF Mono', monospace; color: var(--dim); background: var(--bg3); border-radius: 6px; padding: 2px 6px; }
  .hero svg { filter: drop-shadow(0 0 34px rgba(255, 176, 32, .4)); }
  .sites { display: flex; flex-wrap: wrap; justify-content: center; gap: 22px 18px; max-width: 760px; }
  .site { width: 96px; display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 0; font-size: 12.5px; color: var(--dim); }
  .site:hover { color: var(--fg); background: none; }
  .site .hn { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ico { width: 64px; height: 64px; border-radius: 18px; background: var(--card); border: 1px solid var(--line); display: grid; place-items: center; position: relative; font: 800 24px 'Bricolage Grotesque Variable', system-ui, sans-serif; color: var(--fg); transition: transform .12s, border-color .12s; }
  .site:hover .ico { transform: translateY(-3px); border-color: var(--accent); }
  .ico img { position: absolute; inset: 0; width: 100%; height: 100%; padding: 16px; box-sizing: border-box; border-radius: 18px; background: var(--card); object-fit: contain; }
  .ico i { position: absolute; right: -3px; bottom: -3px; width: 14px; height: 14px; border-radius: 50%; border: 3px solid var(--bg); }
  .nt { position: relative; }
  .keys { display: flex; flex-wrap: wrap; justify-content: center; gap: 12px 28px; font-size: 13px; }
  .keys .hint { display: inline-flex; align-items: center; gap: 10px; }
  .keys .hint b { font-weight: 500; color: var(--fg2, var(--fg)); opacity: .85; }
  .caps { display: inline-flex; gap: 4px; }
  .caps kbd.glyph { font-size: 17px; line-height: 1; padding: 2px 7px; }
  .caps kbd.shift { font-size: 22px; padding: 0 6px; }
  .caps kbd { min-width: 22px; text-align: center; font-size: 12px; padding: 3px 7px; color: var(--fg); background: var(--bg3); border: 1px solid var(--line, rgba(255,255,255,.12)); border-bottom-width: 2px; }
  .lib { display: flex; height: 100%; }
  .hist { flex: 1; min-width: 0; padding: 40px 56px; display: flex; flex-direction: column; gap: 18px; border-right: 1px solid var(--line); }
  .dl { width: 430px; flex: none; padding: 40px 28px; display: flex; flex-direction: column; gap: 12px; overflow-y: auto; }
  header { display: flex; align-items: center; gap: 12px; }
  .chips { display: flex; gap: 6px; flex-wrap: wrap; }
  .chips button { height: 28px; display: flex; align-items: center; gap: 7px; padding: 0 12px; border-radius: 14px; background: var(--card); border: 1px solid var(--line); font-size: 13px; }
  .chips button.on { background: var(--accent); border-color: var(--accent); color: #111114; font-weight: 600; }
  .chips i, .who i { width: 8px; height: 8px; border-radius: 50%; display: block; }
  .list { flex: 1; overflow-y: auto; }
  .day { padding: 16px 0 8px; color: var(--dim); font: 500 10.5px ui-monospace, 'SF Mono', monospace; letter-spacing: .1em; text-transform: uppercase; }
  .hrow { display: flex; align-items: center; gap: 14px; width: 100%; height: 52px; border-radius: 0; border-bottom: 1px solid var(--line); text-align: left; padding: 0 6px; }
  .hrow:hover { background: var(--bg2); }
  .time { font: 12px ui-monospace, 'SF Mono', monospace; color: var(--dim); width: 70px; flex: none; white-space: nowrap; }
  .pb { width: 7px; height: 7px; border-radius: 50%; flex: none; }
  .t { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
  .empty { padding: 16px 0; }
  .dcard { display: flex; align-items: center; gap: 12px; padding: 14px 16px; }
  .ext { width: 34px; height: 34px; border-radius: 10px; background: var(--bg3); display: grid; place-items: center; font: 10px ui-monospace, 'SF Mono', monospace; color: var(--accent); flex: none; }
  .name { display: block; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .state { font-size: 12px; font-weight: 600; }

  .settings { display: flex; height: 100%; }
  aside { width: 220px; flex: none; padding: 36px 16px; display: flex; flex-direction: column; gap: 4px; border-right: 1px solid var(--line); }
  .title { display: flex; align-items: center; gap: 10px; padding: 0 12px 16px; }
  .title b { display: block; font: 800 18px 'Bricolage Grotesque Variable', system-ui, sans-serif; letter-spacing: -.02em; }
  .title small { font-size: 11px; margin: 0; }
  .logo { width: 28px; height: 28px; border-radius: 9px; background: #111114; color: var(--accent); display: grid; place-items: center; flex: none; }
  .logo.big { width: 56px; height: 56px; border-radius: 16px; }
  .nav { height: 38px; text-align: left; padding: 0 12px; border-radius: 10px; color: var(--dim); font-weight: 500; }
  .nav.on { background: var(--bg3); color: var(--fg); }
  .sec { display: flex; flex-direction: column; gap: 14px; padding-bottom: 48px; width: 100%; max-width: 768px; flex: none; }
  main::-webkit-scrollbar-thumb { background-color: color-mix(in srgb, var(--dim) 35%, transparent); }
  .sec:last-child { min-height: 70vh; }
  /* The scroll area spans to the window edge; the content keeps its column width (.sec). */
  main { position: relative; flex: 1; min-width: 0; padding: 40px 56px; overflow-y: auto; display: flex; flex-direction: column; gap: 14px; align-content: start; scroll-behavior: smooth; }
  .list2 { display: flex; flex-direction: column; }
  .item, .kv { display: flex; align-items: center; gap: 16px; padding: 16px 20px; border-top: 1px solid var(--line); }
  .item:first-child, .kv:first-child { border-top: 0; }
  .item b { display: block; font-weight: 600; }
  .sw { appearance: none; width: 42px; height: 24px; border-radius: 12px; background: var(--bg3); border: 0; position: relative; padding: 0; flex: none; cursor: pointer; }
  .sw::after { content: ''; position: absolute; top: 3px; left: 3px; width: 18px; height: 18px; border-radius: 50%; background: var(--dim); transition: left .12s; }
  .sw:checked { background: var(--accent); }
  .sw:checked::after { left: 21px; background: #111114; }
  .wide { width: 280px; height: 34px; background: var(--bg3); border: 0; }
  .swatches { display: flex; gap: 8px; }
  .swatch { width: 22px; height: 22px; padding: 0; border-radius: 50%; border: 2px solid transparent; outline: 1.5px solid transparent; outline-offset: 2px; }
  .swatch.custom { background: conic-gradient(#ff5f5f, #ffd24a, #5ad19a, #6ea8fe, #b79cff, #ff5f5f); position: relative; overflow: hidden; cursor: pointer; }
  .swatch.custom input { position: absolute; inset: -8px; width: 40px; height: 40px; opacity: 0; cursor: pointer; padding: 0; border: 0; }
  .swatch.on { outline-color: var(--fg); }
  .swatch[data-a='amber'] { background: #ffb020; } .swatch[data-a='blue'] { background: #6ea8fe; } .swatch[data-a='green'] { background: #5ad19a; } .swatch[data-a='pink'] { background: #ff7eb6; } .swatch[data-a='violet'] { background: #b79cff; }
  .item input[type='number'] { width: 64px; height: 30px; background: var(--bg3); border: 0; }
  .segc { display: flex; gap: 2px; padding: 3px; border-radius: 11px; background: var(--bg); border: 1px solid var(--line); }
  .segc button { height: 28px; padding: 0 12px; border-radius: 8px; color: var(--dim); }
  .segc button.on { background: var(--accent); color: #111114; font-weight: 600; }
  .tiles { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; }
  .tile { padding: 18px 20px; color: var(--dim); font-size: 13px; }
  .tile b { display: block; font: 800 38px/1 'Bricolage Grotesque Variable', system-ui, sans-serif; letter-spacing: -.03em; color: var(--fg); margin-bottom: 6px; }
  .kv { justify-content: space-between; padding: 12px 20px; }
  .pgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 12px; }
  .pcard { padding: 18px 20px; display: flex; flex-direction: column; gap: 14px; align-items: flex-start; }
  .phead { display: flex; align-items: center; gap: 10px; width: 100%; font: 600 20px 'Bricolage Grotesque Variable', system-ui, sans-serif; }
  .pi { width: 18px; height: 18px; border-radius: 50%; display: block; flex: none; position: relative; overflow: hidden; cursor: pointer; }
  .pi:hover { outline: 2px solid var(--line); outline-offset: 2px; }
  .pi input { position: absolute; inset: -8px; width: 40px; height: 40px; opacity: 0; cursor: pointer; padding: 0; border: 0; }
  .phead .x { font: 500 12px 'Instrument Sans Variable', system-ui, sans-serif; padding: 2px 8px; }
  .stats { display: flex; gap: 18px; color: var(--dim); font-size: 13px; }
  .stats b { display: block; color: var(--fg); font-size: 16px; }
  .add { border: 1.5px dashed var(--line); background: none; display: grid; place-items: center; min-height: 100px; padding: 12px; border-radius: 18px; color: var(--dim); }
  .row { display: flex; gap: 8px; }
  .table { border-radius: 18px; background: var(--card); border: 1px solid var(--line); overflow: hidden; }
  .trow { display: flex; align-items: center; gap: 12px; min-height: 54px; padding: 0 20px; border-top: 1px solid var(--line); }
  .trow:first-child { border-top: 0; }
  .who { width: 150px; display: flex; align-items: center; gap: 8px; font-weight: 600; }
  .w { width: 90px; }
  .engines { display: flex; gap: 10px; }
  .engine { flex: 1; border-radius: 16px; background: var(--card); border: 1px solid var(--line); padding: 16px; text-align: left; font-weight: 600; }
  .engine small { font-weight: 500; opacity: .7; }
  .engine.on { background: var(--accent); border-color: var(--accent); color: #111114; }
  .tpl { padding: 14px 18px; }
  .tpl h2 { margin-top: 0; }
  .tpl input { width: 100%; background: none; border: 0; padding: 0; }
  .about { display: flex; gap: 18px; align-items: center; padding: 20px; }
  .about b { font: 800 22px 'Bricolage Grotesque Variable', system-ui, sans-serif; display: block; }
  p.meta { line-height: 1.5; margin: 0; }
</style>
