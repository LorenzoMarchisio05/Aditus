<script>
  import { onMount } from 'svelte';
  import { call, on } from './lib/api.js';

  let text = $state('');
  let input;
  let url = '';

  onMount(() => {
    const offs = [];
    on('state', (s) => (url = s.tabs.find((t) => t.id === s.active)?.url ?? '')).then((f) => offs.push(f));
    on('palette-open', () => {
      text = url;
      input.focus();
      input.select();
    }).then((f) => offs.push(f));
    call('get_state');
    input.focus();
    return () => offs.forEach((f) => f());
  });

  function submit(e) {
    e.preventDefault();
    call('address', { input: text });
    call('hide_palette');
  }
</script>

<form onsubmit={submit}>
  <input
    bind:this={input}
    bind:value={text}
    placeholder="Search or enter address"
    spellcheck="false"
    onkeydown={(e) => e.key === 'Escape' && call('hide_palette')}
    onblur={() => call('hide_palette')}
  />
</form>

<style>
  form { height: 100vh; padding: 12px; background: var(--bg2); border: 1px solid var(--line); border-radius: 12px; }
  input { width: 100%; height: 100%; font-size: 16px; padding: 0 12px; }
</style>
