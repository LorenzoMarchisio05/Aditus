<script>
  import { untrack } from 'svelte';
  import { call } from './lib/api.js';
  // side: roam only in a strip on the right (settings page).
  let { incognito = false, roam = false, side = false, tabs = [], guest = null, ondone, onbeat, perch = null, look = null, pulse = 0, groove = false, watching = null } = $props();
  // perch: sits on a fixed spot (the sidebar's footer line) while a page is open, watching the pointer over it.
  // guest: a one-off visit at a fixed spot (the sidebar) instead of roaming.
  // Swing wanders the bottom of the browser pages, picking something to do every few seconds.
  let floorEl = $state(), x = $state(guest ? 90 : 0), face = $state(roam || perch ? 1 : -1), mode = $state('idle'), dur = $state(0);
  let fade = $state(false), wide = $state(false), hop = $state(false), said = $state(false), startled = $state(false), wake;
  let mascotEl = $state(), gaze = $state([0, 0]), mouseX = 0, calm = 0;
  // Mood (0-100) is remembered between launches; petting raises it, being ignored or poked too much lowers it.
  let mood = $state(55), pets = 0, emo = $state(''), lastTouch = Date.now(), saveT, emoT;
  const feel = $derived(incognito ? 'neutral' : mood >= 75 ? 'happy' : mood <= 20 ? 'sad' : mood <= 38 ? 'bored' : 'neutral');
  const happyFace = $derived(emo === 'love' || emo === 'music');
  const mouth = $derived(mode === 'sleep' ? 'M20 41h4' : mode === 'angry' || (feel === 'sad' && !happyFace) ? 'M19 43q4-3 8 0' : happyFace || feel === 'happy' ? 'M18 39q5 6 10 0' : feel === 'bored' ? 'M20 41h6' : 'M19 40q4 3 8 0');
  const private_ = () => incognito || !!guest;
  function feelMore(d, touch = true) {
    if (private_()) return;
    mood = Math.max(0, Math.min(100, mood + d));
    if (touch) lastTouch = Date.now();
    clearTimeout(saveT);
    saveT = setTimeout(() => call('swing_set', { mood, pets }).catch(() => {}), 1500);
  }
  function feelNow(e, ms) {
    emo = e;
    clearTimeout(emoT);
    emoT = setTimeout(() => (emo = ''), ms);
  }
  $effect(() => {
    if (private_()) return;
    call('swing_get').then((s) => { if (s) { mood = s.mood + (55 - s.mood) * 0.3; pets = s.pets; } }).catch(() => {});
    const t = setInterval(() => { if (Date.now() - lastTouch > 300000 && mood > 15) feelMore(-1, false); }, 60000);
    return () => clearInterval(t);
  });
  // Stroking: sweeping the pointer back and forth over him counts as a pet.
  let strokeX = 0, strokeDir = 0, strokes = 0, strokeAt = 0, petAt = 0;
  function stroke(px) {
    const now = Date.now(), dx = px - strokeX;
    strokeX = px;
    if (Math.abs(dx) < 6) return;
    if (now - strokeAt > 1200) strokes = 0;
    if (Math.sign(dx) !== strokeDir) { strokeDir = Math.sign(dx); strokes++; strokeAt = now; }
    if (strokes >= 4 && now > petAt) { strokes = 0; petAt = now + 2500; pet(); }
  }
  const PETS = ['Hehe.', 'Mmm, nice.', 'More!', 'Happy!', 'Do not stop.'], FAV = ['You are my favorite.', 'Best human.', 'I love it here.'], SOFT = ['…thanks.', 'Okay, a bit better.', 'You came back.'];
  function pet() {
    pets++;
    feelMore(5);
    if (mode === 'sleep' || resting) wake?.();
    feelNow('love', 2200);
    say(pets % 20 === 0 ? pick(FAV) : pick(feel === 'sad' ? SOFT : PETS), 1800);
  }
  // Eyes follow the pointer; coming close makes Swing jump (not more than every few seconds).
  function track(e) {
    if (!mascotEl) return;
    const r = mascotEl.getBoundingClientRect();
    const dx = e.clientX - (r.left + r.width / 2), dy = e.clientY - (r.top + 22);
    const d = Math.hypot(dx, dy) || 1;
    const k = Math.min(2.6, d / 40);
    gaze = [(dx / d) * k, (dy / d) * k];
    mouseX = e.clientX;
    if (!guest && d < 30) stroke(e.clientX);
    if (!guest && d < (roam ? 70 : 45) && mode !== 'sleep' && feel !== 'happy' && Date.now() > calm) {
      calm = Date.now() + 5000;
      startled = true;
      say('Whoa!', 1200);
      setTimeout(() => (startled = false), 600);
    }
  }
  const LINES = ['Open the way.', 'Where to?', 'Try ⌘L, it is fast.', 'I live in the new tab.', 'Door is open.', 'Need a break?', 'Tab hoarder detected.', 'No ads here. Ssh.', 'Walk with me.', 'Stay curious.', 'Did you close that tab?'];
  const HUSH = ['Shh. Nobody saw.', 'Private mode. I saw nothing.', 'Incognito. Very mysterious.', 'Your secret is safe.', 'I have shades on.'];
  // Reactions to the browser: a tab put to sleep, too much memory. They jump the queue of random acts.
  const queue = [];
  let resting = false, angryAt = 0;
  $effect(() => {
    for (const t of tabs) {
    }
    if (tabs.reduce((n, t) => n + t.mem_mb, 0) > 4000 && Date.now() > angryAt) {
      angryAt = Date.now() + 120000;
      queue.push('angry');
    }
    if (queue.length && resting) wake?.();
  });
  let lookAt = 0;
  $effect(() => {
    if (!look) return;
    lookAt = Date.now();
    gaze = [0.8 + look[0] * 1.8, Math.max(-2.6, Math.min(2.6, (look[1] - 0.8) * 3))];
  });
  $effect(() => {
    if (!pulse) return;
    wide = true;
    const t = setTimeout(() => (wide = false), 350);
    return () => clearTimeout(t);
  });
  // Music: notice when a tab starts or stops playing and say what is on.
  let wasGroove = false, grooveAt = 0;
  $effect(() => {
    const g = groove;
    if (g === wasGroove) return;
    wasGroove = g;
    if (incognito) return;
    if (g) {
      grooveAt = Date.now();
      const t = (untrack(() => watching?.title) ?? '').replace(/\s*[-–|]\s*(YouTube|Spotify|SoundCloud|Bandcamp|YouTube Music).*$/i, '').trim().slice(0, 26);
      say(t ? `♪ ${t}` : '♪ Ooh, music!', 3000);
      feelNow('music', 2500);
      feelMore(2);
    } else if (Date.now() - grooveAt > 10000) {
      say('Aw, quiet again.', 2000);
    }
  });
  // What you watch or search (matched locally on the address, never sent anywhere; silent in private windows).
  const SITES = [
    [/youtube\.com\/watch|twitch\.tv|netflix\.|vimeo\.com/, 'Popcorn time.', 'wow', 1, 'sit'],
    [/github\.com|gitlab\.com|stackoverflow\.com|developer\.mozilla|^docs\./, 'Code! I love code.', 'wow', 1],
    [/wikipedia\.org/, 'Learning. Nice.', '', 1],
    [/amazon\.|ebay\.|etsy\.com|aliexpress/, 'Wallet check?', '', 0],
    [/reddit\.com|twitter\.com|^x\.com|facebook\.com|instagram\.com|tiktok\.com/, 'Just five minutes, right?', '', -1],
    [/mail\.google|outlook\.|proton\.me/, 'Inbox zero?', '', 0],
    [/maps\./, 'Adventure!', 'wow', 1, 'jump'],
  ];
  const QUERIES = [
    [/\b(cats?|kittens?|dogs?|puppy|puppies)\b/, 'Aww!', 'love', 4, 'jump'],
    [/\b(recipes?|pizza|pasta|cake|cook\w*)\b/, 'Now I am hungry.', '', 1],
    [/\b(error|exception|bug|crash|segfault|not working|how to fix)\b/, 'Bugs… I will keep watch.', '', -1],
    [/\b(weather|rain|forecast)\b/, 'Take an umbrella.', '', 0],
    [/\b(holidays?|flights?|hotels?|travel|vacation)\b/, 'Take me with you!', 'wow', 1, 'jump'],
  ];
  let seen = '', reactAt = 0;
  $effect(() => {
    const u = watching?.url;
    if (incognito || !perch || !u || u === seen) return;
    seen = u;
    let url;
    try { url = new URL(u); } catch { return; }
    if (!/^https?:$/.test(url.protocol) || Date.now() < reactAt) return;
    const q = (url.searchParams.get('q') ?? '').toLowerCase();
    const search = q && /google\.|bing\.|duckduckgo\.|brave\.com|kagi\.com|ecosia\.|startpage\./.test(url.hostname);
    const hit = (search && QUERIES.find((r) => r[0].test(q))) || SITES.find((r) => r[0].test(url.hostname.replace(/^www\./, '') + url.pathname));
    if (!hit) return;
    reactAt = Date.now() + 30000;
    say(hit[1], 2800);
    if (hit[2]) feelNow(hit[2], 2200);
    feelMore(hit[3]);
    if (hit[4]) { queue.push(hit[4]); if (resting) wake?.(); }
  });
  // Nothing happening on the page: glance around.
  $effect(() => {
    if (!perch) return;
    const t = setInterval(() => { if (Date.now() - lookAt > 4000) gaze = [0.4 + Math.random() * 2.2, Math.random() * 3 - 2]; }, 2600);
    return () => clearInterval(t);
  });
  const NAP = ['Nap time, tab.', 'Shh… bedtime.', 'Sleep tight, tab.'], OOPS = ['Whoops!', 'Oops!', 'I meant that.'];
  const pick = (l) => l[Math.floor(Math.random() * l.length)];
  let sayTimer;
  function say(text, ms = 2400) {
    said = text;
    clearTimeout(sayTimer);
    sayTimer = setTimeout(() => (said = false), ms);
  }
  const HAPPY = ['Today is a good day.', 'I like it here.', 'Life is good!', 'Best browser. I live here.'], SAD = ['Quiet today…', 'Still here.', 'Pet me?', 'I missed you.'], BORED = ['Bored…', 'Anything to watch?', '*yawn*'];
  const line = () => { if (incognito) return pick(HUSH); const l = { happy: HAPPY, sad: SAD, bored: BORED }[feel]; return pick(l && Math.random() < 0.7 ? l : LINES); };
  let pokes = [];
  function poke() {
    const now = Date.now();
    pokes = pokes.filter((t) => now - t < 2500).concat(now);
    hop = true;
    wake?.();
    if (pokes.length >= 4) { say(pick(['Hey!', 'Stop poking!', 'Rude.']), 1400); feelMore(-3); } else { say(line()); feelMore(0.5); }
    setTimeout(() => (hop = false), 600);
  }
  $effect(() => {
    if (!floorEl) return;
    let alive = true, timer;
    const wait = (ms) => new Promise((r) => { wake = r; timer = setTimeout(r, ms); });
    const doing = async (m, ms, rest = true) => { mode = m; await wait(ms); mode = 'idle'; if (rest) { resting = true; await wait(roam ? 500 + Math.random() * 1200 : 6000 + Math.random() * 8000); resting = false; } };
    let chase = false;
    const walk = async () => {
      const max = floorEl.clientWidth - 50;
      let to = Math.random() * max;
      if (chase) to = Math.min(max, Math.max(0, mouseX - floorEl.getBoundingClientRect().left - 25));
      if (Math.abs(to - x) < 80) to = x < max / 2 ? max : 0;
      face = to > x ? 1 : -1;
      dur = Math.abs(to - x) / ({ happy: 95, sad: 45, bored: 55 }[feel] ?? 70);
      mode = 'walk';
      x = to;
      await wait(dur * 1000 + 60);
      mode = 'idle';
      await wait(400);
    };
    const acts = { walk, wave: () => { say('Hi!', 1800); return doing('wave', 1900); }, jump: () => doing('jump', 700), spin: () => doing('spin', 950), dance: () => { say('♪', 2400); return doing('dance', 2600); }, stretch: () => doing('stretch', 1700), sit: () => doing('sit', 3200), sneeze: async () => { say('Achoo!', 1500); await doing('sneeze', 1300); }, follow: async () => { chase = true; await walk(); chase = false; }, sleep: async () => { feelNow('yawn', 1000); say('*yawn*', 1000); await wait(1000); say('Zzz…', 1600); await doing('sleep', 6500); }, trip: async () => { say('Oof.', 1500); await doing('trip', 1700); }, angry: async () => { say('So much RAM!!', 2800); await doing('angry', 3000); }, tuck: async () => { say(pick(NAP), 1800); onbeat?.('tuck'); await doing('tuck', 1700, false); if (Math.random() < 0.7) { say(pick(OOPS), 1500); onbeat?.('trip'); await doing('trip', 1700, false); say('Ow.', 900); await wait(1000); } else { say('Sweet dreams.', 1400); await wait(1500); } }, rest: () => { say(line(), 2800); return doing('idle', 3000); } };
    if (guest) {
      (async () => {
        await wait(60);
        face = -1; dur = 1; mode = 'walk'; x = 0;
        await wait(1100);
        mode = 'idle'; await wait(250);
        await acts.tuck();
        fade = true; face = 1; dur = 1.4; mode = 'walk'; x = 130;
        await wait(1500);
        if (alive) ondone?.();
      })();
      return () => { alive = false; clearTimeout(timer); };
    }
    (async () => {
      let last = '';
      while (alive) {
        const bag = (perch ? ['wave', 'stretch', 'sneeze', 'rest', 'rest', 'jump'] : (roam ? ['walk', 'walk', 'walk', 'follow', 'follow'] : []).concat(['wave', 'jump', 'spin', 'dance', 'stretch', 'sit', 'sneeze', 'sleep', 'rest', 'trip'])).concat({ happy: ['dance', 'jump', 'spin', 'wave'], sad: ['sit', 'sit', 'rest', 'rest'], bored: ['sit', 'stretch', 'sleep'] }[feel]?.filter((k) => !(perch && k === 'sleep')) ?? []).filter((k) => k !== last);
        last = queue.shift() ?? bag[Math.floor(Math.random() * bag.length)];
        await acts[last]();
      }
    })();
    return () => { alive = false; clearTimeout(timer); };
  });
</script>

<svelte:window onpointermove={track} />

<div class="floor" class:corner={!roam} class:side class:small={!!guest || !!perch} style={guest || perch ? `left:${(guest || perch).x}px; top:${(guest || perch).y}px` : ''} bind:this={floorEl} aria-hidden="true">
  <div class="walker" class:fade style="left:{x}px; --d:{dur}s">
    {#if said}<span class="say" class:hop={mode === 'jump' || hop || startled}>{said}</span>{/if}
    <button class="mascot m-{mode}" class:perch={!!perch} class:groove class:hop class:startle={startled} bind:this={mascotEl} onclick={poke} tabindex="-1" aria-label="Swing">
      {#if mode === 'sleep'}<span class="zzz">z<small>z</small></span>{/if}
      {#if mode === 'trip'}<span class="zzz">✦<small>✦</small></span>{/if}
      {#if emo === 'love'}<span class="zzz love">♥<small>♥</small></span>{/if}
      {#if emo === 'music'}<span class="zzz note">♪<small>♫</small></span>{/if}
      {#if mode === 'angry'}<span class="steam">💢</span>{/if}
      <svg class="fig" style="transform: scaleX({face})" width="50" height="52" viewBox="0 0 50 52">
        <defs><linearGradient id="mg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFE3A3"/><stop offset="1" stop-color="#FFB020"/></linearGradient></defs>
        <rect class="leg l1" x="14" y="43" width="6" height="9" rx="3" fill="currentColor"/>
        <rect class="leg l2" x="28" y="43" width="6" height="9" rx="3" fill="currentColor"/>
        <g class="body">
          <path d="M7 43H41A34 34 0 0 0 7 9z" fill="url(#mg)" stroke="url(#mg)" stroke-width="3" stroke-linejoin="round"/>
          {#if incognito}
            <rect x="13" y="30" width="22" height="7" rx="3" fill="#111114"/>
          {:else if happyFace}
            <path d="M15 35q3-4 6 0M25 35q3-4 6 0" fill="none" stroke="#111114" stroke-width="1.8" stroke-linecap="round"/>
          {:else if mode === 'sleep' || emo === 'yawn'}
            <path d="M16 34q2 2 4 0M26 34q2 2 4 0" fill="none" stroke="#111114" stroke-width="1.8" stroke-linecap="round"/>
          {:else}
            <g class="eyes"><circle class="pupil" style="transform: translate({gaze[0] * face}px, {gaze[1]}px)" cx="18" cy="34" r={startled || wide || emo === 'wow' ? 3.2 : 2.2} fill="#111114"/><circle class="pupil" style="transform: translate({gaze[0] * face}px, {gaze[1]}px)" cx="28" cy="34" r={startled || wide || emo === 'wow' ? 3.2 : 2.2} fill="#111114"/></g>
          {/if}
          {#if mode === 'angry'}<path d="M13 29l9 3.5M33 29l-9 3.5" fill="none" stroke="#111114" stroke-width="2" stroke-linecap="round"/>{/if}
          {#if feel === 'sad' && mode !== 'angry' && !happyFace}<path d="M13 31l9-3.5M33 31l-9-3.5" fill="none" stroke="#111114" stroke-width="1.6" stroke-linecap="round"/>{/if}
          {#if emo === 'yawn' || emo === 'wow'}<ellipse cx="23" cy="42" rx={emo === 'yawn' ? 3 : 2} ry={emo === 'yawn' ? 3.4 : 2.6} fill="#111114"/>
          {:else}<path d={mouth} fill="none" stroke="#111114" stroke-width="1.8" stroke-linecap="round"/>{/if}
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
  .floor.small { right: auto; bottom: auto; width: 36px; height: 38px; }
  .small .fig { width: 36px; height: 38px; }
  .small .say { left: auto; right: 0; transform: none; }
  .floor { position: fixed; left: 40px; right: 40px; z-index: 5; bottom: 0; height: 70px; pointer-events: none; }
  .walker { position: absolute; bottom: 0; transition: left var(--d) linear; }
  .walker.fade { opacity: 0; transition: left var(--d) linear, opacity 1.2s ease-in .2s; }
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
  .perch .l1 { animation: dangle 2.6s ease-in-out infinite; }
  .perch .l2 { animation: dangle 2.6s ease-in-out -1.3s infinite; }
  .perch.m-idle { animation: none; }
  .mascot.groove { animation: dance .5s ease-in-out infinite; }
  @keyframes dangle { 50% { transform: rotate(14deg); } }
  .m-angry { animation: shake .12s linear infinite; }
  .m-angry .body { filter: hue-rotate(-28deg) saturate(2.2); }
  .m-angry .arm { animation: swing .25s ease-in-out infinite; }
  .m-tuck { animation: lean 1.6s ease-in-out; }
  .m-tuck .arm { animation: wave .4s ease-in-out 4; }
  .m-trip { animation: trip 1.6s ease-in-out; }
  .steam { position: absolute; left: 30px; top: -10px; font-size: 16px; animation: float 1s ease-out infinite; }
  .startle { animation: hop .45s ease-out; }
  .m-sleep { animation: sleep .8s ease-out forwards; }
  .m-sleep .body { transform-box: fill-box; transform-origin: 50% 100%; }
  .zzz { position: absolute; left: 34px; top: -4px; font: 700 14px 'Bricolage Grotesque Variable', system-ui, sans-serif; color: var(--dim); animation: float 2.4s ease-out 2; }
  .zzz.love { color: #ff6b9a; }
  .zzz.note { color: var(--accent, #ffb020); }
  .zzz small { font-size: 10px; margin-left: 2px; }
  .say { position: absolute; bottom: 100%; left: 50%; transform: translateX(-50%); white-space: nowrap; font-size: 12px; color: var(--fg); background: var(--bg3); border: 1px solid var(--line); border-radius: 8px; padding: 5px 9px; margin-bottom: 4px; }
  @keyframes shake { 25% { transform: translateX(-2px) rotate(-2deg); } 75% { transform: translateX(2px) rotate(2deg); } }
  @keyframes lean { 0%, 100% { transform: none; } 30%, 80% { transform: rotate(14deg) translateX(3px); } }
  @keyframes trip { 0% { transform: none; } 18% { transform: translateX(7px) rotate(12deg); } 38%, 62% { transform: translate(14px, 4px) rotate(86deg); } 78% { transform: translateY(-4px) rotate(-8deg); } 90% { transform: rotate(4deg); } 100% { transform: none; } }
  @keyframes bob { 50% { transform: translateY(-3px) rotate(2deg); } }
  @keyframes breathe { 50% { transform: scaleY(1.03); } }
  @keyframes step { 50% { transform: rotate(28deg); } }
  @keyframes swing { 50% { transform: rotate(-35deg); } }
  @keyframes wave { 0%, 100% { transform: rotate(-70deg); } 50% { transform: rotate(-120deg); } }
  .say.hop { animation: hopsay .6s ease-out; }
  @keyframes hopsay { 40%, 55% { translate: 0 -34px; } }
  @keyframes hop { 40% { transform: translateY(-34px) scaleY(1.08); } 55% { transform: translateY(-34px); } 100% { transform: none; } }
  @keyframes spin { 0%, 100% { transform: translateY(0) rotateY(0); } 40% { transform: translateY(-22px) rotateY(360deg); } }
  @keyframes dance { 0%, 100% { transform: rotate(-9deg) translateY(0); } 50% { transform: rotate(9deg) translateY(-6px); } }
  @keyframes stretch { 0%, 100% { transform: none; } 35% { transform: scale(.9, 1.15); } 60% { transform: scale(1.1, .88); } }
  @keyframes sit { 0%, 100% { transform: translateY(0); } 15%, 85% { transform: translateY(5px) scaleY(.9); } }
  @keyframes sneeze { 0%, 45% { transform: none; } 55% { transform: scale(1.1, .9) rotate(-5deg); } 65% { transform: scale(.95, 1.08) rotate(5deg); } 75% { transform: translateX(-3px); } 85% { transform: translateX(3px); } }
  @keyframes sleep { to { transform: scaleY(.96) translateY(1px); } }
  @keyframes float { from { opacity: 1; transform: translate(0, 0); } to { opacity: 0; transform: translate(8px, -16px); } }
  @keyframes blink { 0%, 94%, 100% { transform: scaleY(1); } 97% { transform: scaleY(.1); } }
  @media (prefers-reduced-motion: reduce) { .mascot, .mascot *, .zzz, .say { animation: none !important; } .walker { transition: none; } }
</style>
