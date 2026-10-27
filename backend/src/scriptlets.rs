//! Scriptlets for `example.com##+js(name, args...)` filter rules (uBlock Origin syntax).
//!
//! Trust: this code is bundled here and chosen by name only. Filter lists (data) pick a scriptlet and
//! pass string arguments; `adblock` quotes those into the call. Nothing from a page ever reaches it.
//! Written for Aditus, not copied from uBO. Only the commonly used ones; unknown names are ignored.

use adblock::resources::{MimeType, Resource, ResourceType};
use base64::{engine::general_purpose::STANDARD, Engine as _};

/// Shared helpers, pulled in as dependencies (`fn/javascript`).
const LIB: &[(&str, &str)] = &[
    ("aditus-re.fn", r#"function aditusRe(s){s=String(s||'');const m=/^\/(.+)\/([gimsu]*)$/.exec(s);try{return m?new RegExp(m[1],m[2].replace('g','')):new RegExp(s.replace(/[.*+?^${}()|[\]\\]/g,'\\$&'))}catch(e){return /^$/}}"#),
    // Replaces the property at dotted path `chain` on window with an accessor, now or once its parents exist.
    ("aditus-trap.fn", r#"function aditusTrap(chain,get,set){const p=String(chain).split('.');const walk=(o,i)=>{if(!o||(typeof o!=='object'&&typeof o!=='function'))return;const k=p[i];let v=o[k];if(i===p.length-1){Object.defineProperty(o,k,{configurable:true,enumerable:true,get:()=>get(v),set:x=>{v=set(v,x)}});return}if(v&&(typeof v==='object'||typeof v==='function'))walk(v,i+1);else Object.defineProperty(o,k,{configurable:true,enumerable:true,get:()=>v,set:x=>{v=x;try{walk(x,i+1)}catch(e){}}})};walk(window,0)}"#),
    ("aditus-prune.fn", r#"function aditusPrune(o,p,del){if(!o||typeof o!=='object')return false;const k=p[0],ks=(k==='*'||k==='[]')?Object.keys(o):(k in o?[k]:[]);let hit=false;for(const n of ks){if(p.length===1){hit=true;if(del)delete o[n]}else if(aditusPrune(o[n],p.slice(1),del))hit=true}return hit}"#),
    ("aditus-dom.fn", r#"function aditusDom(run){const go=()=>{try{run()}catch(e){}};let t=0;const later=()=>{if(!t)t=setTimeout(()=>{t=0;go()},50)};go();document.addEventListener('DOMContentLoaded',go);new MutationObserver(later).observe(document.documentElement,{childList:true,subtree:true,attributes:true})}"#),
];

/// (name, aliases, dependencies, source)
const SCRIPTLETS: &[(&str, &[&str], &[&str], &str)] = &[
    ("abort-on-property-read", &["aopr"], &["aditus-trap.fn"],
     r#"function aditusAopr(p){aditusTrap(p,()=>{throw new ReferenceError('')},(v,x)=>x)}"#),
    ("abort-on-property-write", &["aopw"], &["aditus-trap.fn"],
     r#"function aditusAopw(p){aditusTrap(p,v=>v,()=>{throw new ReferenceError('')})}"#),
    ("abort-current-script", &["acs", "abort-current-inline-script", "acis"], &["aditus-trap.fn", "aditus-re.fn"],
     r#"function aditusAcs(p,n){const re=aditusRe(n),me=document.currentScript;aditusTrap(p,v=>{const s=document.currentScript;if(s&&s!==me&&(re.test(s.textContent)||re.test(s.src)))throw new ReferenceError('');return v},(v,x)=>x)}"#),
    ("set-constant", &["set"], &["aditus-trap.fn"],
     r#"function aditusSet(p,val){const m={'undefined':undefined,'false':false,'true':true,'null':null,'':'',emptyStr:'',emptyArr:[],emptyObj:{},noopFunc:()=>{},trueFunc:()=>true,falseFunc:()=>false};const x=val in m?m[val]:/^-?\d{1,9}$/.test(val)?+val:undefined;aditusTrap(p,()=>x,v=>v)}"#),
    ("no-setTimeout-if", &["nostif", "setTimeout-defuser"], &["aditus-re.fn"],
     r#"function aditusNostif(n,d){const re=aditusRe(n),o=window.setTimeout;window.setTimeout=new Proxy(o,{apply(t,th,a){const f=a[0],s=typeof f==='function'?Function.prototype.toString.call(f):String(f);if(re.test(s)&&(!d||String(a[1])===String(d)))return;return Reflect.apply(t,th,a)}})}"#),
    ("no-setInterval-if", &["nosiif", "setInterval-defuser"], &["aditus-re.fn"],
     r#"function aditusNosiif(n,d){const re=aditusRe(n),o=window.setInterval;window.setInterval=new Proxy(o,{apply(t,th,a){const f=a[0],s=typeof f==='function'?Function.prototype.toString.call(f):String(f);if(re.test(s)&&(!d||String(a[1])===String(d)))return;return Reflect.apply(t,th,a)}})}"#),
    ("json-prune", &[], &["aditus-prune.fn"],
     r#"function aditusJsonPrune(prune,need){const sp=s=>String(s||'').split(/ +/).filter(Boolean).map(x=>x.split('.')),P=sp(prune),N=sp(need),o=JSON.parse;JSON.parse=new Proxy(o,{apply(t,th,a){const r=Reflect.apply(t,th,a);if(N.length&&!N.every(p=>aditusPrune(r,p,false)))return r;for(const p of P)aditusPrune(r,p,true);return r}})}"#),
    ("remove-attr", &["ra"], &["aditus-dom.fn"],
     r#"function aditusRa(a,sel){const at=String(a).split('|').filter(Boolean),q=sel||at.map(x=>'['+CSS.escape(x)+']').join(',');aditusDom(()=>{for(const e of document.querySelectorAll(q))for(const x of at)e.removeAttribute(x)})}"#),
    ("remove-class", &["rc"], &["aditus-dom.fn"],
     r#"function aditusRc(c,sel){const cl=String(c).split('|').filter(Boolean),q=sel||cl.map(x=>'.'+CSS.escape(x)).join(',');aditusDom(()=>{for(const e of document.querySelectorAll(q))e.classList.remove(...cl)})}"#),
];

fn res(name: &str, aliases: Vec<String>, kind: MimeType, deps: &[&str], src: &str) -> Resource {
    Resource {
        name: name.into(),
        aliases,
        kind: ResourceType::Mime(kind),
        content: STANDARD.encode(src),
        dependencies: deps.iter().map(|d| d.to_string()).collect(),
        permission: Default::default(),
    }
}

pub fn resources() -> Vec<Resource> {
    let mut out: Vec<Resource> = LIB.iter().map(|(n, s)| res(n, vec![], MimeType::FnJavascript, &[], s)).collect();
    for (name, aliases, deps, src) in SCRIPTLETS {
        // Lists use both `aopr` and `ubo-aopr`, with or without `.js`.
        let al = std::iter::once(name).chain(aliases.iter()).flat_map(|a| [format!("{a}.js"), format!("ubo-{a}.js")]).collect();
        out.push(res(&format!("{name}.js"), al, MimeType::ApplicationJavascript, deps, src));
    }
    out
}
