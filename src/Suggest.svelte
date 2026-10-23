<script>
  import { onMount } from 'svelte';
  import { call, on, onState, applyTheme } from './lib/api.js';

  let rows = $state([]), sel = $state(-1), q = $state('');

  const apply = (p) => { rows = p?.rows ?? []; sel = p?.sel ?? -1; q = p?.q ?? ''; };
  // mousedown, not click: the first press moves focus out of the address input, which closes this list.
  const pick = (r) => { call('address', { input: r.url }); };
  const host = (u) => u.replace(/^https?:\/\/(www\.)?/, '').replace(/\/$/, '');
  // a completion reads as what you typed, then the rest in bold
  const rest = (r) => (r.kind === 'search' && r.url.toLowerCase().startsWith(q.toLowerCase()) && r.url.length > q.length ? [r.url.slice(0, q.length), r.url.slice(q.length)] : [r.title, '']);

  onMount(() => {
    const offs = [];
    (async () => {
      offs.push(await onState((s) => applyTheme(s.cfg.settings)));
      offs.push(await on('suggest-rows', apply));
      apply(await call('suggest_get'));
      call('get_state');
    })();
    return () => offs.forEach((f) => f());
  });
</script>

<ul>
  {#each rows as r, i}
    <li class:on={i === sel} class:soft={sel < 0 && i === 0}>
      <button tabindex="-1" onmousedown={(e) => { e.preventDefault(); pick(r); }}>
        <span class="ic">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            {#if r.kind === 'hist'}<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>
            {:else if r.kind === 'go'}<path d="M5 12h14M13 6l6 6-6 6"/>
            {:else}<circle cx="11" cy="11" r="7"/><path d="M20 20l-4-4"/>{/if}
          </svg>
        </span>
        {#if r.kind === 'hist'}
          <span class="t">{r.title || host(r.url)}</span><span class="u">{host(r.url)}</span>
        {:else}
          {@const [a, b] = rest(r)}
          <span class="t">{a}{#if b}<b>{b}</b>{/if}</span>
          {#if i === 0}<span class="u">{r.kind === 'go' ? 'Go to address' : 'Search'}</span>{/if}
        {/if}
      </button>
    </li>
  {/each}
</ul>

<style>
  ul { list-style: none; margin: 0; padding: 7px; height: 100vh; box-sizing: border-box; background: var(--bg2); border: 1px solid var(--line); border-radius: 14px; overflow: hidden; }
  li button { display: flex; align-items: center; gap: 12px; width: 100%; height: 38px; padding: 0 10px; border-radius: 9px; background: none; border: 0; color: var(--fg); text-align: left; cursor: default; }
  li.on button { background: color-mix(in srgb, var(--accent) 20%, transparent); }
  li.soft button, li:not(.on) button:hover { background: color-mix(in srgb, var(--fg) 7%, transparent); }
  .ic { flex: none; display: grid; place-items: center; width: 24px; height: 24px; border-radius: 7px; background: color-mix(in srgb, var(--fg) 8%, transparent); color: var(--dim); }
  li.on .ic { color: var(--accent); }
  .t { flex: 0 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 14px; }
  .t b { font-weight: 650; }
  .u { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; color: var(--dim); }
  .u::before { content: '\2014'; margin-right: 8px; opacity: .5; }
</style>
