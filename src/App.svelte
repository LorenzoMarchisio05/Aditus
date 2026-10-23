<script>
  // A site on a real host is remembered (mapping); blank tabs just switch.
  const setProfile = (t, profile) => (t.host ? call('assign_mapping', { host: t.host, profile }) : call('switch_profile', { profile }));
  import { onMount, tick } from 'svelte';
  import { call, on, onState, color } from './lib/api.js';

  const COLLAPSED = 52;
  const TOP = 44;
  const FIND = 36;

  let st = $state({ tabs: [], active: 0, cfg: null, prompt: null, toast: null, adblock_site: true });
  let width = $state(236);
  let resizing = false;
  let urlText = $state('');
  let editing = $state(false);
  let findOpen = $state(false);
  let findText = $state('');
  let menu = $state(null);
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
    document.documentElement.dataset.theme = cfg?.settings.theme ?? 'system';
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

  onMount(() => {
    const offs = [];
    (async () => {
      offs.push(await onState((s) => (st = s)));
      offs.push(
        await on('menu', (id) => {
          if (id === 'focus-address') focusAddress();
          else if (id === 'find') openFind();
          else if (id.startsWith('profile:')) setProfile(tab, id.slice(8));
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

  function ctx(e, t) {
    e.preventDefault();
    menu = { x: Math.min(e.clientX, sidebarW - 150), y: e.clientY, tab: t };
  }

  function groupTab(e, t) {
    e.preventDefault();
    call('group_tab', { id: t.id, group: e.currentTarget.elements.g.value || null });
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

<svelte:window onclick={() => { menu = null; }} />

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

<aside class="side" style="width:{sidebarW}px; padding-top:{mac ? 38 : 0}px">
  {#if mac}<div class="drag" data-tauri-drag-region></div>{/if}
  {#if collapsed}
    <button class="cbtn" onclick={() => settings({ sidebar_collapsed: false })} title="Expand sidebar" aria-label="Expand sidebar">☰</button>
  {/if}

  {#if tabQuery !== null && !collapsed}
    <input class="tabsearch" bind:this={tabSearchEl} bind:value={tabQuery} onkeydown={tabSearchKey} placeholder="Search tabs" spellcheck="false" />
  {/if}

  {#if pinned.length}
    <div class="pins">{#each pinned as t (t.id)}{@render pin(t)}{/each}</div>
  {/if}

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
    <div class="bar" data-tauri-drag-region>
      <button disabled={!tab?.can_back} onclick={() => call('back')} aria-label="Back"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6"/></svg></button>
      <button disabled={!tab?.can_forward} onclick={() => call('forward')} aria-label="Forward"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg></button>
      <button onclick={() => call('reload')} aria-label="Reload"><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 12a8 8 0 1 1-2.6-5.9M20 4v5h-5"/></svg></button>
      <div class="pill">
        {#if tab && !tab.url?.startsWith('aditus://')}
          <button class="seg" style="background:{color(tab.profile)}" title="Profile for this site" onclick={() => call('profile_menu')}>
            {tab.profile}<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><path d="M6 9l6 6 6-6"/></svg>
          </button>
        {/if}
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
          title={st.adblock_site ? 'Ad blocking on for this site' : 'Ad blocking paused for this site'}
          aria-label="Toggle ad blocking"
        ><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"/></svg></button>
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

{#if menu}
  <div class="ctx" style="left:{menu.x}px; top:{menu.y}px">
    <button onclick={() => call('reload')}>Reload</button>
    <button onclick={() => call('pin_tab', { id: menu.tab.id, pinned: !menu.tab.pinned })}>{menu.tab.pinned ? 'Unpin tab' : 'Pin tab'}</button>
    <h6>Add to group</h6>
    {#each groupNames as g}
      <button class:sel={menu.tab.group === g} onclick={() => call('group_tab', { id: menu.tab.id, group: g })}>{g}</button>
    {/each}
    <form onsubmit={(e) => groupTab(e, menu.tab)} onclick={(e) => e.stopPropagation()}>
      <input name="g" placeholder="New group name…" value={menu.tab.group ?? ''} />
    </form>
    <h6>Always open this site in</h6>
    {#each cfg.profiles as p}
      <button class:sel={menu.tab.profile === p.name} onclick={async () => { await call('activate_tab', { id: menu.tab.id }); setProfile(menu.tab, p.name); }}>
        <i style="background:{color(p.name)}"></i>{p.name}
      </button>
    {/each}
    <hr />
    <button onclick={() => call('close_tab', { id: menu.tab.id })}>Close tab</button>
    <button onclick={() => st.tabs.filter((t) => t.id !== menu.tab.id && !t.pinned).forEach((t) => call('close_tab', { id: t.id }))}>Close other tabs</button>
  </div>
{/if}

<style>
  .side { position: fixed; inset: 0 auto 0 0; background: var(--bg2); border-right: 1px solid var(--line); display: flex; flex-direction: column; }
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
  .seg { font-size: 12px; font-weight: 700; color: #111114; padding: 0 12px; height: 100%; border-radius: 0; display: flex; align-items: center; gap: 6px; cursor: pointer; flex: none; }
  .tab.discarded .title { opacity: .55; }
  .fav { width: 16px; height: 16px; flex: none; display: grid; place-items: center; }
  .fav img { width: 16px; height: 16px; border-radius: 3px; }
  .title { flex: 1; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .x { padding: 0 6px; opacity: 0; line-height: 1.2; }
  .tab:hover .x { opacity: 1; }
  .spin { width: 12px; height: 12px; border: 2px solid var(--line); border-top-color: var(--accent); border-radius: 50%; animation: s .7s linear infinite; }
  @keyframes s { to { transform: rotate(360deg); } }
  hr { border: 0; border-top: 1px solid var(--line); margin: 6px 4px; }
  .group { display: flex; align-items: center; gap: 6px; width: 100%; text-align: left; font-size: 10.5px; text-transform: uppercase; letter-spacing: .09em; color: var(--dim); font-weight: 600; padding: 10px 8px 3px; }
  .group i { width: 6px; height: 6px; border-radius: 2px; background: var(--dim); }
  .group span { flex: 1; }
  .group em { font: 500 10.5px ui-monospace, monospace; font-style: normal; }
  .card { margin: 6px; padding: 8px; background: var(--bg3); border-radius: 8px; display: flex; flex-direction: column; gap: 6px; }
  .card.toast { flex-direction: row; align-items: center; justify-content: space-between; }
  .choices { display: flex; flex-wrap: wrap; gap: 4px; }
  .choices button { border: 1px solid; }
  .dim { color: var(--dim); }
  .foot { display: flex; gap: 4px; padding: 8px; }
  .newtab { flex: 1; height: 32px; display: flex; align-items: center; gap: 8px; padding: 0 10px; background: var(--bg3); border-radius: 9px; font-weight: 500; }
  .sq { width: 32px; height: 32px; padding: 0; display: grid; place-items: center; background: var(--bg3); color: var(--dim); border-radius: 9px; }
  kbd { font: 10.5px ui-monospace, monospace; color: var(--dim); }
  .grow { flex: 1; }
  .cbtn { margin: 8px auto 4px; }
  .pins { display: flex; flex-wrap: wrap; gap: 4px; padding: 4px 8px; }
  .pin { flex: 1; min-width: 34px; height: 34px; padding: 0; border-radius: 9px; background: var(--bg3); display: grid; place-items: center; position: relative; opacity: .85; }
  .pin.active { opacity: 1; outline: 1.5px solid var(--accent); }
  .pin img { width: 16px; height: 16px; border-radius: 4px; }
  .pu { position: absolute; left: 6px; right: 6px; bottom: 3px; height: 2.5px; border-radius: 2px; }
  .pill { flex: 1; height: 34px; display: flex; align-items: center; border-radius: 10px; background: var(--bg3); overflow: hidden; margin: 0 6px; }
  .pill form { flex: 1; position: relative; height: 100%; }
  .pill input { width: 100%; height: 100%; border: 0; background: none; padding: 0 12px; font-size: 13px; }
  .pill input.ghost { color: transparent; }
  .shown { position: absolute; inset: 0; padding: 0 12px; display: flex; align-items: center; pointer-events: none; color: var(--dim); font-size: 13px; white-space: nowrap; overflow: hidden; }
  .shown b { color: #fff; font-weight: 600; }
  .pill kbd { padding-right: 12px; }
  .shield { height: 30px; background: #123a2c; color: #5dffb8; border-radius: 9px; padding: 0 10px; }
  .shield.off { background: #3a1620; color: var(--danger); }
  .grip { position: absolute; top: 0; right: -3px; width: 6px; height: 100%; cursor: col-resize; }
  .top { position: fixed; top: 0; right: 0; background: var(--bg2); border-bottom: 1px solid var(--line); }
  .drag { position: absolute; top: 0; left: 0; right: 0; height: 38px; }
  .bar { height: 44px; display: flex; align-items: center; gap: 4px; padding: 0 8px; }
  form { flex: 1; }
  form input { width: 100%; }
  .find { height: 36px; display: flex; align-items: center; justify-content: flex-end; gap: 6px; padding: 0 12px; }
  .find input { width: 260px; height: 28px; background: var(--bg3); border: 0; border-radius: 8px; }
  .find button { width: 28px; height: 28px; padding: 0; background: var(--bg3); border-radius: 8px; }
  .ctx { position: fixed; z-index: 10; background: #1c1530; border-radius: 12px; padding: 6px; display: flex; flex-direction: column; width: 220px; box-shadow: 0 18px 50px rgba(0,0,0,.45); }
  .ctx button { text-align: left; height: 30px; border-radius: 7px; display: flex; align-items: center; gap: 8px; }
  .ctx .sel { background: var(--bg3); font-weight: 600; }
  .ctx i { width: 10px; height: 10px; border-radius: 3px; display: block; }
  .ctx h6 { margin: 6px 10px 2px; font-size: 10.5px; letter-spacing: .09em; text-transform: uppercase; color: var(--dim); font-weight: 600; }
  .ctx hr { margin: 5px 4px; }
  .ctx form { padding: 2px; }
  .ctx input { width: 100%; }
</style>
