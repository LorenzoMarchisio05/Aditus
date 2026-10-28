//! What a page reports about itself so a rebuilt view can put it back (scroll, form fields, media position,
//! sessionStorage, scrolled containers). The report comes from an untrusted page: it is parsed into this
//! typed shape, capped and filtered here, and only the re-serialized result reaches `restore_script`,
//! which hands it to the new page as a JSON string literal, never as code.

use serde::{Deserialize, Serialize};

const MAX_JSON: usize = 60_000;
const MAX_FIELDS: usize = 100;
const MAX_FIELD_LEN: usize = 5000;
const MAX_INDEX: u32 = 100_000;
const MAX_MEDIA: usize = 4;
const MAX_STORAGE_ITEMS: usize = 50;
const MAX_STORAGE_KEY: usize = 500;
const MAX_STORAGE_VALUE: usize = 4000;
const MAX_STORAGE_TOTAL: usize = 20_000;
const MAX_SCROLLERS: usize = 50;
const MAX_OFFSET: f64 = 10_000_000.0;
/// Keys that usually hold credentials or session identity are never carried over.
const SENSITIVE: &[&str] = &["token", "auth", "pass", "secret", "session", "jwt", "csrf", "cred", "key", "login"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Field {
    Checked(bool),
    Text(String),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PageSnap {
    /// Window scroll.
    pub x: f64,
    pub y: f64,
    /// (index among input/textarea/select, value)
    pub f: Vec<(u32, Field)>,
    /// (index among audio/video, seconds)
    pub m: Vec<(u32, f64)>,
    /// sessionStorage of the page's origin.
    pub ss: Vec<(String, String)>,
    /// (index among all elements, scrollLeft, scrollTop) of scrolled containers.
    pub sc: Vec<(u32, f64, f64)>,
}

fn offset(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, MAX_OFFSET)
    } else {
        0.0
    }
}

impl PageSnap {
    /// `None` for anything that is not a well-formed report; otherwise the report cut down to the caps.
    pub fn parse(json: &str) -> Option<Self> {
        if json.len() > MAX_JSON {
            return None;
        }
        let mut s: PageSnap = serde_json::from_str(json).ok()?;
        s.x = offset(s.x);
        s.y = offset(s.y);
        s.f.retain(|(i, v)| *i < MAX_INDEX && !matches!(v, Field::Text(t) if t.is_empty() || t.len() > MAX_FIELD_LEN));
        s.f.truncate(MAX_FIELDS);
        s.m.retain(|(i, t)| *i < MAX_INDEX && t.is_finite() && *t >= 0.0);
        s.m.truncate(MAX_MEDIA);
        let mut total = 0;
        s.ss.retain(|(k, v)| {
            let lk = k.to_lowercase();
            let ok = !k.is_empty() && k.len() <= MAX_STORAGE_KEY && v.len() <= MAX_STORAGE_VALUE && !SENSITIVE.iter().any(|w| lk.contains(w)) && total + k.len() + v.len() <= MAX_STORAGE_TOTAL;
            if ok {
                total += k.len() + v.len();
            }
            ok
        });
        s.ss.truncate(MAX_STORAGE_ITEMS);
        s.sc.retain(|(i, _, _)| *i < MAX_INDEX);
        s.sc.truncate(MAX_SCROLLERS);
        for c in &mut s.sc {
            c.1 = offset(c.1);
            c.2 = offset(c.2);
        }
        Some(s)
    }

    /// Storage belongs to the profile it was reported in: across profiles it would carry one account's
    /// page state into another's, so only the same profile gets it back.
    pub fn for_profile(mut self, reported: &str, now: &str) -> Self {
        if reported != now {
            self.ss.clear();
        }
        self
    }
}

/// Applies a snapshot: sessionStorage (once, only keys the page has not set itself), scroll (retried while
/// the page lays out), scrolled containers, empty form fields, media position. It runs after the load commits,
/// so scripts that read sessionStorage at startup have already run; that is the limit of evaluating from the browser process.
const RESTORE: &str = "j=>{const s=JSON.parse(j);let n=0;const go=()=>{n++;try{if(n===1)for(const [k,v] of s.ss)if(sessionStorage.getItem(k)===null)sessionStorage.setItem(k,v);const els=document.querySelectorAll('input,textarea,select');for(const [i,v] of s.f){const e=els[i];if(!e)continue;if(typeof v==='boolean'){if(e.checked!==v)e.click()}else if(e.value===''){e.value=v;e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}))}}const ms=document.querySelectorAll('video,audio');for(const [i,t] of s.m){const e=ms[i];if(e&&e.currentTime<1)e.currentTime=t}const all=document.querySelectorAll('*');for(const [i,l,t] of s.sc){const e=all[i];if(e){e.scrollLeft=l;e.scrollTop=t}}}catch(e){}scrollTo(s.x,s.y);if(n<15&&Math.abs(scrollY-s.y)>3)setTimeout(go,200)};go()}";

/// The script to evaluate in the restored page. The data is a string literal inside it.
pub fn restore_script(s: &PageSnap) -> String {
    let json = serde_json::to_string(s).unwrap_or_default();
    format!("({RESTORE})({})", serde_json::to_string(&json).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_report_round_trips() {
        let s = PageSnap::parse(r#"{"x":1,"y":20.5,"f":[[0,"hi"],[1,true]],"m":[[0,12.5]],"ss":[["draft","text"]],"sc":[[7,0,300]]}"#).unwrap();
        assert_eq!((s.y, s.f.len(), s.ss.len(), s.sc[0].2), (20.5, 2, 1, 300.0));
    }

    #[test]
    fn junk_and_oversize_are_rejected() {
        assert!(PageSnap::parse("not json").is_none());
        assert!(PageSnap::parse(r#"{"f":[["a","b"]]}"#).is_none());
        assert!(PageSnap::parse(&format!(r#"{{"x":0,"pad":"{}"}}"#, "a".repeat(MAX_JSON))).is_none());
    }

    #[test]
    fn caps_are_enforced() {
        let many: Vec<String> = (0..300).map(|i| format!("[{i},\"v\"]")).collect();
        let s = PageSnap::parse(&format!(r#"{{"f":[{}],"x":-5,"y":1e30}}"#, many.join(","))).unwrap();
        assert_eq!((s.f.len(), s.x, s.y), (MAX_FIELDS, 0.0, MAX_OFFSET));
        let long = "z".repeat(MAX_FIELD_LEN + 1);
        let s = PageSnap::parse(&format!(r#"{{"f":[[0,"{long}"],[100000,"x"],[1,""],[2,"ok"]]}}"#)).unwrap();
        assert_eq!(s.f, vec![(2, Field::Text("ok".into()))]);
    }

    #[test]
    fn storage_is_capped_and_credentials_skipped() {
        let s = PageSnap::parse(r#"{"ss":[["authToken","x"],["Draft","a"],["SESSION_ID","y"],["ok","b"],["","c"]]}"#).unwrap();
        assert_eq!(s.ss, vec![("Draft".to_string(), "a".to_string()), ("ok".to_string(), "b".to_string())]);
        let big = "v".repeat(MAX_STORAGE_VALUE);
        let items: Vec<String> = (0..12).map(|i| format!(r#"["n{i}","{big}"]"#)).collect();
        let s = PageSnap::parse(&format!(r#"{{"ss":[{}]}}"#, items.join(","))).unwrap();
        assert!(s.ss.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() <= MAX_STORAGE_TOTAL);
        assert!(s.ss.len() < 12);
    }

    #[test]
    fn storage_stays_in_its_profile() {
        let s = PageSnap::parse(r#"{"ss":[["a","b"]],"sc":[[1,2,3]]}"#).unwrap();
        assert_eq!(s.clone().for_profile("A", "A").ss.len(), 1);
        let moved = s.for_profile("A", "B");
        assert!(moved.ss.is_empty() && moved.sc.len() == 1);
    }

    #[test]
    fn data_never_becomes_code() {
        let s = PageSnap::parse(r#"{"f":[[0,"\");alert(1);(\""]],"ss":[["k","'`${x}"]]}"#).unwrap();
        let js = restore_script(&s);
        // The argument is one string literal that decodes back to the exact snapshot.
        let lit = &js[js.rfind(")(").unwrap() + 2..js.len() - 1];
        let json: String = serde_json::from_str(lit).unwrap();
        assert_eq!(serde_json::from_str::<PageSnap>(&json).unwrap(), s);
    }
}
