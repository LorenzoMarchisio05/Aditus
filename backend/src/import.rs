//! Import bookmarks from a Chrome-format `Bookmarks` JSON file (Chrome, Brave, Edge, Vivaldi, Arc).

use serde_json::Value;

/// All `(url, title)` leaves under the bookmark roots.
pub fn chrome_bookmarks(json: &str) -> Vec<(String, String)> {
    fn walk(n: &Value, out: &mut Vec<(String, String)>) {
        if n["type"] == "url" {
            if let Some(u) = n["url"].as_str().filter(|u| u.starts_with("http")) {
                out.push((u.into(), n["name"].as_str().unwrap_or(u).into()));
            }
        }
        for c in n["children"].as_array().into_iter().flatten() {
            walk(c, out);
        }
    }
    let mut out = vec![];
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    for root in v["roots"].as_object().into_iter().flat_map(|m| m.values()) {
        walk(root, &mut out);
    }
    out
}

/// Rows of RFC 4180 CSV (quoted fields, doubled quotes, newlines inside quotes).
fn csv(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut f, mut q) = (vec![], vec![], String::new(), false);
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match (c, q) {
            ('"', true) if it.peek() == Some(&'"') => {
                f.push('"');
                it.next();
            }
            ('"', _) => q = !q,
            (',', false) => row.push(std::mem::take(&mut f)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut f));
                rows.push(std::mem::take(&mut row));
            }
            _ => f.push(c),
        }
    }
    if !f.is_empty() || !row.is_empty() {
        row.push(f);
        rows.push(row);
    }
    rows
}

/// `(origin, username, password)` from Chrome / Edge / Brave's `name,url,username,password,note` export
/// (header row found by name, so Firefox / 1Password style column order works too).
pub fn chrome_passwords_csv(text: &str) -> Vec<(String, String, String)> {
    let rows = csv(text);
    let Some(head) = rows.first() else { return vec![] };
    let col = |n: &str| head.iter().position(|h| h.trim().eq_ignore_ascii_case(n));
    let (Some(u), Some(l), Some(p)) = (col("url"), col("username"), col("password")) else { return vec![] };
    rows[1..]
        .iter()
        .filter_map(|r| {
            let (url, user, pw) = (r.get(u)?, r.get(l)?, r.get(p)?);
            Some((crate::passwords::origin_of(url)?, user.clone(), pw.clone())).filter(|(_, _, pw)| !pw.is_empty())
        })
        .collect()
}

/// Most recent 5000 `(url, title, unix seconds)` from a Chrome-format `History` database. Works on a
/// copy, because Chrome keeps the original locked while it runs.
pub fn chrome_history(db: &std::path::Path) -> Option<Vec<(String, String, i64)>> {
    let tmp = std::env::temp_dir().join(format!("aditus-import-{}.db", std::process::id()));
    std::fs::copy(db, &tmp).ok()?;
    let rows = (|| -> rusqlite::Result<_> {
        let c = rusqlite::Connection::open_with_flags(&tmp, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut st = c.prepare("SELECT url,title,last_visit_time FROM urls WHERE url LIKE 'http%' ORDER BY last_visit_time DESC LIMIT 5000")?;
        let v = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? / 1_000_000 - 11_644_473_600)))?.flatten().collect();
        Ok(v)
    })();
    let _ = std::fs::remove_file(&tmp);
    rows.ok()
}

/// Default Chrome bookmarks file on this machine.
pub fn chrome_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    return std::env::var_os("LOCALAPPDATA").map(|d| std::path::Path::new(&d).join("Google/Chrome/User Data/Default/Bookmarks"));
    #[cfg(not(target_os = "windows"))]
    chrome_path_unix()
}

#[cfg(not(target_os = "windows"))]
fn chrome_path_unix() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    #[cfg(target_os = "macos")]
    let rel = "Library/Application Support/Google/Chrome/Default/Bookmarks";
    #[cfg(not(target_os = "macos"))]
    let rel = ".config/google-chrome/Default/Bookmarks";
    Some(std::path::Path::new(&home).join(rel))
}

#[cfg(test)]
mod tests {
    #[test]
    fn passwords_csv() {
        let t = "name,url,username,password,note\r\na,https://a.com/login,me,\"p,\"\"w\"\"\",\nb,ftp://x,u,p,\n";
        assert_eq!(super::chrome_passwords_csv(t), vec![("https://a.com".into(), "me".into(), "p,\"w\"".into())]);
    }

    #[test]
    fn history() {
        let p = std::env::temp_dir().join("aditus-hist-fixture.db");
        let _ = std::fs::remove_file(&p);
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch("CREATE TABLE urls(url TEXT, title TEXT, last_visit_time INTEGER); INSERT INTO urls VALUES('https://a.com/','A',13400000000000000),('chrome://x','X',1);").unwrap();
        drop(c);
        let h = super::chrome_history(&p).unwrap();
        assert_eq!(h, vec![("https://a.com/".to_string(), "A".to_string(), 13_400_000_000 - 11_644_473_600)]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn chrome() {
        let j = r#"{"roots":{"bookmark_bar":{"children":[{"type":"url","name":"A","url":"https://a.com/"},{"type":"folder","children":[{"type":"url","name":"B","url":"http://b.com/"},{"type":"url","name":"J","url":"javascript:1"}]}]}}}"#;
        let b = super::chrome_bookmarks(j);
        assert_eq!(b, vec![("https://a.com/".into(), "A".into()), ("http://b.com/".into(), "B".into())]);
    }
}
