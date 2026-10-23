<script>
  import { onMount } from 'svelte';
  import { call, on, color } from './lib/api.js';

  let { view } = $props();
  let st = $state(null);
  let q = $state('');
  let rows = $state([]);
  let downloads = $state([]);
  let newProfile = $state('');
  let newHost = $state('');
  let newHostProfile = $state('');

  const titles = { history: 'History', downloads: 'Downloads', settings: 'Settings' };

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
        await on('state', (s) => {
          st = s;
          newHostProfile ||= s.cfg.profiles[0].name;
        })
      );
      offs.push(await on('downloads-changed', loadDownloads));
      call('get_state'); // after listeners exist, or the reply is lost
    })();
    if (view === 'history') loadHistory();
    if (view === 'downloads') loadDownloads();
    return () => offs.forEach((f) => f());
  });

  const set = (patch) => call('save_settings', { settings: { ...st.cfg.settings, ...patch } });
  const engine = $derived(
    !st ? 'ddg' : st.cfg.settings.search_template.includes('duckduckgo') ? 'ddg' : st.cfg.settings.search_template.includes('google.com/search') ? 'google' : 'custom'
  );
  const when = (t) => new Date(t * 1000).toLocaleString();
</script>

<div class="panel">
  <header>
    <h1>{titles[view]}</h1>
    <button onclick={() => call('hide_panel')} aria-label="Close">✕</button>
  </header>

  {#if view === 'history'}
    <div class="row">
      <input placeholder="Search history" bind:value={q} oninput={loadHistory} />
      <button onclick={async () => { await call('history_clear'); loadHistory(); }}>Clear all</button>
    </div>
    <ul>
      {#each rows as r}
        <li>
          <button class="link" onclick={() => { call('new_tab', { url: r.url }); call('hide_panel'); }}>
            <span class="t">{r.title || r.url}</span><span class="u">{r.url}</span>
          </button>
          <span class="meta" style="color:{color(r.profile)}">{r.profile} · {when(r.visited_at)}</span>
        </li>
      {/each}
    </ul>
  {:else if view === 'downloads'}
    <ul>
      {#each downloads as d}
        <li>
          <span class="t">{d.path.split('/').pop() || d.url}</span>
          <span class="meta">{d.status} · {when(d.started_at)}</span>
          {#if d.path}<button onclick={() => call('reveal_path', { path: d.path })}>Show</button>{/if}
        </li>
      {:else}
        <li class="meta">No downloads yet.</li>
      {/each}
    </ul>
  {:else if view === 'settings' && st}
    <section>
      <h2>Search engine</h2>
      <div class="row">
        <select value={engine} onchange={(e) => {
          const v = e.currentTarget.value;
          if (v === 'ddg') set({ search_template: 'https://duckduckgo.com/?q=%s' });
          if (v === 'google') set({ search_template: 'https://www.google.com/search?q=%s' });
          if (v === 'custom') set({ search_template: 'https://example.com/search?q=%s' });
        }}>
          <option value="ddg">DuckDuckGo</option>
          <option value="google">Google</option>
          <option value="custom">Custom</option>
        </select>
        {#if engine === 'custom'}
          <input class="grow" value={st.cfg.settings.search_template} onchange={(e) => set({ search_template: e.currentTarget.value })} />
        {/if}
      </div>
    </section>

    <section>
      <h2>Memory</h2>
      <label>Discard idle background tabs after
        <input type="number" min="1" max="1440" value={st.cfg.settings.idle_minutes} onchange={(e) => set({ idle_minutes: Math.max(1, +e.currentTarget.value) })} /> minutes
      </label>
      <label><input type="checkbox" checked={st.cfg.settings.adblock} onchange={(e) => set({ adblock: e.currentTarget.checked })} /> Ad blocker enabled</label>
    </section>

    <section>
      <h2>Profiles</h2>
      <ul>
        {#each st.cfg.profiles as p}
          <li><span class="badge" style="background:{color(p.name)}"></span> {p.name}
            <button onclick={() => call('delete_profile', { name: p.name })} disabled={st.cfg.profiles.length < 2 || st.tabs.some((t) => t.profile === p.name)}>Delete</button></li>
        {/each}
      </ul>
      <form class="row" onsubmit={(e) => { e.preventDefault(); call('add_profile', { name: newProfile }); newProfile = ''; }}>
        <input placeholder="New profile name" bind:value={newProfile} /><button>Add</button>
      </form>
    </section>

    <section>
      <h2>Site → profile</h2>
      <ul>
        {#each st.cfg.mappings as m}
          <li><span class="t">{m.pattern}</span> <span class="meta" style="color:{color(m.profile)}">{m.profile}</span>
            <button onclick={() => call('delete_mapping', { pattern: m.pattern })}>Remove</button></li>
        {:else}<li class="meta">None yet. You'll be asked the first time you visit a site.</li>{/each}
      </ul>
      <form class="row" onsubmit={(e) => { e.preventDefault(); if (newHost.trim()) call('assign_mapping', { host: newHost.trim(), profile: newHostProfile }); newHost = ''; }}>
        <input class="grow" placeholder="youtube.com or *.google.com" bind:value={newHost} />
        <select bind:value={newHostProfile}>{#each st.cfg.profiles as p}<option>{p.name}</option>{/each}</select>
        <button>Add</button>
      </form>
    </section>

    <section>
      <h2>Ad blocker paused on</h2>
      <ul>
        {#each st.cfg.allowlist as h}
          <li><span class="t">{h}</span><button onclick={() => call('toggle_allowlist', { host: h })}>Resume</button></li>
        {:else}<li class="meta">No sites.</li>{/each}
      </ul>
    </section>
  {/if}
</div>

<style>
  .panel { height: 100vh; overflow-y: auto; padding: 16px 24px 40px; background: var(--bg); }
  header { display: flex; justify-content: space-between; align-items: center; }
  h1 { font-size: 20px; margin: 0 0 12px; }
  h2 { font-size: 13px; text-transform: uppercase; letter-spacing: .06em; color: var(--dim); margin: 24px 0 8px; }
  section label { display: block; margin: 6px 0; }
  .row { display: flex; gap: 8px; margin: 8px 0; }
  .grow { flex: 1; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { display: flex; align-items: center; gap: 12px; padding: 7px 0; border-bottom: 1px solid var(--line); }
  .t { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .u { color: var(--dim); margin-left: 10px; }
  .meta { color: var(--dim); font-size: 12px; white-space: nowrap; }
  .link { flex: 1; text-align: left; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge { width: 10px; height: 10px; border-radius: 50%; display: inline-block; }
  input[type='number'] { width: 70px; }
</style>
