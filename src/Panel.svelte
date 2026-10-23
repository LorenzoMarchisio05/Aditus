<script>
  import { onMount } from 'svelte';
  import { call, on, onState, color } from './lib/api.js';

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
  let ntq = $state('');
  let ab = $state(null);
  let upd = $state(null); // null | 'checking' | result

  const NAV = ['General', 'Profiles and sites', 'Ad blocker', 'Search', 'About'];
  const newtab = $derived(view === 'newtab');
  $effect(() => {
    if (!newtab) document.title = { history: 'History', downloads: 'Downloads', settings: 'Settings' }[view] ?? 'Aditus';
  });
  const closeTab = () => call('close_tab', { id: st.active });
  const library = $derived(view === 'history' || view === 'downloads');

  async function loadHistory() {
    rows = await call('history_search', { q });
  }
  async function loadDownloads() {
    downloads = await call('downloads_list');
  }

  onMount(() => {
    const offs = [];
    (async () => {
      offs.push(
        await onState((s) => {
          st = s;
          newHostProfile ||= s.cfg.profiles[0].name;
        })
      );
      offs.push(await on('downloads-changed', loadDownloads));
      call('get_state'); // after listeners exist, or the reply is lost
    })();
    if (library || newtab) {
      loadHistory();
      loadDownloads();
    }
    return () => offs.forEach((f) => f());
  });

  $effect(() => {
    document.documentElement.dataset.theme = st?.cfg.settings.theme ?? 'system';
  });

  $effect(() => {
    if (view === 'settings' && nav === 'Ad blocker') call('adblock_info').then((i) => (ab = i));
  });
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
  const topSites = $derived.by(() => {
    const m = new Map();
    for (const r of rows) {
      const h = host(r.url);
      if (!h || /^(127\.|localhost)/.test(h)) continue;
      if (!m.has(h)) m.set(h, { host: h, url: r.url, profile: r.profile, n: 0 });
      m.get(h).n++;
    }
    return [...m.values()].sort((a, b) => b.n - a.n).slice(0, 6);
  });
  const count = (name) => st.tabs.filter((t) => t.profile === name).length;
  const sites = (name) => st.cfg.mappings.filter((m) => m.profile === name).length;
</script>

<div class="panel">
  {#if library}
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
            {#if d.path}<button class="ghost" onclick={() => call('reveal_path', { path: d.path })}>Show</button>{/if}
          </div>
        {:else}
          <div class="meta">No downloads yet.</div>
        {/each}
      </section>
    </div>
  {:else if newtab && st}
    <div class="nt">
      <div class="hero"><h1>Aditus</h1><span class="meta big">One window. A different account for every site.</span></div>
      <form class="nsearch" onsubmit={(e) => { e.preventDefault(); call('address', { input: ntq }); call('hide_panel'); }}>
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><circle cx="11" cy="11" r="7"/><path d="M20 20l-4-4"/></svg>
        <!-- svelte-ignore a11y_autofocus -->
        <input autofocus bind:value={ntq} placeholder="Search with {ENGINES.find((e) => e[0] === engine)[1]} or type an address" spellcheck="false" />
        <kbd>⌘L</kbd>
      </form>
      <div class="sites">
        {#each topSites as s}
          <button class="site" onclick={() => { call('new_tab', { url: s.url }); call('hide_panel'); }}>
            <span class="ico"><b>{s.host[0].toUpperCase()}</b><i style="background:{color(s.profile)}"></i></span>
            <span>{s.host.replace(/^www\./, '')}</span>
          </button>
        {/each}
      </div>
    </div>
  {:else if st}
    <div class="settings">
      <aside>
        <div class="title"><span class="logo"><svg viewBox="0 0 100 100" width="70%" height="70%" fill="currentColor" aria-hidden="true"><path d="M46 18h8l15 60H31z" stroke="currentColor" stroke-width="4" stroke-linejoin="round"/><path d="M20 85h60" stroke="currentColor" stroke-width="5" stroke-linecap="round"/></svg></span><span><b>Settings</b><small>Aditus 0.1</small></span></div>
        {#each NAV as n}
          <button class="nav" class:on={nav === n} onclick={() => (nav = n)}>{n}</button>
        {/each}
        <span class="grow"></span>
        <button class="nav" onclick={closeTab}>Close</button>
      </aside>

      <main>
        {#if nav === 'General'}
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
              <span class="grow"><b>Sidebar width</b><small>Drag the sidebar edge, or set it here.</small></span>
              <span><input type="number" min="160" max="480" value={st.cfg.settings.sidebar_width} onchange={(e) => set({ sidebar_width: Math.min(480, Math.max(160, +e.currentTarget.value)) })} /> px</span>
            </label>
            <label class="item">
              <span class="grow"><b>Sleep idle tabs after</b><small>Pinned and playing tabs stay awake.</small></span>
              <span><input type="number" min="1" max="1440" value={st.cfg.settings.idle_minutes} onchange={(e) => set({ idle_minutes: Math.max(1, +e.currentTarget.value) })} /> min</span>
            </label>
          </div>
        {:else if nav === 'Profiles and sites'}
          <h1>Profiles <small>Separate cookies and logins. Each tab runs in one profile.</small></h1>
          <div class="pgrid">
            {#each st.cfg.profiles as p}
              <div class="card pcard" style="border-top-color:{color(p.name)}">
                <div class="phead"><span class="pi" style="background:{color(p.name)}">{p.name[0].toUpperCase()}</span><b class="grow">{p.name}</b></div>
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
          <p class="meta">Login and OAuth hosts (accounts.*, auth.*, sso.*) are exempt automatically, so sign-in flows are never interrupted.</p>
        {:else if nav === 'Ad blocker'}
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
        {:else if nav === 'Search'}
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
        {:else}
          <h1>About</h1>
          <div class="card about">
            <span class="logo big"><svg viewBox="0 0 100 100" width="70%" height="70%" fill="currentColor" aria-hidden="true"><path d="M46 18h8l15 60H31z" stroke="currentColor" stroke-width="4" stroke-linejoin="round"/><path d="M20 85h60" stroke="currentColor" stroke-width="5" stroke-linecap="round"/></svg></span>
            <span class="grow"><b>Aditus 0.1.0</b><small>Open source, GPL-3.0</small></span>
            {#if upd && upd !== 'checking' && upd.newer}
              <button class="solid" onclick={() => call('new_tab', { url: upd.url })}>Get {upd.latest}</button>
            {:else}
              <button class="ghost" onclick={checkUpdate} disabled={upd === 'checking'}>{upd === 'checking' ? 'Checking…' : 'Check for updates'}</button>
            {/if}
          </div>
          {#if upd && upd !== 'checking'}
            <p class="meta">{upd.latest === null ? 'No published release found.' : upd.newer ? `Version ${upd.latest} is available.` : "You're on the latest version."}</p>
          {/if}
          <div class="card list2">
            <div class="kv"><span class="meta">Engine</span><span class="mono">System webview</span></div>
            <div class="kv"><span class="meta">Interface</span><span class="mono">Tauri 2 + Svelte</span></div>
            <div class="kv"><span class="meta">Data</span><span class="mono">config.toml · aditus.db</span></div>
          </div>
        {/if}
      </main>
    </div>
  {/if}
</div>

<style>
  .panel {
    --card: var(--bg3);
    --hi: color-mix(in srgb, var(--fg) 12%, var(--bg3));
    height: 100vh; overflow: hidden; background: var(--bg); color: var(--fg);
  }
  .grow { flex: 1; min-width: 0; }
  small { display: block; color: var(--dim); font-size: 12px; font-weight: 400; margin-top: 2px; }
  .meta { color: var(--dim); font-size: 12px; }
  .mono { font-family: ui-monospace, 'JetBrains Mono', monospace; font-size: 12px; }
  h1 { font-size: 24px; letter-spacing: -.01em; margin: 0; display: flex; align-items: baseline; gap: 12px; }
  h1 small { display: inline; margin: 0; font-size: 12.5px; }
  h2 { font-size: 10.5px; text-transform: uppercase; letter-spacing: .09em; color: var(--dim); margin: 18px 0 8px; font-weight: 600; }
  .card { background: var(--card); border-radius: 14px; }
  .ghost { border: 1px solid var(--line); color: #c9bfe6; font-weight: 500; height: 28px; }
  .solid { background: #f1eefb; color: #111114; font-weight: 700; height: 34px; border-radius: 10px; padding: 0 14px; }
  .solid:hover:not(:disabled) { background: #fff; }
  .x { color: var(--dim); }
  input.search, .row input, .add input { height: 36px; background: var(--card); border: 0; border-radius: 10px; padding: 0 14px; }

  .nt { height: 100%; display: flex; flex-direction: column; align-items: center; padding-top: 150px; gap: 36px; overflow-y: auto; }
  .hero { display: flex; flex-direction: column; align-items: center; gap: 10px; }
  .hero h1 { font-size: 64px; font-weight: 800; letter-spacing: -.03em; line-height: 1; display: block; }
  .big { font-size: 14px; }
  .nsearch { width: 620px; max-width: 90%; height: 52px; border-radius: 14px; background: var(--card); display: flex; align-items: center; gap: 12px; padding: 0 18px; color: var(--dim); }
  .nsearch input { flex: 1; height: 100%; background: none; border: 0; font-size: 15px; color: #f1eefb; padding: 0; }
  kbd { font: 11px ui-monospace, monospace; border: 1px solid var(--line); border-radius: 6px; padding: 2px 6px; }
  .sites { display: flex; gap: 28px; }
  .site { width: 84px; display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 0; font-size: 12px; font-weight: 500; }
  .site > span:last-child { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ico { width: 64px; height: 64px; border-radius: 18px; background: var(--card); display: grid; place-items: center; position: relative; font-size: 22px; }
  .ico i { position: absolute; left: 10px; right: 10px; bottom: 7px; height: 3px; border-radius: 2px; }
  .lib { display: flex; height: 100%; }
  .hist { flex: 1; min-width: 0; padding: 28px 36px; display: flex; flex-direction: column; gap: 14px; border-right: 1px solid var(--line); }
  .dl { width: 430px; flex: none; padding: 28px; display: flex; flex-direction: column; gap: 12px; overflow-y: auto; }
  header { display: flex; align-items: center; gap: 12px; }
  .chips { display: flex; gap: 6px; flex-wrap: wrap; }
  .chips button { height: 26px; display: flex; align-items: center; gap: 6px; padding: 0 10px; border-radius: 7px; background: var(--card); font-size: 12px; }
  .chips button.on { background: #f1eefb; color: #111114; font-weight: 600; }
  .chips i, .who i { width: 8px; height: 8px; border-radius: 2px; display: block; }
  .list { flex: 1; overflow-y: auto; }
  .day { padding: 14px 0 6px; color: var(--dim); font-size: 10.5px; letter-spacing: .09em; text-transform: uppercase; font-weight: 600; }
  .hrow { display: flex; align-items: center; gap: 12px; width: 100%; height: 34px; border-radius: 0; border-bottom: 1px solid var(--card); text-align: left; padding: 0 2px; }
  .time { font-family: ui-monospace, monospace; font-size: 11px; color: var(--dim); width: 70px; flex: none; white-space: nowrap; }
  .pb { width: 4px; height: 16px; border-radius: 2px; flex: none; }
  .t { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
  .empty { padding: 16px 0; }
  .dcard { display: flex; align-items: center; gap: 10px; padding: 12px 14px; }
  .ext { width: 32px; height: 32px; border-radius: 9px; background: var(--hi); display: grid; place-items: center; font-family: ui-monospace, monospace; font-size: 9.5px; color: var(--accent); flex: none; }
  .name { display: block; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .state { font-size: 12px; font-weight: 600; }

  .settings { display: flex; height: 100%; }
  aside { width: 236px; flex: none; padding: 10px 8px; display: flex; flex-direction: column; gap: 2px; }
  .title { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-radius: 12px; background: var(--accent); color: #111114; margin-bottom: 10px; }
  .title b { display: block; font-size: 13.5px; }
  .title small { color: inherit; opacity: .7; font-size: 11px; margin: 0; }
  .logo { width: 28px; height: 28px; border-radius: 9px; background: #111114; color: var(--accent); display: grid; place-items: center; font-weight: 700; }
  .logo.big { width: 56px; height: 56px; border-radius: 16px; background: var(--accent); color: #111114; font-size: 28px; font-weight: 800; }
  .nav { height: 32px; text-align: left; padding: 0 12px; border-radius: 8px; color: #c9bfe6; font-weight: 500; }
  .nav.on { background: var(--hi); color: #fff; font-weight: 600; }
  main { flex: 1; min-width: 0; margin: 8px 8px 8px 0; border-radius: 12px; background: var(--bg2); padding: 32px 44px; overflow-y: auto; display: flex; flex-direction: column; gap: 12px; align-content: start; }
  .list2 { display: flex; flex-direction: column; }
  .item, .kv { display: flex; align-items: center; gap: 14px; padding: 14px 16px; border-top: 1px solid var(--line); }
  .item:first-child, .kv:first-child { border-top: 0; }
  .item b { display: block; font-weight: 600; }
  .sw { appearance: none; width: 38px; height: 22px; border-radius: 11px; background: var(--hi); border: 0; position: relative; padding: 0; flex: none; cursor: pointer; }
  .sw::after { content: ''; position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%; background: #fff; transition: left .12s; }
  .sw:checked { background: var(--accent); }
  .sw:checked::after { left: 19px; }
  .item input[type='number'] { width: 64px; height: 30px; background: var(--hi); border: 0; }
  .segc { display: flex; gap: 2px; padding: 2px; border-radius: 9px; background: var(--bg); }
  .segc button { height: 26px; padding: 0 12px; border-radius: 7px; color: var(--dim); }
  .segc button.on { background: var(--accent); color: #111114; font-weight: 700; }
  .tiles { display: flex; gap: 10px; }
  .tile { flex: 1; padding: 14px; color: var(--dim); }
  .tile b { display: block; font-size: 24px; letter-spacing: -.02em; color: var(--fg); }
  .kv { justify-content: space-between; padding: 11px 16px; }
  .pgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 12px; }
  .pcard { padding: 14px; display: flex; flex-direction: column; gap: 12px; border-top: 3px solid; align-items: flex-start; }
  .phead { display: flex; align-items: center; gap: 10px; width: 100%; }
  .pi { width: 28px; height: 28px; border-radius: 9px; color: #111114; display: grid; place-items: center; font-weight: 700; }
  .stats { display: flex; gap: 18px; color: var(--dim); font-size: 12px; }
  .stats b { display: block; color: #f1eefb; font-size: 16px; }
  .add { border: 1.5px dashed var(--hi); background: none; display: grid; place-items: center; min-height: 120px; padding: 12px; }
  .row { display: flex; gap: 8px; }
  .table { border-radius: 14px; background: var(--card); overflow: hidden; }
  .trow { display: flex; align-items: center; gap: 12px; min-height: 40px; padding: 0 16px; border-top: 1px solid var(--line); }
  .trow:first-child { border-top: 0; }
  .who { width: 150px; display: flex; align-items: center; gap: 8px; font-weight: 600; }
  .w { width: 90px; }
  .engines { display: flex; gap: 8px; }
  .engine { flex: 1; border-radius: 14px; background: var(--card); padding: 14px; text-align: left; font-weight: 600; }
  .engine small { font-weight: 500; opacity: .7; }
  .engine.on { background: var(--accent); color: #111114; }
  .tpl { padding: 12px 14px; }
  .tpl h2 { margin-top: 0; }
  .tpl input { width: 100%; background: none; border: 0; padding: 0; }
  .about { display: flex; gap: 16px; align-items: center; padding: 18px; }
  .about b { font-size: 18px; display: block; }
  p.meta { line-height: 1.5; margin: 0; }
</style>
