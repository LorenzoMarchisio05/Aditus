<script>
  import { onMount, tick } from 'svelte';
  import { call, on, color } from './lib/api.js';

  const COLLAPSED = 52;
  const TOP = 44;
  const FIND = 36;

  let st = $state({ tabs: [], active: 0, cfg: null, prompt: null, toast: null, adblock_site: true });
  let width = $state(Number(localStorage.getItem('sidebar') || 240));
  let urlText = $state('');
  let editing = $state(false);
  let findOpen = $state(false);
  let findText = $state('');
  let menu = $state(null);
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
  const rest = $derived([...loose, ...groups.flatMap((g) => g.tabs)]);
  function fold(g) {
    folded.has(g) ? folded.delete(g) : folded.add(g);
    folded = new Set(folded);
  }

  // Chrome takes the active profile's tint.
  $effect(() => {
    const c = tab ? color(tab.profile) : '#6ea8fe';
    const r = document.documentElement.style;
    r.setProperty('--accent', c);
    r.setProperty('--bg', `color-mix(in srgb, ${c} 7%, #0e0f12)`);
    r.setProperty('--bg2', `color-mix(in srgb, ${c} 9%, #121317)`);
    r.setProperty('--bg3', `color-mix(in srgb, ${c} 18%, #1a1b20)`);
    r.setProperty('--line', `color-mix(in srgb, ${c} 16%, #23242a)`);
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

  onMount(() => {
    const offs = [];
    (async () => {
      offs.push(await on('state', (s) => (st = s)));
      offs.push(
        await on('menu', (id) => {
          if (id === 'focus-address') focusAddress();
          else if (id === 'find') openFind();
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

  function submitAddress(e) {
    e.preventDefault();
    call('address', { input: urlText });
    addrEl.blur();
  }

  function startResize(e) {
    e.preventDefault();
    const move = (ev) => (width = Math.min(480, Math.max(160, ev.clientX)));
    const up = () => {
      removeEventListener('pointermove', move);
      removeEventListener('pointerup', up);
      localStorage.setItem('sidebar', String(width));
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

  function ctx(e, t) {
    e.preventDefault();
    menu = { x: Math.min(e.clientX, sidebarW - 150), y: e.clientY, tab: t };
  }

  function groupTab(e, t) {
    e.preventDefault();
    call('group_tab', { id: t.id, group: e.currentTarget.elements.g.value || null });
  }

  const initial = (n) => n.slice(0, 1).toUpperCase();
</script>

<svelte:window onclick={() => (menu = null)} />

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
    <span class="pbar" style="background:{color(t.profile)}"></span>
    <span class="fav">
      {#if t.loading}<span class="spin"></span>{:else if favicon(t)}<img src={favicon(t)} alt="" onerror={(e) => (e.currentTarget.style.display = 'none')} />{/if}
    </span>
    {#if !collapsed}
      <span class="title">{t.title}{#if t.host}<span class="host"> {t.host}</span>{/if}</span>
      <span class="tag" style="color:{color(t.profile)}" title={t.profile}>{t.profile.slice(0, 3).toUpperCase()}</span>
      {#if !t.pinned}
        <button class="x" onclick={(e) => { e.stopPropagation(); call('close_tab', { id: t.id }); }} aria-label="Close tab">×</button>
      {/if}
    {:else}
    {/if}
  </div>
{/snippet}

<aside class="side" style="width:{sidebarW}px">
  <div class="head">
    <button onclick={() => settings({ sidebar_collapsed: !collapsed })} title="Toggle sidebar" aria-label="Toggle sidebar">☰</button>
    {#if !collapsed}<span class="brand">Aditus</span>{/if}
    <button onclick={() => { call('new_tab', {}); focusAddress(); }} title="New tab" aria-label="New tab">＋</button>
  </div>

  {#if tab && !collapsed}
    <div class="pcard" style="background:{color(tab.profile)}">
      <span class="pinit" style="color:{color(tab.profile)}">{initial(tab.profile)}</span>
      <span class="pname">{tab.profile}<small>{st.tabs.filter((x) => x.profile === tab.profile).length} tabs</small></span>
    </div>
  {/if}

  {#if tabQuery !== null && !collapsed}
    <input class="tabsearch" bind:this={tabSearchEl} bind:value={tabQuery} onkeydown={tabSearchKey} placeholder="Search tabs" spellcheck="false" />
  {/if}

  <div class="tabs">
    {#each pinned as t (t.id)}{@render row(t)}{/each}
    {#if pinned.length}<hr />{/if}
    {#each loose as t (t.id)}{@render row(t)}{/each}
    {#each groups as g (g.name)}
      {#if !collapsed}
        <button class="group" onclick={() => fold(g.name)}>{folded.has(g.name) ? '▸' : '▾'} {g.name} <span>{g.tabs.length}</span></button>
      {/if}
      {#each g.tabs as t (t.id)}
        {#if collapsed || !folded.has(g.name) || t.id === st.active}{@render row(t)}{/if}
      {/each}
    {/each}
  </div>

  {#if st.prompt && !collapsed}
    <div class="card">
      <div>Profile for <b>{st.prompt}</b>?</div>
      <div class="choices">
        {#each cfg.profiles as p}
          <button style="border-color:{color(p.name)}" onclick={() => call('assign_mapping', { host: st.prompt, profile: p.name })}>{p.name}</button>
        {/each}
        <button class="dim" onclick={() => call('skip_prompt')}>Skip</button>
      </div>
    </div>
  {/if}

  {#if st.toast && !collapsed}
    <div class="card toast">
      <span>{st.toast.message}</span>
      {#if st.toast.undo}<button onclick={() => call('undo_swap')}>Undo</button>{/if}
    </div>
  {/if}

  <div class="foot">
    <button onclick={() => call('show_panel', { view: 'history' })} title="History" aria-label="History">◷</button>
    <button onclick={() => call('show_panel', { view: 'downloads' })} title="Downloads" aria-label="Downloads">↓</button>
    <button onclick={() => call('show_panel', { view: 'settings' })} title="Settings" aria-label="Settings">⚙</button>
  </div>

  {#if !collapsed}<div class="grip" onpointerdown={startResize} role="separator" aria-orientation="vertical"></div>{/if}
</aside>

{#if topbar}
  <header class="top" style="left:{sidebarW}px">
    <div class="bar">
      <button disabled={!tab?.can_back} onclick={() => call('back')} aria-label="Back">‹</button>
      <button disabled={!tab?.can_forward} onclick={() => call('forward')} aria-label="Forward">›</button>
      <button onclick={() => call('reload')} aria-label="Reload">↻</button>
      {#if tab}
        <select class="seg" style="background:{color(tab.profile)}" value={tab.profile} onchange={(e) => call('switch_profile', { profile: e.currentTarget.value })} title="Profile for this tab">
          {#each cfg.profiles as p}<option value={p.name}>{p.name}</option>{/each}
        </select>
      {/if}
      <form onsubmit={submitAddress}>
        <input
          bind:this={addrEl}
          bind:value={urlText}
          placeholder="Search or enter address"
          onfocus={(e) => { editing = true; e.currentTarget.select(); }}
          onblur={() => (editing = false)}
          spellcheck="false"
          autocapitalize="off"
        />
      </form>
      {#if tab}
        {#if tab.host}
          <button
            class:off={!st.adblock_site}
            onclick={() => call('toggle_allowlist', { host: tab.host })}
            title={st.adblock_site ? 'Ad blocking on for this site' : 'Ad blocking paused for this site'}
            aria-label="Toggle ad blocking"
          >⛨</button>
        {/if}
      {/if}
      <button onclick={() => call('open_devtools')} title="Developer tools" aria-label="Developer tools">⌥</button>
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

{#if menu}
  <div class="ctx" style="left:{menu.x}px; top:{menu.y}px">
    <button onclick={() => call('pin_tab', { id: menu.tab.id, pinned: !menu.tab.pinned })}>{menu.tab.pinned ? 'Unpin' : 'Pin'}</button>
    <form onsubmit={(e) => groupTab(e, menu.tab)} onclick={(e) => e.stopPropagation()}>
      <input name="g" placeholder="Group name" value={menu.tab.group ?? ''} />
    </form>
    <button onclick={() => call('close_tab', { id: menu.tab.id })}>Close</button>
  </div>
{/if}

<style>
  .side { position: fixed; inset: 0 auto 0 0; background: var(--bg2); border-right: 1px solid var(--line); display: flex; flex-direction: column; }
  .head { display: flex; align-items: center; gap: 4px; padding: 8px 8px 6px; }
  .brand { flex: 1; font-weight: 600; letter-spacing: .06em; color: var(--dim); }
  .tabsearch { margin: 0 8px 4px; }
  .tabs { flex: 1; overflow-y: auto; padding: 4px 6px; }
  .tab { display: flex; align-items: center; gap: 8px; height: 32px; padding: 0 8px; border-radius: 8px; cursor: default; position: relative; }
  .tab:hover { background: var(--bg3); }
  .tab { padding-left: 0; overflow: hidden; }
  .tab.active { background: var(--bg3); }
  .pbar { width: 4px; align-self: stretch; flex: none; opacity: .5; }
  .tab.active .pbar { opacity: 1; }
  .host { color: var(--dim); font-weight: 400; margin-left: 6px; }
  .tag { font-size: 9.5px; font-weight: 700; letter-spacing: .03em; flex: none; }
  .pcard { display: flex; align-items: center; gap: 10px; margin: 0 8px 6px; padding: 7px 10px; border-radius: 12px; color: #120d1d; }
  .pinit { width: 26px; height: 26px; border-radius: 8px; background: #120d1d; display: grid; place-items: center; font-weight: 700;}
  .pname { display: flex; flex-direction: column; font-weight: 700; line-height: 1.15; }
  .pname small { font-weight: 500; opacity: .7; font-size: 11px; }
  .seg { font-size: 12px; font-weight: 700; color: #120d1d; padding: 0 12px; height: 34px; border: 0; border-radius: 10px 0 0 10px; margin-right: -4px; }
  .tab.discarded .title { opacity: .55; }
  .fav { width: 16px; height: 16px; flex: none; display: grid; place-items: center; }
  .fav img { width: 16px; height: 16px; border-radius: 3px; }
  .title { flex: 1; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .x { padding: 0 6px; opacity: 0; line-height: 1.2; }
  .tab:hover .x { opacity: 1; }
  .spin { width: 12px; height: 12px; border: 2px solid var(--line); border-top-color: var(--accent); border-radius: 50%; animation: s .7s linear infinite; }
  @keyframes s { to { transform: rotate(360deg); } }
  hr { border: 0; border-top: 1px solid var(--line); margin: 6px 4px; }
  .group { background: none; border: 0; width: 100%; text-align: left; cursor: pointer; font-size: 11px; text-transform: uppercase; letter-spacing: .06em; color: var(--dim); padding: 8px 8px 2px; }
  .card { margin: 6px; padding: 8px; background: var(--bg3); border-radius: 8px; display: flex; flex-direction: column; gap: 6px; }
  .card.toast { flex-direction: row; align-items: center; justify-content: space-between; }
  .choices { display: flex; flex-wrap: wrap; gap: 4px; }
  .choices button { border: 1px solid; }
  .dim { color: var(--dim); }
  .foot { display: flex; justify-content: space-around; padding: 6px; border-top: 1px solid var(--line); }
  .grip { position: absolute; top: 0; right: -3px; width: 6px; height: 100%; cursor: col-resize; }
  .top { position: fixed; top: 0; right: 0; background: var(--bg2); border-bottom: 1px solid var(--line); }
  .bar { height: 44px; display: flex; align-items: center; gap: 4px; padding: 0 8px; }
  form { flex: 1; }
  form input { width: 100%; }
  .off { color: var(--danger); }
  .find { height: 36px; display: flex; align-items: center; gap: 4px; padding: 0 8px; border-top: 1px solid var(--line); }
  .find input { flex: 1; }
  .ctx { position: fixed; z-index: 10; background: var(--bg3); border: 1px solid var(--line); border-radius: 8px; padding: 4px; display: flex; flex-direction: column; min-width: 130px; }
  .ctx button { text-align: left; }
  .ctx form { padding: 2px; }
  .ctx input { width: 100%; }
</style>
