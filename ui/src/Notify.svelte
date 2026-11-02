<script>
  // Notifications for one window, drawn bottom-right over the page area (see `notify_place` in commands.rs).
  // The view reports its height; Rust sizes the overlay to fit and hides it when there is nothing to show.
  import { onMount } from 'svelte';
  import { call, on, onState, applyTheme } from './lib/api.js';

  let st = $state(null);
  let newer = $state(null); // update info when a release is available
  let updating = $state('');
  let offer = $state(null);
  let netUser = $state('');
  let netPass = $state('');
  let stack;
  let toastTimer;

  $effect(() => {
    clearTimeout(toastTimer);
    if (st?.toast) toastTimer = setTimeout(() => call('dismiss_toast'), 7000);
  });

  // A primitive: the effect re-runs only when the setting flips, not on every state broadcast.
  const autoUpdate = $derived(!!st?.cfg.settings.auto_update);
  $effect(() => {
    if (!autoUpdate) return;
    const look = () => call('check_update').then((u) => { if (u?.newer) newer = u; }).catch(() => {});
    look();
    const id = setInterval(look, 6 * 3600 * 1000); // long-running sessions still hear about releases
    return () => clearInterval(id);
  });

  async function installUpdate() {
    updating = 'Downloading…';
    try {
      await call('install_update');
      updating = 'Restarting…';
    } catch (e) {
      updating = '';
      call('new_tab', { url: newer.url }); // fall back to the release page
      newer = null;
    }
  }

  onMount(() => {
    const offs = [];
    // Rust shows the view when the height is > 0, so report every change of the stack.
    const ro = new ResizeObserver(() => call('notify_size', { h: Math.ceil(stack.getBoundingClientRect().height) }));
    ro.observe(stack);
    (async () => {
      offs.push(await onState((s) => { st = s; applyTheme(s.cfg.settings); }));
      offs.push(await on('login-offer', (o) => (offer = o)));
      call('get_state');
    })();
    return () => { ro.disconnect(); offs.forEach((f) => f()); };
  });
</script>

<div class="stack" bind:this={stack}>
  {#if newer}
    <div class="card toast">
      <span>Aditus {newer.latest} is out <button class="link" onclick={() => call('show_panel', { view: 'whatsnew' })}>What's new</button></span>
      {#if updating}<small>{updating}</small>
      {:else if newer.can_install}<button onclick={installUpdate}>Install and restart</button>
      {:else}<button onclick={() => { call('new_tab', { url: newer.url }); newer = null; }}>Get</button>{/if}
    </div>
  {/if}

  {#if st?.perm}
    <div class="card perm">
      <span><b>{st.perm[0]}</b> wants to use your {st.perm[1]}</span>
      <div class="row2"><button onclick={() => call('answer_permission', { allow: false })}>Block</button><button class="ok" onclick={() => call('answer_permission', { allow: true })}>Allow</button></div>
    </div>
  {/if}

  {#if st?.net}
    {#key st.net.id}
      <form class="card perm" onsubmit={(e) => { e.preventDefault(); call('answer_net', { id: st.net.id, allow: true, user: netUser, pass: netPass }); netUser = netPass = ''; }}>
        {#if st.net.cert}
          <span><b>{st.net.host}</b> is not secure: {st.net.detail}. Proceeding may expose your data. Allowed until you quit.</span>
          <div class="row2"><button type="button" class="ok" onclick={() => call('answer_net', { id: st.net.id, allow: false })}>Go back</button><button type="button" onclick={() => call('answer_net', { id: st.net.id, allow: true })}>Proceed anyway</button></div>
        {:else}
          <span><b>{st.net.host}</b> asks you to sign in{st.net.detail ? ` (${st.net.detail})` : ''}</span>
          <input placeholder="Username" autocomplete="off" bind:value={netUser} />
          <input type="password" placeholder="Password" autocomplete="off" bind:value={netPass} />
          <div class="row2"><button type="button" onclick={() => { call('answer_net', { id: st.net.id, allow: false }); netUser = netPass = ''; }}>Cancel</button><button class="ok" type="submit">Sign in</button></div>
        {/if}
      </form>
    {/key}
  {/if}

  {#if offer}
    <div class="card perm">
      <span>Save password for <b>{offer.username || 'this login'}</b> on <b>{offer.origin.replace(/^https?:\/\//, '')}</b>?</span>
      <div class="row2"><button onclick={() => { call('login_dismiss', { label: offer.label }); offer = null; }}>Not now</button><button class="ok" onclick={() => { call('login_save', { label: offer.label }).catch(() => {}); offer = null; }}>Save</button></div>
    </div>
  {/if}

  {#if st?.toast}
    <div class="card toast">
      <span>{st.toast.message}</span>
      {#if st.toast.undo}<button onclick={() => call('undo_swap')}>Undo</button>{/if}
    </div>
  {/if}
</div>

<style>
  :global(html), :global(body) { background: transparent; }
  .stack { display: flex; flex-direction: column; gap: 8px; }
  .card { padding: 10px 10px 10px 14px; background: var(--bg3); border: 1px solid var(--line); border-radius: 14px; box-shadow: 0 8px 24px rgb(0 0 0 / .28); display: flex; flex-direction: column; gap: 10px; }
  .card span { overflow-wrap: anywhere; }
  .row2 { display: flex; gap: 8px; justify-content: flex-end; }
  .row2 button { height: 30px; padding: 0 14px; border-radius: 8px; background: var(--bg2); font-weight: 500; }
  .row2 .ok { background: var(--accent); color: #111114; font-weight: 600; }
  .card.toast { flex-direction: row; align-items: center; justify-content: space-between; gap: 12px; }
  .card.toast button { color: var(--accent); font-weight: 600; flex: none; }
  .card.toast .link { display: block; padding: 0; font-weight: 500; text-decoration: underline; }
  input { width: 100%; }
</style>
