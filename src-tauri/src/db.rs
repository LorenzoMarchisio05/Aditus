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
               status TEXT NOT NULL, started_at INTEGER NOT NULL);",
        )?;
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

    pub fn prune_history(&self, days: u32) {
        let _ = self.0.lock().unwrap().execute(
            "DELETE FROM history WHERE visited_at < strftime('%s','now') - ?1 * 86400",
            params![days],
        );
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
        db.clear_history();
        assert!(db.search_history("", 10).is_empty());
    }
}
