<script>
  // A site on a real host is remembered (mapping); blank tabs just switch.
  const setProfile = (t, profile) => (t.host ? call('assign_mapping', { host: t.host, profile }) : call('switch_profile', { profile }));
  import { onMount, tick } from 'svelte';
  import { call, on, onState, color, applyTheme, setProfileColors } from './lib/api.js';

  const COLLAPSED = 52;
  const TOP = 52;
  const FIND = 40;

  let st = $state({ tabs: [], active: 0, cfg: null, prompt: null, toast: null, adblock_site: true });
  let width = $state(236);
  let newer = $state(null); // { latest, url } when a release is available
  let checked = false;
  let resizing = false;
  let urlText = $state('');
  let editing = $state(false);
  let findOpen = $state(false);
  let findText = $state('');
  let newtabOpen = false;
  let tabQuery = $state(null); // null = closed
  let tabSearchEl = $state();
  let dragId = $state(null);
  let addrEl, findEl;
  let toastTimer;

  const cfg = $derived(st.cfg);
  const collapsed = $derived(cfg?.settings.sidebar_collapsed ?? false);
  const topbar = $derived((cfg?.settings.topbar_visible ?? true) || findOpen);
  const sidebarW = $derived(collapsed ? COLLAPSED : width);
  const top = $derived(topbar ? TOP + (findOpen ? FIND : 0) : 0);
  const tab = $derived(st.tabs.find((t) => t.id === st.active));
  const matches = (t) =>
    tabQuery === null || `${t.title} ${t.url} ${t.group ?? ''} ${t.profile}`.toLowerCase().includes(tabQuery.toLowerCase());
  const pinned = $derived(st.tabs.filter((t) => t.pinned && matches(t)));
  let folded = $state(new Set());
  const loose = $derived(st.tabs.filter((t) => !t.pinned && !t.group && matches(t)));
  const groups = $derived(
    [...new Set(st.tabs.filter((t) => !t.pinned && t.group && matches(t)).map((t) => t.group))].map((g) => ({
      name: g,
      tabs: st.tabs.filter((t) => !t.pinned && t.group === g && matches(t)),
    })),
  );
  const groupNames = $derived([...new Set(st.tabs.map((t) => t.group).filter(Boolean))]);
  const blank = $derived(!!tab && !tab.url);
  const rest = $derived([...loose, ...groups.flatMap((g) => g.tabs)]);
  function fold(g) {
    folded.has(g) ? folded.delete(g) : folded.add(g);
    folded = new Set(folded);
  }

  $effect(() => {
    applyTheme(cfg?.settings);
    if (cfg && !resizing) width = cfg.settings.sidebar_width;
  });

  // Brand theme; profile colour only tints the profile chip.
  $effect(() => {
    const r = document.documentElement.style;
    for (const v of ['--bg', '--bg2', '--bg3', '--line']) r.removeProperty(v); // stale tint from older builds
  });

  $effect(() => {
    if (blank && !newtabOpen) call('show_panel', { view: 'newtab' });
    else if (!blank && newtabOpen) call('hide_panel');
    newtabOpen = blank;
  });

  $effect(() => {
    call('set_layout', { sidebar: sidebarW, top });
  });

  $effect(() => {
    if (!editing) urlText = tab?.url ?? '';
  });

  $effect(() => {
    clearTimeout(toastTimer);
    if (st.toast) toastTimer = setTimeout(() => call('dismiss_toast'), 7000);
  });

  function settings(patch) {
    call('save_settings', { settings: { ...cfg.settings, ...patch } });
  }

  async function focusAddress() {
    if (cfg.settings.topbar_visible) {
      await tick();
      addrEl?.focus();
      addrEl?.select();
    } else {
      call('show_palette');
    }
  }

  async function openTabSearch() {
    if (collapsed) settings({ sidebar_collapsed: false });
    tabQuery = '';
    await tick();
    tabSearchEl?.focus();
  }

  function tabSearchKey(e) {
    if (e.key === 'Escape') tabQuery = null;
    if (e.key === 'Enter') {
      const first = [...pinned, ...rest][0];
      if (first) call('activate_tab', { id: first.id });
      tabQuery = null;
    }
  }

  async function openFind() {
    findOpen = true;
    await tick();
    findEl?.focus();
    findEl?.select();
  }

  function closeFind() {
    findOpen = false;
    call('find_clear');
  }

  $effect(() => {
    if (!cfg || checked || !cfg.settings.auto_update) return;
    checked = true;
    call('check_update').then((u) => { if (u?.newer) newer = u; }).catch(() => {});
  });

  onMount(() => {
    const offs = [];
    (async () => {
      offs.push(await onState((s) => { setProfileColors(s.cfg.profiles); st = s; }));
      offs.push(
        await on('menu', (id) => {
          if (id === 'focus-address') focusAddress();
          else if (id === 'find') openFind();
          else if (id.startsWith('profile:')) setProfile(tab, id.slice(8));
          else if (id.startsWith('tabm:')) tabAction(id);
          else if (id === 'search-tabs') openTabSearch();
          else if (id === 'toggle-sidebar') settings({ sidebar_collapsed: !collapsed });
          else if (id === 'toggle-topbar') settings({ topbar_visible: !cfg.settings.topbar_visible });
          else if (['history', 'downloads', 'settings'].includes(id)) call('show_panel', { view: id });
        })
      );
      call('get_state'); // after listeners exist, or the reply is lost
    })();
    return () => offs.forEach((f) => f());
  });

  // inline completion from history: fill the rest of the most recent matching URL, selected so Backspace drops it
  async function complete(e) {
    const el = e.currentTarget, typed = el.value;
    if (e.inputType?.startsWith('delete') || !typed || typed.includes(' ')) return;
    const rows = await call('history_search', { q: typed });
    if (el.value !== typed || el.selectionStart !== typed.length) return;
    const bare = (u) => u.replace(/^https?:\/\/(www\.)?/, '');
    const hit = rows.map((r) => bare(r.url)).find((u) => u.toLowerCase().startsWith(typed.toLowerCase()));
    if (!hit || hit.length === typed.length) return;
    urlText = typed + hit.slice(typed.length);
    await tick();
    el.setSelectionRange(typed.length, urlText.length);
  }

  function submitAddress(e) {
    e.preventDefault();
    call('address', { input: urlText });
    addrEl.blur();
  }

  function startResize(e) {
    e.preventDefault();
    resizing = true;
    const move = (ev) => (width = Math.min(480, Math.max(160, ev.clientX)));
    const up = () => {
      removeEventListener('pointermove', move);
      removeEventListener('pointerup', up);
      resizing = false;
      settings({ sidebar_width: width });
    };
    addEventListener('pointermove', move);
    addEventListener('pointerup', up);
  }

  function drop(e, target) {
    e.preventDefault();
    if (dragId == null || dragId === target.id) return;
    call('reorder_tab', { id: dragId, to: st.tabs.findIndex((t) => t.id === target.id) });
    dragId = null;
  }

  function favicon(t) {
    return t.host ? `https://${t.host}/favicon.ico` : '';
  }

  const ctx = (e, t) => { e.preventDefault(); call('tab_menu', { id: t.id }); };

  let groupPrompt = $state(null); // tab id awaiting a new group name
  async function tabAction(raw) {
    const [, act, tid, ...rest] = raw.split(':');
    const id = +tid, arg = rest.join(':'), t = st.tabs.find((x) => x.id === id);
    if (!t) return;
    if (act === 'reload') { await call('activate_tab', { id }); call('reload'); }
    else if (act === 'pin') call('pin_tab', { id, pinned: !t.pinned });
    else if (act === 'group') call('group_tab', { id, group: arg });
    else if (act === 'ungroup') call('group_tab', { id, group: null });
    else if (act === 'newgroup') { groupPrompt = id; await tick(); document.getElementById('gp')?.focus(); }
    else if (act === 'profile') { await call('activate_tab', { id }); setProfile(t, arg); }
    else if (act === 'close') call('close_tab', { id });
    else if (act === 'closeothers') st.tabs.filter((x) => x.id !== id && !x.pinned).forEach((x) => call('close_tab', { id: x.id }));
  }
  function submitGroup(e) {
    e.preventDefault();
    const name = e.currentTarget.elements.g.value.trim();
    if (name) call('group_tab', { id: groupPrompt, group: name });
    groupPrompt = null;
  }

  const parts = $derived.by(() => {
    try {
      const u = new URL(tab?.url ?? '');
      return { host: u.host, path: (u.pathname === '/' ? '' : u.pathname) + u.search };
    } catch {
      return { host: tab?.url ?? '', path: '' };
    }
  });

  const mac = navigator.platform.includes('Mac'); // traffic lights overlay the sidebar top

</script>

{#snippet pin(t)}
  <button class="pin" class:active={t.id === st.active} title={t.title} onclick={() => call('activate_tab', { id: t.id })} oncontextmenu={(e) => ctx(e, t)}>
    {#if favicon(t)}<img src={favicon(t)} alt="" onerror={(e) => (e.currentTarget.style.display = 'none')} />{/if}
    <span class="pu" style="background:{color(t.profile)}"></span>
  </button>
{/snippet}

{#snippet row(t)}
  <div
    class="tab"
    class:active={t.id === st.active}
    class:discarded={t.discarded}
    draggable="true"
    role="button"
    tabindex="0"
    title={collapsed ? t.title : ''}
    ondragstart={() => (dragId = t.id)}
    ondragover={(e) => e.preventDefault()}
    ondrop={(e) => drop(e, t)}
    onclick={() => call('activate_tab', { id: t.id })}
    onkeydown={(e) => e.key === 'Enter' && call('activate_tab', { id: t.id })}
    oncontextmenu={(e) => ctx(e, t)}
  >
    <span class="fav">
      {#if t.loading}<span class="spin"></span>{:else}{(t.host || t.title || '·')[0].toUpperCase()}{#if favicon(t)}<img class="fi" src={favicon(t)} alt="" onerror={(e) => (e.currentTarget.style.display = 'none')} />{/if}{/if}
    </span>
    {#if !collapsed}
      <span class="title">{t.title}</span>
      <span class="tag" title={t.profile}><i class="dot" style="background:{color(t.profile)}"></i>{t.profile.slice(0, 3).toLowerCase()}</span>
      {#if !t.pinned}
        <button class="x" onclick={(e) => { e.stopPropagation(); call('close_tab', { id: t.id }); }} aria-label="Close tab">×</button>
      {/if}
    {:else}
    {/if}
  </div>
{/snippet}

<aside class="side" class:compact={cfg?.settings.compact_tabs} style="width:{sidebarW}px; padding-top:{mac ? 38 : 0}px">
  {#if mac}<div class="drag" data-tauri-drag-region></div>{/if}
  {#if !collapsed}
    <div class="brand">
      <svg width="24" height="24" viewBox="0 0 100 100" aria-hidden="true"><defs><linearGradient id="gc" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFE3A3"/><stop offset="1" stop-color="#FFB020"/></linearGradient></defs><path d="M28 74V26" stroke="var(--fg)" stroke-width="7" stroke-linecap="round"/><path d="M28 74H74A46 46 0 0 0 28 28z" fill="url(#gc)"/><path d="M28 74H74" stroke="var(--fg)" stroke-width="7" stroke-linecap="round"/></svg>
      Aditus{#if st.incognito}<span class="priv">private</span>{/if}
    </div>
  {/if}
  {#if collapsed}
    <button class="cbtn" onclick={() => settings({ sidebar_collapsed: false })} title="Expand sidebar" aria-label="Expand sidebar">☰</button>
  {/if}

  {#if tabQuery !== null && !collapsed}
    <input class="tabsearch" bind:this={tabSearchEl} bind:value={tabQuery} onkeydown={tabSearchKey} placeholder="Search tabs" spellcheck="false" />
  {/if}

  {#if pinned.length}
    {#if !collapsed}<div class="lbl">PINNED</div>{/if}
    <div class="pins">{#each pinned as t (t.id)}{@render pin(t)}{/each}</div>
  {/if}

  {#if !collapsed}<div class="lbl">TABS · {st.tabs.filter((t) => !t.pinned).length}</div>{/if}
  <div class="tabs">
    {#each loose as t (t.id)}{@render row(t)}{/each}
    {#each groups as g (g.name)}
      {#if !collapsed}
        <button class="group" onclick={() => fold(g.name)}><i></i><span>{g.name}</span><em>{g.tabs.length}</em></button>
      {/if}
      {#each g.tabs as t (t.id)}
        {#if collapsed || !folded.has(g.name) || t.id === st.active}{@render row(t)}{/if}
      {/each}
    {/each}
  </div>

  {#if groupPrompt !== null}
    <form class="card" onsubmit={submitGroup}>
      <input id="gp" name="g" placeholder="Group name" onkeydown={(e) => e.key === 'Escape' && (groupPrompt = null)} onblur={() => (groupPrompt = null)} />
    </form>
  {/if}

  {#if newer && !collapsed}
    <div class="card toast">
      <span>Aditus {newer.latest} is out</span>
      <button onclick={() => { call('new_tab', { url: newer.url }); newer = null; }}>Get</button>
    </div>
  {/if}

  {#if st.perm && !collapsed}
    <div class="card perm">
      <span><b>{st.perm[0]}</b> wants to use your {st.perm[1]}</span>
      <div class="row2"><button onclick={() => call('answer_permission', { allow: false })}>Block</button><button class="ok" onclick={() => call('answer_permission', { allow: true })}>Allow</button></div>
    </div>
  {/if}

  {#if st.toast && !collapsed}
    <div class="card toast">
      <span>{st.toast.message}</span>
      {#if st.toast.undo}<button onclick={() => call('undo_swap')}>Undo</button>{/if}
    </div>
  {/if}

  <div class="foot">
    {#if !collapsed}
      <button class="newtab" onclick={() => call('new_tab', {})}>
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>New tab<span class="grow"></span><kbd>⌘T</kbd>
      </button>
    {/if}
    <button class="sq" onclick={() => call('show_panel', { view: 'history' })} title="History and downloads" aria-label="History and downloads">
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></svg>
    </button>
    <button class="sq" onclick={() => call('show_panel', { view: 'settings' })} title="Settings" aria-label="Settings">
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 7h10M18 7h2M4 17h2M10 17h10"/><circle cx="16" cy="7" r="2"/><circle cx="8" cy="17" r="2"/></svg>
    </button>
  </div>

  {#if !collapsed}<div class="grip" onpointerdown={startResize} role="separator" aria-orientation="vertical"></div>{/if}
</aside>

{#if topbar}
  <header class="top" style="left:{sidebarW}px">
    {#if tab?.loading}<div class="load"></div>{/if}
    <div class="bar" data-tauri-drag-region>
      <button disabled={!tab?.can_back} onclick={() => call('back')} aria-label="Back"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6"/></svg></button>
      <button disabled={!tab?.can_forward} onclick={() => call('forward')} aria-label="Forward"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg></button>
      <button onclick={() => call('reload')} aria-label="Reload"><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 12a8 8 0 1 1-2.6-5.9M20 4v5h-5"/></svg></button>
      {#if tab && tab.url && !tab.url.startsWith('aditus://') && !st.incognito}
        <button class="seg" title="Profile for this site" onclick={() => call('profile_menu')}>
          <i class="dot" style="background:{color(tab.profile)}"></i>{tab.profile}<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><path d="M6 9l6 6 6-6"/></svg>
        </button>
      {/if}
      <div class="pill" class:focus={editing}>
        <form onsubmit={submitAddress}>
          <input
            class:ghost={!editing && tab?.url}
            bind:this={addrEl}
            bind:value={urlText}
            oninput={complete}
            placeholder="Search or enter address"
            onfocus={(e) => { editing = true; e.currentTarget.select(); }}
            onblur={() => (editing = false)}
            spellcheck="false"
            autocapitalize="off"
          />
          {#if !editing && tab?.url}<span class="shown"><b>{parts.host}</b>{parts.path}</span>{/if}
        </form>
        <kbd>⌘L</kbd>
      </div>
      {#if tab?.host}
        <button
          class="shield"
          class:off={!st.adblock_site}
          onclick={() => call('toggle_allowlist', { host: tab.host })}
          title={st.adblock_site ? 'Ads and trackers blocked on this site. Click to pause.' : 'Ad blocking paused on this site. Click to resume.'}
          aria-label="Toggle ad blocking"
        ><svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          {#if st.adblock_site}
            <path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z" fill="currentColor" fill-opacity=".22"/><path d="M8.5 12l2.5 2.5 4.5-5"/>
          {:else}
            <path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"/><path d="M4 4l16 16"/>
          {/if}
        </svg></button>
      {/if}
      <button onclick={() => call('open_devtools')} title="Developer tools" aria-label="Developer tools"><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M8 7l-5 5 5 5M16 7l5 5-5 5"/></svg></button>
    </div>
    {#if findOpen}
      <div class="find">
        <input
          bind:this={findEl}
          bind:value={findText}
          placeholder="Find in page"
          oninput={() => call('find', { text: findText, forward: true })}
          onkeydown={(e) => {
            if (e.key === 'Enter') call('find', { text: findText, forward: !e.shiftKey });
            if (e.key === 'Escape') closeFind();
          }}
        />
        <button onclick={() => call('find', { text: findText, forward: false })} aria-label="Previous match">↑</button>
        <button onclick={() => call('find', { text: findText, forward: true })} aria-label="Next match">↓</button>
        <button onclick={closeFind} aria-label="Close find">✕</button>
      </div>
    {/if}
  </header>
{/if}


<style>
  .side { position: fixed; inset: 0 auto 0 0; background: var(--sb); border-right: 1px solid var(--line); display: flex; flex-direction: column; padding-bottom: 4px; }
  .priv { margin-left: auto; font: 500 10.5px/1 'JetBrains Mono Variable', monospace; letter-spacing: .1em; text-transform: uppercase; color: var(--accent); border: 1px solid var(--accent); border-radius: 6px; padding: 3px 6px; }
  .brand { display: flex; align-items: center; gap: 10px; padding: 8px 18px 12px; font: 800 20px/1 'Bricolage Grotesque Variable', system-ui, sans-serif; letter-spacing: -.02em; flex: none; }
  .lbl { font: 500 10.5px ui-monospace, 'SF Mono', monospace; letter-spacing: .1em; color: var(--dim); padding: 10px 18px 4px; }
  .tabsearch { margin: 0 10px 4px; }
  .tabs { flex: 1; overflow-y: auto; padding: 2px 10px; display: flex; flex-direction: column; gap: 2px; }
  .tab { display: flex; align-items: center; gap: 10px; height: 38px; padding: 0 10px; border-radius: 10px; cursor: default; position: relative; color: var(--dim); flex: none; }
  .tab:hover { background: var(--bg2); color: var(--fg); }
  .tab.active { background: var(--bg3); color: var(--fg); }
  .tab.active::before { content: ''; position: absolute; left: 0; top: 9px; bottom: 9px; width: 3px; border-radius: 2px; background: var(--accent); }
  .tab.discarded { opacity: .55; }
  .compact .tab { height: 30px; }
  .compact .fav { width: 18px; height: 18px; }
  .tag { display: flex; align-items: center; gap: 5px; font: 10px ui-monospace, 'SF Mono', monospace; color: var(--dim); flex: none; }
  .dot { width: 7px; height: 7px; border-radius: 50%; display: inline-block; flex: none; }
  .fav { width: 20px; height: 20px; flex: none; display: grid; place-items: center; border-radius: 6px; background: var(--bg2); border: 1px solid var(--line); font-size: 11px; font-weight: 600; color: var(--fg); overflow: hidden; }
  .fav { position: relative; }
  .fav img { width: 14px; height: 14px; border-radius: 3px; }
  .fav img.fi { position: absolute; inset: 0; width: 100%; height: 100%; padding: 2px; background: var(--bg2); border-radius: 5px; box-sizing: border-box; }
  .title { flex: 1; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; font-weight: 500; }
  .x { padding: 0 6px; opacity: 0; line-height: 1.2; color: var(--dim); }
  .tab:hover .x { opacity: 1; }
  .spin { width: 12px; height: 12px; border: 2px solid var(--line); border-top-color: var(--accent); border-radius: 50%; animation: s .7s linear infinite; }
  @keyframes s { to { transform: rotate(360deg); } }
  hr { border: 0; border-top: 1px solid var(--line); margin: 6px 4px; }
  .group { display: flex; align-items: center; gap: 6px; width: 100%; text-align: left; font: 500 10.5px ui-monospace, 'SF Mono', monospace; text-transform: uppercase; letter-spacing: .1em; color: var(--dim); padding: 10px 10px 3px; }
  .group i { width: 6px; height: 6px; border-radius: 2px; background: var(--dim); }
  .group span { flex: 1; }
  .group em { font-style: normal; }
  .card { margin: 6px 10px; padding: 8px 8px 8px 14px; background: var(--bg3); border-radius: 12px; display: flex; flex-direction: column; gap: 6px; }
  .card.perm { gap: 10px; }
  .card.perm span { overflow-wrap: anywhere; }
  .row2 { display: flex; gap: 8px; justify-content: flex-end; }
  .row2 button { height: 30px; padding: 0 14px; border-radius: 8px; background: var(--bg3); font-weight: 500; }
  .row2 .ok { background: var(--accent); color: #111114; font-weight: 600; }
  .card.toast { flex-direction: row; align-items: center; justify-content: space-between; }
  .card.toast button { color: var(--accent); font-weight: 600; }
  .foot { display: flex; gap: 6px; padding: 10px 10px 6px; margin: 0 0 0; border-top: 1px solid var(--line); }
  .newtab { flex: 1; height: 36px; display: flex; align-items: center; gap: 8px; padding: 0 12px; background: var(--bg2); border-radius: 10px; font-weight: 500; color: var(--dim); }
  .newtab:hover { color: var(--fg); }
  .sq { width: 36px; height: 36px; padding: 0; display: grid; place-items: center; color: var(--dim); border-radius: 10px; }
  kbd { font: 11px ui-monospace, 'SF Mono', monospace; color: var(--dim); background: var(--bg3); border-radius: 6px; padding: 2px 6px; }
  .grow { flex: 1; }
  .cbtn { margin: 8px auto 4px; }
  .pins { display: flex; flex-wrap: wrap; gap: 6px; padding: 2px 10px 6px; }
  .pin { flex: 1; min-width: 38px; height: 38px; padding: 0; border-radius: 10px; background: var(--bg2); display: grid; place-items: center; position: relative; opacity: .85; }
  .pin.active { opacity: 1; background: var(--bg3); outline: 1.5px solid var(--accent); }
  .pin img { width: 16px; height: 16px; border-radius: 4px; }
  .pu { position: absolute; right: 5px; bottom: 5px; width: 7px; height: 7px; border-radius: 50%; }
  .load { position: absolute; left: 0; right: 0; bottom: -1px; height: 2px; overflow: hidden; pointer-events: none; }
  .load::after { content: ""; position: absolute; top: 0; bottom: 0; width: 35%; background: var(--accent); border-radius: 2px; animation: ld 1.1s ease-in-out infinite; }
  @keyframes ld { from { left: -35%; } to { left: 100%; } }
  .top { position: fixed; top: 0; right: 0; background: var(--bg); border-bottom: 1px solid var(--line); }
  .drag { position: absolute; top: 0; left: 0; right: 0; height: 38px; }
  .bar { height: 52px; display: flex; align-items: center; gap: 6px; padding: 0 14px; }
  .bar > button { width: 34px; height: 34px; padding: 0; display: grid; place-items: center; border-radius: 10px; color: var(--dim); }
  .bar > button:hover:not(:disabled) { color: var(--fg); }
  .bar > .seg { width: auto; padding: 0 12px; display: flex; align-items: center; gap: 8px; background: var(--bg2); border: 1px solid var(--line); color: var(--fg); font-weight: 500; flex: none; }
  .pill { flex: 1; height: 36px; display: flex; align-items: center; border-radius: 12px; background: var(--bg2); border: 1px solid var(--line); overflow: hidden; }
  .pill.focus { border-color: var(--accent); }
  .pill form { flex: 1; position: relative; height: 100%; }
  .pill input { width: 100%; height: 100%; border: 0; background: none; padding: 0 14px; font-size: 15px; }
  .pill input.ghost { color: transparent; }
  .shown { position: absolute; inset: 0; padding: 0 14px; display: flex; align-items: center; pointer-events: none; color: var(--dim); font-size: 15px; white-space: nowrap; overflow: hidden; }
  .shown b { color: var(--fg); font-weight: 600; }
  .pill kbd { margin-right: 12px; }
  .bar > .shield { color: var(--accent); background: color-mix(in srgb, var(--accent) 14%, var(--bg2)); border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--line)); }
  .bar > .shield.off { color: var(--dim); background: transparent; border: 1px dashed var(--line); }
  .grip { position: absolute; top: 0; right: -3px; width: 6px; height: 100%; cursor: col-resize; }
  form { flex: 1; }
  form input { width: 100%; }
  .find { height: 40px; display: flex; align-items: center; justify-content: flex-end; gap: 6px; padding: 0 14px; border-top: 1px solid var(--line); }
  .find input { width: 260px; height: 30px; background: var(--bg2); border-radius: 9px; }
  .find button { width: 30px; height: 30px; padding: 0; background: var(--bg2); border-radius: 9px; }
</style>
