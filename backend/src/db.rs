//! SQLite: history and downloads.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

pub struct Db(Mutex<Connection>);

#[derive(Serialize)]
pub struct HistoryRow {
    pub url: String,
    pub title: String,
    pub profile: String,
    pub visited_at: i64,
}

#[derive(Serialize)]
pub struct DownloadRow {
    pub id: i64,
    pub url: String,
    pub path: String,
    pub status: String,
    pub started_at: i64,
}

#[derive(Serialize)]
pub struct BookmarkRow {
    pub url: String,
    pub title: String,
    pub profile: String,
    pub added_at: i64,
}

/// `%q%` with LIKE wildcards in `q` escaped (pair with `ESCAPE '\\'`).
fn like(q: &str) -> String {
    format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?).or_else(|_| {
            // Corrupt or unopenable: move it aside (history is lost, the browser still launches).
            let _ = std::fs::rename(path, path.with_extension("db.bak"));
            let _ = std::fs::remove_file(path.with_extension("db-wal"));
            let _ = std::fs::remove_file(path.with_extension("db-shm"));
            Self::init(Connection::open(path)?)
        })
    }

    fn rows<T>(&self, sql: &str, p: impl rusqlite::Params, f: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>) -> Vec<T> {
        let c = self.0.lock().unwrap();
        let Ok(mut st) = c.prepare(sql) else { return vec![] };
        let Ok(it) = st.query_map(p, f) else { return vec![] };
        it.flatten().collect()
    }

    fn init(c: Connection) -> rusqlite::Result<Self> {
        c.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS history(
               id INTEGER PRIMARY KEY, url TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
               profile TEXT NOT NULL, visited_at INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS history_time ON history(visited_at DESC);
             CREATE INDEX IF NOT EXISTS history_url ON history(url, visited_at);
             CREATE TABLE IF NOT EXISTS downloads(
               id INTEGER PRIMARY KEY, url TEXT NOT NULL, path TEXT NOT NULL,
               status TEXT NOT NULL, started_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS bookmarks(
               url TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '',
               profile TEXT NOT NULL, added_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS logins(origin TEXT NOT NULL, username TEXT NOT NULL, PRIMARY KEY(origin,username));",
        )?;
        // Schema versioning: bump `user_version` and add an `if v < N` step when the schema changes.
        // Tables use IF NOT EXISTS above, so v1 is just the baseline.
        let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v < 1 {
            c.execute_batch("PRAGMA user_version=1")?;
        }
        // Nothing can be downloading at startup: an 'active' row is left over from a crash.
        c.execute("UPDATE downloads SET status='failed' WHERE status='active'", [])?;
        Ok(Self(Mutex::new(c)))
    }

    pub fn add_visit(&self, url: &str, title: &str, profile: &str) {
        let _ = crate::applog::check("db add_visit", self.0.lock().unwrap().execute(
            "INSERT INTO history(url,title,profile,visited_at) VALUES(?1,?2,?3,strftime('%s','now'))",
            params![url, title, profile],
        ));
    }

    pub fn set_title(&self, url: &str, title: &str) {
        let _ = crate::applog::check("db set_title", self.0.lock().unwrap().execute(
            "UPDATE history SET title=?2 WHERE id=(SELECT MAX(id) FROM history WHERE url=?1)",
            params![url, title],
        ));
    }

    pub fn search_history(&self, q: &str, limit: i64) -> Vec<HistoryRow> {
        let like = like(q);
        self.rows("SELECT url,title,profile,MAX(visited_at) v FROM history WHERE url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\' GROUP BY url ORDER BY MAX(id) DESC LIMIT ?2", params![like, limit], |r| {
            Ok(HistoryRow { url: r.get(0)?, title: r.get(1)?, profile: r.get(2)?, visited_at: r.get(3)? })
        })
    }

    /// Bulk insert for imports: `(url, title, unix seconds)`.
    pub fn import_visits(&self, rows: &[(String, String, i64)], profile: &str) {
        let mut c = self.0.lock().unwrap();
        let Ok(tx) = c.transaction() else { return };
        for (u, t, at) in rows {
            let _ = tx.execute(
                "INSERT INTO history(url,title,profile,visited_at) SELECT ?1,?2,?3,?4 WHERE NOT EXISTS(SELECT 1 FROM history WHERE url=?1 AND visited_at=?4)",
                params![u, t, profile, at],
            );
        }
        let _ = crate::applog::check("db import_visits", tx.commit());
    }

    pub fn add_bookmark(&self, url: &str, title: &str, profile: &str) {
        let _ = crate::applog::check("db add_bookmark", self.0.lock().unwrap().execute(
            "INSERT OR REPLACE INTO bookmarks(url,title,profile,added_at) VALUES(?1,?2,?3,strftime('%s','now'))",
            params![url, title, profile],
        ));
    }

    pub fn remove_bookmark(&self, url: &str) {
        let _ = crate::applog::check("db remove_bookmark", self.0.lock().unwrap().execute("DELETE FROM bookmarks WHERE url=?1", params![url]));
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.0.lock().unwrap().query_row("SELECT 1 FROM bookmarks WHERE url=?1", params![url], |_| Ok(())).is_ok()
    }

    pub fn search_bookmarks(&self, q: &str, limit: i64) -> Vec<BookmarkRow> {
        let like = like(q);
        self.rows("SELECT url,title,profile,added_at FROM bookmarks WHERE url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\' ORDER BY added_at DESC LIMIT ?2", params![like, limit], |r| {
            Ok(BookmarkRow { url: r.get(0)?, title: r.get(1)?, profile: r.get(2)?, added_at: r.get(3)? })
        })
    }

    /// Saved-login index. Passwords live in the OS keychain, never here.
    pub fn add_login(&self, origin: &str, user: &str) {
        let _ = crate::applog::check("db add_login", self.0.lock().unwrap().execute("INSERT OR IGNORE INTO logins VALUES(?1,?2)", params![origin, user]));
    }

    pub fn remove_login(&self, origin: &str, user: &str) {
        let _ = crate::applog::check("db remove_login", self.0.lock().unwrap().execute("DELETE FROM logins WHERE origin=?1 AND username=?2", params![origin, user]));
    }

    /// `(origin, username)`, optionally for one origin.
    pub fn logins(&self, origin: Option<&str>) -> Vec<(String, String)> {
        self.rows("SELECT origin,username FROM logins WHERE ?1 IS NULL OR origin=?1 ORDER BY origin,username", params![origin], |r| Ok((r.get(0)?, r.get(1)?)))
    }

    pub fn prune_history(&self, days: u32) {
        let _ = crate::applog::check("db prune_history", self.0.lock().unwrap().execute(
            "DELETE FROM history WHERE visited_at < strftime('%s','now') - ?1 * 86400",
            params![days],
        ));
    }

    /// History from the last `hours` hours; 0 clears everything.
    pub fn clear_history_since(&self, hours: u32) {
        let _ = crate::applog::check("db clear_history_since", self.0.lock().unwrap().execute("DELETE FROM history WHERE ?1 = 0 OR visited_at >= strftime('%s','now') - ?1 * 3600", params![hours]));
    }

    pub fn clear_downloads(&self) {
        let _ = crate::applog::check("db clear_downloads", self.0.lock().unwrap().execute("DELETE FROM downloads", []));
    }

    pub fn clear_history(&self) {
        let _ = crate::applog::check("db clear_history", self.0.lock().unwrap().execute("DELETE FROM history", []));
    }

    pub fn add_download(&self, url: &str, path: &str) -> i64 {
        let c = self.0.lock().unwrap();
        let _ = crate::applog::check("db add_download", c.execute(
            "INSERT INTO downloads(url,path,status,started_at) VALUES(?1,?2,'active',strftime('%s','now'))",
            params![url, path],
        ));
        c.last_insert_rowid()
    }

    pub fn finish_download(&self, id: i64, path: &str, ok: bool) {
        let _ = crate::applog::check("db finish_download", self.0.lock().unwrap().execute(
            "UPDATE downloads SET status=?3, path=CASE WHEN ?2='' THEN path ELSE ?2 END WHERE id=?1",
            params![id, path, if ok { "done" } else { "failed" }],
        ));
    }

    pub fn downloads(&self) -> Vec<DownloadRow> {
        self.rows("SELECT id,url,path,status,started_at FROM downloads ORDER BY id DESC LIMIT 200", [], |r| {
            Ok(DownloadRow { id: r.get(0)?, url: r.get(1)?, path: r.get(2)?, status: r.get(3)?, started_at: r.get(4)? })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_and_downloads() {
        let db = Db::init(Connection::open_in_memory().unwrap()).unwrap();
        db.add_visit("https://a.com/", "A site", "Default");
        db.set_title("https://a.com/", "Renamed");
        assert_eq!(db.search_history("renam", 10).len(), 1);
        assert_eq!(db.search_history("zzz", 10).len(), 0);
        let id = db.add_download("https://a.com/f.zip", "/tmp/f.zip");
        let id2 = db.add_download("https://a.com/f.zip", "/tmp/f (1).zip");
        db.finish_download(id, "", true);
        assert_eq!(db.downloads()[1].status, "done");
        assert_eq!(db.downloads()[0].status, "active"); // same URL, other id: untouched
        db.finish_download(id2, "", false);
        db.clear_downloads();
        let id = db.add_download("https://a.com/f.zip", "/tmp/f.zip");
        db.finish_download(id, "", true);
        assert_eq!(db.downloads()[0].status, "done");
        assert_eq!(db.downloads()[0].path, "/tmp/f.zip");
        db.clear_history_since(1);
        assert!(db.search_history("", 10).is_empty());
        db.add_visit("https://b.com/", "B", "Default");
        db.clear_history_since(0);
        assert!(db.search_history("", 10).is_empty());
        db.clear_downloads();
        assert!(db.downloads().is_empty());
    }

    #[test]
    fn history_search_escapes_groups_and_dedupes() {
        let db = Db::init(Connection::open_in_memory().unwrap()).unwrap();
        db.add_visit("https://a.com/50%_off", "Sale", "Default");
        db.add_visit("https://a.com/50%_off", "Sale", "Default");
        db.add_visit("https://b.com/", "B", "Default");
        assert_eq!(db.search_history("%", 10).len(), 1);
        assert_eq!(db.search_history("_", 10).len(), 1);
        assert_eq!(db.search_history("a.com", 10).len(), 1); // grouped by url
        let rows = vec![("https://c.com/".to_string(), "C".to_string(), 5)];
        db.import_visits(&rows, "Default");
        db.import_visits(&rows, "Default");
        assert_eq!(db.search_history("c.com", 10).len(), 1);
        assert_eq!(db.0.lock().unwrap().query_row("SELECT COUNT(*) FROM history WHERE url='https://c.com/'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn bookmarks() {
        let db = Db::init(Connection::open_in_memory().unwrap()).unwrap();
        db.add_bookmark("https://a.com/", "A", "Default");
        db.add_bookmark("https://a.com/", "A again", "Work"); // same URL replaces
        assert!(db.is_bookmarked("https://a.com/"));
        assert_eq!(db.search_bookmarks("again", 10).len(), 1);
        db.remove_bookmark("https://a.com/");
        assert!(!db.is_bookmarked("https://a.com/"));
    }
}
