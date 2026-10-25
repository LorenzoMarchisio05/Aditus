//! Scripts injected into page contexts by the renderer process (see `app.rs`).
//! Pings are the only IPC page contexts may use; Rust never trusts their payload.

/// Click/keyboard gesture pings, SPA navigation pings, media state, cosmetic ad-hiding CSS.
pub const PAGE: &str = r#"(()=>{
if(window.__aditus)return;window.__aditus=1;
const q=window.cefQuery;if(!q)return;
const send=(cmd,args)=>new Promise((ok,no)=>{try{q({request:JSON.stringify({cmd,args}),onSuccess:ok,onFailure:(c,m)=>no(m)})}catch(e){no(e)}});
const ping=k=>{send('page_ping',{kind:k}).catch(()=>{})};
addEventListener('click',()=>ping('g'),true);
if(window===top){let lm=0;addEventListener('pointermove',e=>{const n=Date.now();if(n-lm<120)return;lm=n;send('page_ping',{kind:'m',x:e.clientX/innerWidth,y:e.clientY/innerHeight}).catch(()=>{})},true)}
addEventListener('keydown',e=>{if(e.key==='Enter')ping('g')},true);
addEventListener('submit',e=>{ping('g');try{const f=e.target,p=f.querySelector&&f.querySelector('input[type=password]');if(p&&p.value){const all=[...f.querySelectorAll('input')],u=all.slice(0,all.indexOf(p)).reverse().find(x=>/^(text|email|tel|)$/.test(x.type)&&x.value);send('page_login',{u:u?u.value:'',p:p.value}).catch(()=>{})}}catch(_){}},true);
for(const m of ['pushState','replaceState']){const o=history[m];history[m]=function(){const r=o.apply(this,arguments);ping('u');return r}}
addEventListener('popstate',()=>ping('u'));addEventListener('hashchange',()=>ping('u'));
addEventListener('playing',()=>ping('p1'),true);
addEventListener('pause',()=>ping('p0'),true);
addEventListener('ended',()=>ping('p0'),true);
if(window.top===window){let t,last='';
const snap=()=>{try{const f=[];document.querySelectorAll('input,textarea,select').forEach((e,i)=>{const ty=(e.type||'').toLowerCase();if(['password','hidden','file','submit','button','image','reset'].includes(ty)||/^cc-/.test(e.autocomplete))return;const v=ty==='checkbox'||ty==='radio'?e.checked:e.value;if(v===''||v===false||v==null||(typeof v==='string'&&v.length>5000))return;f.push([i,v])});const m=[];document.querySelectorAll('video,audio').forEach((e,i)=>{if(e.currentTime>1)m.push([i,e.currentTime])});const s=JSON.stringify({x:scrollX,y:scrollY,f:f.slice(0,100),m:m.slice(0,4)});if(s!==last&&s.length<60000){last=s;send('page_state',{s}).catch(()=>{})}}catch(e){}};
const lazy=()=>{clearTimeout(t);t=setTimeout(snap,1000)};
addEventListener('scroll',lazy,{passive:true,capture:true});for(const ev of ['input','change','pause','seeked'])addEventListener(ev,lazy,true);
addEventListener('visibilitychange',()=>{if(document.hidden)snap()});setInterval(()=>{if(!document.hidden)snap()},5000)}
if(window.top===window){send('cosmetic_css',{}).then(r=>{const css=r&&JSON.parse(r);if(!css)return;const s=document.createElement('style');s.textContent=css;(document.head||document.documentElement).appendChild(s)}).catch(()=>{})}
})();"#;

/// YouTube serves ads from its own origin, so network and cosmetic filters can't touch them:
/// strip ad data from player responses and skip any ad that still plays.
/// ponytail: hand-written, breaks when YouTube renames fields; port uBO scriptlets if it keeps breaking.
/// Paused per tab via sessionStorage (set by adblock::refresh).
pub const YOUTUBE: &str = r#"(()=>{
if(!/(^|\.)youtube\.com$/.test(location.hostname))return;
try{if(sessionStorage.getItem('__aditus_off'))return}catch(e){}
const K=['adPlacements','playerAds','adSlots','adBreakParams','adBreakHeartbeatParams','adSafetyReason'];
const strip=(o,d=0)=>{if(o&&typeof o==='object'&&d<5){for(const k of K)if(k in o){try{delete o[k]}catch(e){}console.debug('[aditus] stripped',k)}for(const k in o)if(o[k]&&typeof o[k]==='object')strip(o[k],d+1)}return o};
let v;try{Object.defineProperty(window,'ytInitialPlayerResponse',{configurable:true,get:()=>v,set:x=>{v=strip(x)}})}catch(e){}
const p=JSON.parse;JSON.parse=function(){return strip(p.apply(this,arguments))};
const f=window.fetch;window.fetch=function(...a){return f.apply(this,a).then(r=>{try{const u=String(a[0]&&a[0].url||a[0]);if(!/\/youtubei\/v1\/(player|next|browse|search|reel)/.test(u))return r;return r.clone().json().then(j=>new Response(JSON.stringify(strip(j)),{status:r.status,statusText:r.statusText,headers:r.headers})).catch(()=>r)}catch(e){return r}})};
const st=document.createElement('style');st.textContent='#movie_player.ad-showing video{opacity:0!important}.video-ads,.ytp-ad-module,.ytp-ad-overlay-container,#player-ads,ytd-ad-slot-renderer,ytd-in-feed-ad-layout-renderer,ytd-display-ad-renderer,ytd-promoted-sparkles-web-renderer{display:none!important}';
const add=()=>(document.head||document.documentElement).appendChild(st);document.documentElement?add():addEventListener('DOMContentLoaded',add);
let was=0;const skip=()=>{const m=document.querySelector('#movie_player.ad-showing');if(m&&!was)console.debug('[aditus] ad playing, skipping');if(!m){if(was){was=0;const pl=document.querySelector('#movie_player');if(pl&&pl.playVideo)pl.playVideo()}return}was=1;const vd=m.querySelector('video');if(vd&&isFinite(vd.duration)){vd.muted=true;vd.playbackRate=16;vd.currentTime=vd.duration}const b=document.querySelector('.ytp-skip-ad-button,.ytp-ad-skip-button-modern,.ytp-ad-skip-button');if(b)b.click()};
new MutationObserver(skip).observe(document,{subtree:true,attributes:true,attributeFilter:['class']});for(const ev of ['loadstart','loadedmetadata','durationchange','playing'])document.addEventListener(ev,skip,true);setInterval(skip,100);
})();"#;

/// Reader view: Mozilla's Readability (Apache-2.0, see resources/reader) plus our renderer; run in the page by the browser process.
pub const READER: &str = concat!(include_str!("../resources/reader/Readability.js"), "\n", include_str!("../resources/reader/reader.js"));
