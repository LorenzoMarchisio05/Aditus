<script>
  import { onMount } from 'svelte';
  import { call, on, onState, applyTheme } from './lib/api.js';
  import { createSuggest } from './lib/suggest.js';

  let text = $state('');
  let input;
  let url = '';
  const sg = createSuggest({ palette: true, fill: (t) => (text = t) });

  onMount(() => {
    const offs = [];
    (async () => {
      offs.push(await onState((s) => { url = s.tabs.find((t) => t.id === s.active)?.url ?? ''; applyTheme(s.cfg.settings); }));
      offs.push(
        await on('palette-open', () => {
          text = url;
          input.focus();
          input.select();
        })
      );
      call('get_state');
    })();
    input.focus();
    return () => offs.forEach((f) => f());
  });

  function submit(e) {
    e.preventDefault();
    call('address', { input: text });
    call('hide_palette');
  }
  // delayed so a press on a suggestion lands before the palette closes
  const close = () => setTimeout(() => call('hide_palette'), 150);
</script>

<form onsubmit={submit}>
  <input
    bind:this={input}
    bind:value={text}
    placeholder="Search or enter address"
    spellcheck="false"
    oninput={() => sg.input(text)}
    onkeydown={(e) => !sg.key(e) && e.key === 'Escape' && call('hide_palette')}
    onblur={close}
  />
</form>

<style>
  form { height: 100vh; padding: 12px; background: var(--bg2); border: 1px solid var(--line); border-radius: 20px; box-shadow: 0 24px 60px rgba(0,0,0,.5); }
  input { width: 100%; height: 100%; font-size: 18px; padding: 0 18px; background: var(--bg); border: 1.5px solid var(--accent); border-radius: 14px; box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 12%, transparent); }
</style>
