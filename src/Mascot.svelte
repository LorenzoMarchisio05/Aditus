<script>
  // side: roam only in a strip on the right (settings page).
  let { incognito = false, roam = false, side = false } = $props();
  // Swing wanders the bottom of the browser pages, picking something to do every few seconds.
  let floorEl = $state(), x = $state(0), face = $state(roam ? 1 : -1), mode = $state('idle'), dur = $state(0);
  let hop = $state(false), said = $state(false), startled = $state(false), wake;
  let mascotEl = $state(), gaze = $state([0, 0]), mouseX = 0, calm = 0;
  // Eyes follow the pointer; coming close makes Swing jump (not more than every few seconds).
  function track(e) {
    if (!mascotEl) return;
    const r = mascotEl.getBoundingClientRect();
    const dx = e.clientX - (r.left + r.width / 2), dy = e.clientY - (r.top + 22);
    const d = Math.hypot(dx, dy) || 1;
    const k = Math.min(2.6, d / 40);
    gaze = [(dx / d) * k, (dy / d) * k];
    mouseX = e.clientX;
    if (d < (roam ? 70 : 45) && mode !== 'sleep' && Date.now() > calm) {
      calm = Date.now() + 5000;
      startled = true;
      say('Whoa!', 1200);
      setTimeout(() => (startled = false), 600);
    }
  }
  const LINES = ['Open the way.', 'Where to?', 'Try ⌘L, it is fast.', 'I live in the new tab.', 'Door is open.', 'Need a break?', 'Tab hoarder detected.', 'No ads here. Ssh.', 'Walk with me.', 'Stay curious.', 'Did you close that tab?'];
  const HUSH = ['Shh. Nobody saw.', 'Private mode. I saw nothing.', 'Incognito. Very mysterious.', 'Your secret is safe.', 'I have shades on.'];
  let sayTimer;
  function say(text, ms = 2400) {
    said = text;
    clearTimeout(sayTimer);
    sayTimer = setTimeout(() => (said = false), ms);
  }
  const line = () => { const l = incognito ? HUSH : LINES; return l[Math.floor(Math.random() * l.length)]; };
  function poke() {
    hop = true;
    wake?.();
    say(line());
    setTimeout(() => (hop = false), 600);
  }
  $effect(() => {
    if (!floorEl) return;
    let alive = true, timer;
    const wait = (ms) => new Promise((r) => { wake = r; timer = setTimeout(r, ms); });
    const doing = async (m, ms) => { mode = m; await wait(ms); mode = 'idle'; await wait(roam ? 500 + Math.random() * 1200 : 6000 + Math.random() * 8000); };
    let chase = false;
    const walk = async () => {
      const max = floorEl.clientWidth - 50;
      let to = Math.random() * max;
      if (chase) to = Math.min(max, Math.max(0, mouseX - floorEl.getBoundingClientRect().left - 25));
      if (Math.abs(to - x) < 80) to = x < max / 2 ? max : 0;
      face = to > x ? 1 : -1;
      dur = Math.abs(to - x) / 70;
      mode = 'walk';
      x = to;
      await wait(dur * 1000 + 60);
      mode = 'idle';
      await wait(400);
    };
    (async () => {
      let last = '';
      while (alive) {
        const acts = { walk, wave: () => { say('Hi!', 1800); return doing('wave', 1900); }, jump: () => doing('jump', 700), spin: () => doing('spin', 950), dance: () => { say('♪', 2400); return doing('dance', 2600); }, stretch: () => doing('stretch', 1700), sit: () => doing('sit', 3200), sneeze: async () => { say('Achoo!', 1500); await doing('sneeze', 1300); }, follow: async () => { chase = true; await walk(); chase = false; }, sleep: () => { say('Zzz…', 1600); return doing('sleep', 6500); }, rest: () => { say(line(), 2800); return doing('idle', 3000); } };
        const bag = (roam ? ['walk', 'walk', 'walk', 'follow', 'follow'] : []).concat(['wave', 'jump', 'spin', 'dance', 'stretch', 'sit', 'sneeze', 'sleep', 'rest']).filter((k) => k !== last);
        last = bag[Math.floor(Math.random() * bag.length)];
        await acts[last]();
      }
    })();
    return () => { alive = false; clearTimeout(timer); };
  });
</script>

<svelte:window onpointermove={track} />

<div class="floor" class:corner={!roam} class:side bind:this={floorEl} aria-hidden="true">
  <div class="walker" style="left:{x}px; --d:{dur}s">
    <button class="mascot m-{mode}" class:hop class:startle={startled} bind:this={mascotEl} onclick={poke} tabindex="-1" aria-label="Swing">
      {#if said}<span class="say">{said}</span>{/if}
      {#if mode === 'sleep'}<span class="zzz">z<small>z</small></span>{/if}
      <svg class="fig" style="transform: scaleX({face})" width="50" height="52" viewBox="0 0 50 52">
        <defs><linearGradient id="mg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFE3A3"/><stop offset="1" stop-color="#FFB020"/></linearGradient></defs>
        <rect class="leg l1" x="14" y="43" width="6" height="9" rx="3" fill="currentColor"/>
        <rect class="leg l2" x="28" y="43" width="6" height="9" rx="3" fill="currentColor"/>
        <g class="body">
          <path d="M7 43H41A34 34 0 0 0 7 9z" fill="url(#mg)" stroke="url(#mg)" stroke-width="3" stroke-linejoin="round"/>
          {#if incognito}
            <rect x="13" y="30" width="22" height="7" rx="3" fill="#111114"/>
          {:else if mode === 'sleep'}
            <path d="M16 34q2 2 4 0M26 34q2 2 4 0" fill="none" stroke="#111114" stroke-width="1.8" stroke-linecap="round"/>
          {:else}
            <g class="eyes"><circle class="pupil" style="transform: translate({gaze[0] * face}px, {gaze[1]}px)" cx="18" cy="34" r={startled ? 3.2 : 2.2} fill="#111114"/><circle class="pupil" style="transform: translate({gaze[0] * face}px, {gaze[1]}px)" cx="28" cy="34" r={startled ? 3.2 : 2.2} fill="#111114"/></g>
          {/if}
          <path d={mode === 'sleep' ? 'M20 41h4' : 'M19 40q4 3 8 0'} fill="none" stroke="#111114" stroke-width="1.8" stroke-linecap="round"/>
        </g>
        <path class="arm" d="M40 34l6 4" stroke="#FFB020" stroke-width="4" stroke-linecap="round"/>
      </svg>
    </button>
  </div>
</div>

<style>
  .floor.corner { left: auto; right: 18px; width: 50px; }
  .floor.side { left: auto; right: 18px; width: 360px; }
  .corner .say { left: auto; right: 0; transform: none; }
  .floor { position: fixed; left: 40px; right: 40px; z-index: 5; bottom: 0; height: 70px; pointer-events: none; }
  .walker { position: absolute; bottom: 0; transition: left var(--d) linear; }
  .mascot { position: relative; display: block; padding: 0; background: none; color: var(--fg); pointer-events: auto; cursor: pointer; transform-origin: 50% 100%; }
  .mascot:hover { background: none; }
  .fig { display: block; overflow: visible; }
  .leg { transform-box: fill-box; transform-origin: 50% 0; }
  .arm { transform-box: fill-box; transform-origin: 0 50%; }
  .eyes { transform-box: fill-box; transform-origin: 50% 50%; animation: blink 5s infinite; }
  .m-walk { animation: bob .5s ease-in-out infinite; }
  .m-walk .l1 { animation: step .5s ease-in-out infinite; }
  .m-walk .l2 { animation: step .5s ease-in-out infinite reverse; }
  .m-walk .arm { animation: swing .5s ease-in-out infinite; }
  .m-idle { animation: breathe 3s ease-in-out infinite; }
  .m-wave .arm { animation: wave .45s ease-in-out 4; }
  .m-jump, .mascot.hop { animation: hop .6s ease-out; }
  .m-spin { animation: spin .9s ease-in-out; }
  .pupil { transition: transform .08s linear; }
  .m-dance { animation: dance .5s ease-in-out 5; }
  .m-stretch { animation: stretch 1.6s ease-in-out; }
  .m-sit .l1, .m-sit .l2 { transform: scaleY(.3); }
  .m-sit { animation: sit 3s ease-in-out; }
  .m-sneeze { animation: sneeze 1.2s ease-in-out; }
  .startle { animation: hop .45s ease-out; }
  .m-sleep { animation: sleep 2.4s ease-in-out infinite; }
  .m-sleep .body { transform-box: fill-box; transform-origin: 50% 100%; }
  .zzz { position: absolute; left: 34px; top: -4px; font: 700 14px 'Bricolage Grotesque Variable', system-ui, sans-serif; color: var(--dim); animation: float 2.4s ease-out infinite; }
  .zzz small { font-size: 10px; margin-left: 2px; }
  .say { position: absolute; bottom: 100%; left: 50%; transform: translateX(-50%); white-space: nowrap; font-size: 12px; color: var(--fg); background: var(--bg3); border: 1px solid var(--line); border-radius: 8px; padding: 5px 9px; margin-bottom: 4px; }
  @keyframes bob { 50% { transform: translateY(-3px) rotate(2deg); } }
  @keyframes breathe { 50% { transform: scaleY(1.03); } }
  @keyframes step { 50% { transform: rotate(28deg); } }
  @keyframes swing { 50% { transform: rotate(-35deg); } }
  @keyframes wave { 0%, 100% { transform: rotate(-70deg); } 50% { transform: rotate(-120deg); } }
  @keyframes hop { 40% { transform: translateY(-34px) scaleY(1.08); } 55% { transform: translateY(-34px); } 100% { transform: none; } }
  @keyframes spin { 0%, 100% { transform: translateY(0) rotateY(0); } 40% { transform: translateY(-22px) rotateY(360deg); } }
  @keyframes dance { 0%, 100% { transform: rotate(-9deg) translateY(0); } 50% { transform: rotate(9deg) translateY(-6px); } }
  @keyframes stretch { 0%, 100% { transform: none; } 35% { transform: scale(.9, 1.15); } 60% { transform: scale(1.1, .88); } }
  @keyframes sit { 0%, 100% { transform: translateY(0); } 15%, 85% { transform: translateY(5px) scaleY(.9); } }
  @keyframes sneeze { 0%, 45% { transform: none; } 55% { transform: scale(1.1, .9) rotate(-5deg); } 65% { transform: scale(.95, 1.08) rotate(5deg); } 75% { transform: translateX(-3px); } 85% { transform: translateX(3px); } }
  @keyframes sleep { 50% { transform: scaleY(.96) translateY(1px); } }
  @keyframes float { from { opacity: 1; transform: translate(0, 0); } to { opacity: 0; transform: translate(8px, -16px); } }
  @keyframes blink { 0%, 94%, 100% { transform: scaleY(1); } 97% { transform: scaleY(.1); } }
  @media (prefers-reduced-motion: reduce) { .mascot, .mascot *, .zzz { animation: none !important; } .walker { transition: none; } }
</style>
