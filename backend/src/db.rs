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

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    fn init(c: Connection) -> rusqlite::Result<Self> {
        c.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS history(
               id INTEGER PRIMARY KEY, url TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
               profile TEXT NOT NULL, visited_at INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS history_time ON history(visited_at DESC);
             CREATE TABLE IF NOT EXISTS downloads(
               id INTEGER PRIMARY KEY, url TEXT NOT NULL, path TEXT NOT NULL,
               status TEXT NOT NULL, started_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS bookmarks(
               url TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '',
               profile TEXT NOT NULL, added_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS logins(origin TEXT NOT NULL, username TEXT NOT NULL, PRIMARY KEY(origin,username));",
        )?;
        // PRAGMA user_version migrations. 2: logins per profile ('' = saved before profiles, see claim_legacy_logins).
        if c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? < 2 {
            c.execute_batch(
                "BEGIN;
                 CREATE TABLE logins2(profile TEXT NOT NULL DEFAULT '', origin TEXT NOT NULL, username TEXT NOT NULL, PRIMARY KEY(profile,origin,username));
                 INSERT INTO logins2(origin,username) SELECT origin,username FROM logins;
                 DROP TABLE logins;
                 ALTER TABLE logins2 RENAME TO logins;
                 PRAGMA user_version=2;
                 COMMIT;",
            )?;
        }
        Ok(Self(Mutex::new(c)))
    }

    pub fn add_visit(&self, url: &str, title: &str, profile: &str) {
        let _ = self.0.lock().unwrap().execute(
            "INSERT INTO history(url,title,profile,visited_at) VALUES(?1,?2,?3,strftime('%s','now'))",
            params![url, title, profile],
        );
    }

    pub fn set_title(&self, url: &str, title: &str) {
        let _ = self.0.lock().unwrap().execute(
            "UPDATE history SET title=?2 WHERE id=(SELECT MAX(id) FROM history WHERE url=?1)",
            params![url, title],
        );
    }

    pub fn search_history(&self, q: &str, limit: i64) -> Vec<HistoryRow> {
        let c = self.0.lock().unwrap();
        let like = format!("%{}%", q.replace('%', ""));
        let mut st = c
            .prepare("SELECT url,title,profile,visited_at FROM history WHERE url LIKE ?1 OR title LIKE ?1 ORDER BY id DESC LIMIT ?2")
            .unwrap();
        st.query_map(params![like, limit], |r| {
            Ok(HistoryRow { url: r.get(0)?, title: r.get(1)?, profile: r.get(2)?, visited_at: r.get(3)? })
        })
        .unwrap()
        .flatten()
        .collect()
    }

    /// Bulk insert for imports: `(url, title, unix seconds)`.
    pub fn import_visits(&self, rows: &[(String, String, i64)], profile: &str) {
        let mut c = self.0.lock().unwrap();
        let Ok(tx) = c.transaction() else { return };
        for (u, t, at) in rows {
            let _ = tx.execute("INSERT INTO history(url,title,profile,visited_at) VALUES(?1,?2,?3,?4)", params![u, t, profile, at]);
        }
        let _ = tx.commit();
    }

    pub fn add_bookmark(&self, url: &str, title: &str, profile: &str) {
        let _ = self.0.lock().unwrap().execute(
            "INSERT OR REPLACE INTO bookmarks(url,title,profile,added_at) VALUES(?1,?2,?3,strftime('%s','now'))",
            params![url, title, profile],
        );
    }

    pub fn remove_bookmark(&self, url: &str) {
        let _ = self.0.lock().unwrap().execute("DELETE FROM bookmarks WHERE url=?1", params![url]);
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.0.lock().unwrap().query_row("SELECT 1 FROM bookmarks WHERE url=?1", params![url], |_| Ok(())).is_ok()
    }

    pub fn search_bookmarks(&self, q: &str, limit: i64) -> Vec<BookmarkRow> {
        let c = self.0.lock().unwrap();
        let like = format!("%{}%", q.replace('%', ""));
        let mut st = c
            .prepare("SELECT url,title,profile,added_at FROM bookmarks WHERE url LIKE ?1 OR title LIKE ?1 ORDER BY added_at DESC LIMIT ?2")
            .unwrap();
        st.query_map(params![like, limit], |r| {
            Ok(BookmarkRow { url: r.get(0)?, title: r.get(1)?, profile: r.get(2)?, added_at: r.get(3)? })
        })
        .unwrap()
        .flatten()
        .collect()
    }

    /// Saved-login index. Passwords live in the OS keychain, never here.
    pub fn add_login(&self, profile: &str, origin: &str, user: &str) {
        let _ = self.0.lock().unwrap().execute("INSERT OR IGNORE INTO logins(profile,origin,username) VALUES(?1,?2,?3)", params![profile, origin, user]);
    }

    pub fn remove_login(&self, profile: &str, origin: &str, user: &str) {
        let _ = self.0.lock().unwrap().execute("DELETE FROM logins WHERE profile=?1 AND origin=?2 AND username=?3", params![profile, origin, user]);
    }

    /// Logins saved before profiles existed belong to the first one.
    pub fn claim_legacy_logins(&self, profile: &str) {
        let _ = self.0.lock().unwrap().execute("UPDATE OR IGNORE logins SET profile=?1 WHERE profile=''", params![profile]);
    }

    /// `(origin, username, profile)`, optionally for one profile and/or origin.
    pub fn logins(&self, profile: Option<&str>, origin: Option<&str>) -> Vec<(String, String, String)> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT origin,username,profile FROM logins WHERE (?1 IS NULL OR profile=?1) AND (?2 IS NULL OR origin=?2) ORDER BY origin,username").unwrap();
        st.query_map(params![profile, origin], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().flatten().collect()
    }

    pub fn prune_history(&self, days: u32) {
        let _ = self.0.lock().unwrap().execute(
            "DELETE FROM history WHERE visited_at < strftime('%s','now') - ?1 * 86400",
            params![days],
        );
    }

    /// History from the last `hours` hours; 0 clears everything.
    pub fn clear_history_since(&self, hours: u32) {
        let _ = self.0.lock().unwrap().execute("DELETE FROM history WHERE ?1 = 0 OR visited_at >= strftime('%s','now') - ?1 * 3600", params![hours]);
    }

    pub fn clear_downloads(&self) {
        let _ = self.0.lock().unwrap().execute("DELETE FROM downloads", []);
    }

    pub fn clear_history(&self) {
        let _ = self.0.lock().unwrap().execute("DELETE FROM history", []);
    }

    pub fn add_download(&self, url: &str, path: &str) -> i64 {
        let c = self.0.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO downloads(url,path,status,started_at) VALUES(?1,?2,'active',strftime('%s','now'))",
            params![url, path],
        );
        c.last_insert_rowid()
    }

    pub fn finish_download(&self, url: &str, path: &str, ok: bool) {
        let _ = self.0.lock().unwrap().execute(
            "UPDATE downloads SET status=?3, path=CASE WHEN ?2='' THEN path ELSE ?2 END WHERE id=(SELECT MAX(id) FROM downloads WHERE url=?1 AND status='active')",
            params![url, path, if ok { "done" } else { "failed" }],
        );
    }

    pub fn downloads(&self) -> Vec<DownloadRow> {
        let c = self.0.lock().unwrap();
        let mut st = c
            .prepare("SELECT id,url,path,status,started_at FROM downloads ORDER BY id DESC LIMIT 200")
            .unwrap();
        st.query_map([], |r| {
            Ok(DownloadRow { id: r.get(0)?, url: r.get(1)?, path: r.get(2)?, status: r.get(3)?, started_at: r.get(4)? })
        })
        .unwrap()
        .flatten()
        .collect()
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
        db.add_download("https://a.com/f.zip", "/tmp/f.zip");
        db.finish_download("https://a.com/f.zip", "", true);
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
    fn logins_per_profile_and_migration() {
        let c = Connection::open_in_memory().unwrap();
        // A version-1 database: logins without a profile column.
        c.execute_batch("CREATE TABLE logins(origin TEXT NOT NULL, username TEXT NOT NULL, PRIMARY KEY(origin,username)); INSERT INTO logins VALUES('https://a.com','old');").unwrap();
        let db = Db::init(c).unwrap();
        db.claim_legacy_logins("p1");
        assert_eq!(db.logins(Some("p1"), Some("https://a.com")), vec![("https://a.com".to_string(), "old".to_string(), "p1".to_string())]);
        db.add_login("p2", "https://a.com", "old");
        db.add_login("p2", "https://a.com", "new");
        assert_eq!(db.logins(Some("p2"), Some("https://a.com")).len(), 2);
        assert_eq!(db.logins(None, None).len(), 3);
        db.remove_login("p2", "https://a.com", "old");
        assert_eq!(db.logins(Some("p1"), None).len(), 1);
        assert_eq!(db.logins(Some("p2"), None).len(), 1);
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
