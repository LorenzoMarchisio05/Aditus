//! Scripts injected into page contexts by the renderer process (see `app.rs`).
//! Pings are the only IPC page contexts may use; Rust never trusts their payload.

/// Click/keyboard gesture pings, SPA navigation pings, media state, the selected link, cosmetic ad-hiding CSS.
pub const PAGE: &str = r#"(()=>{
const q=window.cefQuery;if(!q)return;
// Keep the only reference in this closure: page code can't call cefQuery to forge gesture/back/forward pings
// (events are checked with isTrusted below), and the globals stop being a fingerprint.
try{delete window.cefQuery;delete window.cefQueryCancel}catch(e){}
// Builtins captured now, before page code runs (we are injected at context creation), so a page that later patches
// them can't read or forge our pings. Limit: code that ran before us (extensions, earlier inline patches) is out of reach.
const stringify=JSON.stringify,parse=JSON.parse,P=Promise,now=Date.now,st=setTimeout,ct=clearTimeout,si=setInterval;
const send=(cmd,args)=>new P((ok,no)=>{try{q({request:stringify({cmd,args}),onSuccess:ok,onFailure:(c,m)=>no(m)})}catch(e){no(e)}});
const ping=(k,href)=>{send('page_ping',{kind:k,href:href||''}).catch(()=>{})};
// Real clicks only (an ad's synthetic a.click() is untrusted), with the link they landed on: popups and
// navigations that follow may go there and nowhere else (see adblock::hijack).
const lk=e=>{const a=e.composedPath().find(n=>n instanceof HTMLAnchorElement&&/^https?:/.test(n.href)&&!(n.hash&&n.href.split('#')[0]===location.href.split('#')[0]));return a?a.href:''};
for(const t of ['click','auxclick'])addEventListener(t,e=>{if(e.isTrusted)ping('g',lk(e))},true);
if(window===top){let lm=0;addEventListener('pointermove',e=>{const n=now();if(n-lm<120)return;lm=n;send('page_ping',{kind:'m',x:e.clientX/innerWidth,y:e.clientY/innerHeight}).catch(()=>{})},true)}
addEventListener('keydown',e=>{if(e.key==='Enter'&&e.isTrusted)ping('g',lk(e))},true);
// Two-finger horizontal swipe = back/forward, unless something under the pointer can scroll sideways.
if(window===top){let acc=0,until=0,rs;const sx=(e,d)=>{for(let n=e.target;n&&n.nodeType===1;n=n.parentElement){const o=getComputedStyle(n).overflowX;if((o==='auto'||o==='scroll')&&(d<0?n.scrollLeft>0:n.scrollLeft+n.clientWidth<n.scrollWidth-1))return 1}};
addEventListener('wheel',e=>{const n=now();if(!e.isTrusted||n<until||Math.abs(e.deltaX)<Math.abs(e.deltaY)*2||sx(e,e.deltaX))return;acc+=e.deltaX;ct(rs);rs=st(()=>acc=0,150);if(Math.abs(acc)>=150){ping(acc<0?'b':'f');acc=0;until=n+800}},{passive:true})}
addEventListener('submit',e=>{if(e.isTrusted)ping('g',e.target.action);try{const f=e.target,p=f.querySelector&&f.querySelector('input[type=password]');if(p&&p.value){const all=[...f.querySelectorAll('input')],u=all.slice(0,all.indexOf(p)).reverse().find(x=>/^(text|email|tel|)$/.test(x.type)&&x.value);send('page_login',{u:u?u.value:'',p:p.value}).catch(()=>{})}}catch(_){}},true);
for(const [m,k] of [['pushState','u'],['replaceState','r']]){const o=history[m];history[m]=function(){const r=o.apply(this,arguments);ping(k);return r}}
addEventListener('popstate',()=>ping('u'));addEventListener('hashchange',()=>ping('u'));
// Audible = some media element here is playing unmuted. CEF has no audio-state callback (its AudioHandler diverts the sound), so this stays a page-side guess.
{const pl=new Set();let pa=false;const pu=e=>{const m=e.target;if(!(m instanceof HTMLMediaElement))return;pl[m.paused||m.ended||m.muted||!m.volume?'delete':'add'](m);const a=pl.size>0;if(a!==pa){pa=a;ping(a?'p1':'p0')}};
for(const ev of ['playing','pause','ended','volumechange','emptied'])addEventListener(ev,pu,true)}
if(window.top===window){let t,last='';const scr=new Set;addEventListener('scroll',e=>{const n=e.target;if(n instanceof Element&&scr.size<50)scr.add(n)},{passive:true,capture:true});
const snap=()=>{try{const f=[];document.querySelectorAll('input,textarea,select').forEach((e,i)=>{const ty=(e.type||'').toLowerCase();if(['password','hidden','file','submit','button','image','reset'].includes(ty)||/^cc-/.test(e.autocomplete))return;const v=ty==='checkbox'||ty==='radio'?e.checked:e.value;if(v===''||v===false||v==null||(typeof v==='string'&&v.length>5000))return;f.push([i,v])});const m=[];document.querySelectorAll('video,audio').forEach((e,i)=>{if(e.currentTime>1)m.push([i,e.currentTime])});const all=document.querySelectorAll('*'),sc=[];for(const e of scr){if(!e.isConnected){scr.delete(e);continue}if(e.scrollTop||e.scrollLeft){const i=Array.prototype.indexOf.call(all,e);if(i>=0)sc.push([i,e.scrollLeft,e.scrollTop])}}const ss=[];try{for(let i=0;i<sessionStorage.length&&ss.length<50;i++){const k=sessionStorage.key(i),v=sessionStorage.getItem(k);if(k.length<=500&&v!=null&&v.length<=4000)ss.push([k,v])}}catch(e){}const s=stringify({x:scrollX,y:scrollY,f:f.slice(0,100),m:m.slice(0,4),ss,sc:sc.slice(0,50)});if(s!==last&&s.length<60000){last=s;send('page_state',{s}).catch(()=>{})}}catch(e){}};
const lazy=()=>{ct(t);t=st(snap,1000)};
addEventListener('scroll',lazy,{passive:true,capture:true});for(const ev of ['input','change','pause','seeked'])addEventListener(ev,lazy,true);
addEventListener('visibilitychange',()=>{if(document.hidden)snap()});si(()=>{if(!document.hidden)snap()},5000)}
if(window===top){let ll='';const el=n=>n&&(n.nodeType===1?n:n.parentElement);
document.addEventListener('selectionchange',()=>{let u='';try{const s=getSelection();if(s&&!s.isCollapsed){const a=[s.anchorNode,s.focusNode].map(n=>el(n)&&el(n).closest('a[href]')).find(Boolean),all=a?[]:s.getRangeAt(0).cloneContents().querySelectorAll('a[href]'),t=s.toString().trim();u=a?a.href:all.length===1?all[0].href:/^https?:\/\/\S+$/i.test(t)?t:/^www\.\S+\.\S+$/i.test(t)?'https://'+t:''}}catch(e){}if(u!==ll){ll=u;send('page_link',{u}).catch(()=>{})}})}
if(window.top===window){const css=r=>{r=r&&parse(r);if(!r)return;const s=document.createElement('style');s.textContent=r;(document.head||document.documentElement).appendChild(s)};
send('cosmetic_css',{}).then(css).catch(()=>{});
// Generic ##.class / ###id rules: report each class and id once, batched, and get the matching selectors back.
const C=new Set,I=new Set;let nc=[],ni=[],gt=0;
const seen=n=>{if(n.nodeType!==1)return;if(n.id&&!I.has(n.id)){I.add(n.id);ni.push(n.id)}if(n.classList)for(const c of n.classList)if(!C.has(c)){C.add(c);nc.push(c)}};
const scan=n=>{seen(n);if(n.querySelectorAll)n.querySelectorAll('[class],[id]').forEach(seen)};
const flush=()=>{gt=0;if(!nc.length&&!ni.length)return;const c=nc.slice(0,2000),i=ni.slice(0,2000);nc=nc.slice(2000);ni=ni.slice(2000);send('cosmetic_generic',{c,i}).then(css).catch(()=>{});if(nc.length||ni.length)gt=st(flush,500)};
const sched=()=>{if(!gt)gt=st(flush,300)};
const go=()=>{try{scan(document.documentElement);sched();new MutationObserver(m=>{for(const r of m){if(r.type==='attributes')seen(r.target);else r.addedNodes.forEach(scan)}if(nc.length||ni.length)sched()}).observe(document.documentElement,{subtree:true,childList:true,attributes:true,attributeFilter:['class','id']})}catch(e){}};
document.readyState==='loading'?addEventListener('DOMContentLoaded',go):go()}
})();"#;

/// Ad fields YouTube puts in player responses. The one place to edit when YouTube renames them: `adblock::youtube_rules`
/// turns this list into `json-prune` / `set-constant` scriptlet rules (see `scriptlets.rs`), so the stripping itself
/// is generic and goes through the filter pipeline; `YOUTUBE` below is only the DOM fallback.
pub const YT_AD_FIELDS: &[&str] = &["adPlacements", "playerAds", "adSlots", "adBreakParams", "adBreakHeartbeatParams", "adSafetyReason"];

/// YouTube serves ads from its own origin, so network and cosmetic filters can't touch them. Ad data is stripped by the
/// scriptlets (`YT_AD_FIELDS`); this is the thin fallback for what still plays: skip via `.ad-showing` and the skip
/// button, the stall retry, the enforcement wall. Paused per tab via sessionStorage (set by adblock::refresh).
pub const YOUTUBE: &str = r#"(()=>{
if(!/(^|\.)youtube\.com$/.test(location.hostname))return;
try{if(sessionStorage.getItem('__aditus_off'))return}catch(e){}
// Dev diagnostic: the json-prune scriptlet counts what it pruned in window.__aditusPruned.
setTimeout(()=>{try{if(!window.__aditusPruned)console.debug('[aditus] no ad fields pruned after 20s: YouTube may have renamed them (scripts::YT_AD_FIELDS)')}catch(e){}},20000);
// uBO's trick: YouTube holds the video back for the length of an ad it still schedules server-side. Player requests are
// rewritten (V=0 channel screen, then other variants) and, if the video stalls with nothing playing, reloaded with the next one.
let V=0,stall=0;
JSON.stringify=new Proxy(JSON.stringify,{apply(t,th,a){try{const o=a[0],cp=o&&o.attestationRequest&&o.playbackContext&&o.playbackContext.contentPlaybackContext,c=cp&&o.context&&o.context.client;
if(c&&c.clientName==='WEB'&&!/[?&]list=/.test(location.href)&&!location.pathname.startsWith('/shorts/')){if(V===0)c.clientScreen='CHANNEL';if(V<2)cp.lactMilliseconds=String(Date.now());if(V===1)o.params='8AUB';if(V===2)o.params='YAHI';if(V===3)cp.referer=String(cp.referer||'').replace(/(#reloadxhr)?$/,'#reloadxhr')}}catch(e){}return Reflect.apply(t,th,a)}});
// Self-diagnosis: nothing stripped within 20s of a watch page loading hints that YouTube changed the player-response shape.
addEventListener('yt-navigate-finish',()=>{V=0;stall=0;S=0;clearTimeout(dt);if(location.pathname.startsWith('/watch'))dt=setTimeout(()=>{if(!S)console.debug('[aditus] no ad fields stripped on this watch page; player response format may have changed')},20000)});
setInterval(()=>{try{if(document.hidden||!location.pathname.startsWith('/watch'))return;const p=document.getElementById('movie_player'),s=p&&p.getStatsForNerds&&p.getStatsForNerds();if(!s)return;const st=p.getPlayerState();
if(s.buffer_health_seconds==='0.00 s'&&s.resolution==='0x0'&&(st===3||st===-1)&&!p.classList.contains('ad-showing')&&!String(s.debug_info||'').startsWith('SSAP, AD'))stall++;else stall=0;
if(stall>=2&&V<3){const r=p.getPlayerResponse&&p.getPlayerResponse();if(r&&r.videoDetails){V++;stall=0;console.debug('[aditus] stalled, retrying variant',V);p.loadVideoById(r.videoDetails.videoId,(r.playerConfig&&r.playerConfig.playbackStartConfig&&r.playerConfig.playbackStartConfig.startSeconds)||0)}}}catch(e){}},1000);
const st=document.createElement('style');st.textContent='#movie_player.ad-showing video{opacity:0!important}.video-ads,.ytp-ad-module,.ytp-ad-overlay-container,#player-ads,ytd-ad-slot-renderer,ytd-in-feed-ad-layout-renderer,ytd-display-ad-renderer,ytd-promoted-sparkles-web-renderer{display:none!important}';
const add=G(()=>(document.head||document.documentElement).appendChild(st));document.documentElement?add():addEventListener('DOMContentLoaded',add);
let was=0;const skip=()=>{const m=document.querySelector('#movie_player.ad-showing');if(m&&!was)console.debug('[aditus] ad playing, skipping');if(!m){if(was){was=0;const v0=document.querySelector('#movie_player video');if(v0){v0.muted=false;v0.playbackRate=1}const pl=document.querySelector('#movie_player');if(pl&&pl.playVideo)pl.playVideo()}return}was=1;const vd=m.querySelector('video');if(vd&&isFinite(vd.duration)){vd.muted=true;vd.playbackRate=16;vd.currentTime=vd.duration}const b=document.querySelector('.ytp-skip-ad-button,.ytp-ad-skip-button-modern,.ytp-ad-skip-button');if(b)b.click()};
// "Ad blockers violate YouTube's Terms" wall: drop the dialog and its backdrop, let the video play.
const wall=()=>{const w=document.querySelector('ytd-enforcement-message-view-model');if(!w)return;console.debug('[aditus] removing enforcement dialog');(w.closest('tp-yt-paper-dialog')||w).remove();document.querySelectorAll('tp-yt-iron-overlay-backdrop').forEach(e=>e.remove());document.body.style.overflow='';document.documentElement.style.overflow='';const p=document.getElementById('movie_player');if(p&&p.getPlayerState&&p.getPlayerState()===2&&p.playVideo)p.playVideo()};
new MutationObserver(G(wall)).observe(document,{subtree:true,childList:true});setInterval(G(wall),1000);
new MutationObserver(G(skip)).observe(document,{subtree:true,attributes:true,attributeFilter:['class']});for(const ev of ['loadstart','loadedmetadata','durationchange','playing'])document.addEventListener(ev,G(skip),true);setInterval(G(skip),100);
})();"#;

/// Reader view: Mozilla's Readability (Apache-2.0, see resources/reader) plus our renderer; run in the page by the browser process.
pub const READER: &str = concat!(include_str!("../resources/reader/Readability.js"), "\n", include_str!("../resources/reader/reader.js"));
