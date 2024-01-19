use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use anyhow::bail;
use anyhow::Result;
use rusqlite::Connection;

use crate::resolution::ResolutionCtx;

const TABLES: &'static str = r#"
    CREATE TABLE files (
        oid TEXT NOT NULL,
        filename TEXT NOT NULL,
        value BLOB NOT NULL,
        PRIMARY KEY (oid, filename)
    ) STRICT;
"#;

const TEMP_TABLES: &'static str = r#"
    CREATE TEMP TABLE working_files (
        oid TEXT NOT NULL,
        filename TEXT NOT NULL,
        PRIMARY KEY (oid, filename)
    ) STRICT;
"#;

const PRAGMAS: &str = r#"
    PRAGMA journal_mode = WAL;
"#;

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub struct StoreKey {
    pub oid: String,
    pub filename: String,
}

impl StoreKey {
    pub fn new(oid: String, filename: String) -> Self {
        Self { oid, filename }
    }
}

impl fmt::Display for StoreKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.filename, self.oid)
    }
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Store> {
        let is_new = !path.as_ref().exists();
        let conn = Connection::open(path)?;
        conn.execute_batch(PRAGMAS)?;
        if is_new {
            conn.execute_batch(TABLES)?;
        }
        conn.execute_batch(TEMP_TABLES)?;
        Ok(Self { conn })
    }

    pub fn save(&mut self, key: &StoreKey, ctx: &mut ResolutionCtx) -> Result<()> {
        self.conn
            .prepare_cached("INSERT INTO files (oid, filename, value) VALUES (?, ?, ?)")?
            .execute((&key.oid, &key.filename, ctx.encode()?))?;
        Ok(())
    }

    pub fn load<'a, K>(&mut self, keys: K) -> Result<ResolutionCtx>
    where
        K: IntoIterator<Item = &'a StoreKey>,
    {
        self.prepare_working_files(keys)?;

        let mut stmt = self.conn.prepare_cached(
            r#"SELECT W.oid, W.filename, F.value
            FROM working_files W
            LEFT JOIN files F ON F.oid = W.oid AND F.filename = W.filename"#,
        )?;

        let mut rows = stmt.query([])?;
        let mut bytes: Vec<Vec<u8>> = Vec::new();

        while let Some(row) = rows.next()? {
            let value: Option<Vec<u8>> = row.get(2)?;

            match value {
                Some(v) => bytes.push(v),
                None => {
                    let key = StoreKey::new(row.get(0)?, row.get(1)?);
                    bail!("no value found for {}", key);
                }
            };
        }

        Ok(ResolutionCtx::decode_many(bytes.iter().map(|b| &b[..]))?)
    }

    pub fn find_missing<'a, K>(&mut self, keys: K) -> Result<HashSet<StoreKey>>
    where
        K: IntoIterator<Item = &'a StoreKey>,
    {
        self.prepare_working_files(keys)?;

        let mut stmt = self.conn.prepare_cached(
            r#"SELECT W.oid, W.filename
            FROM working_files W
            LEFT JOIN files F ON F.oid = W.oid AND F.filename = W.filename
            WHERE F.value IS NULL"#,
        )?;

        let results: rusqlite::Result<HashSet<StoreKey>> = stmt
            .query_map([], |r| Ok(StoreKey::new(r.get(0)?, r.get(1)?)))?
            .collect();

        Ok(results?)
    }

    fn prepare_working_files<'a, K>(&mut self, keys: K) -> Result<()>
    where
        K: IntoIterator<Item = &'a StoreKey>,
    {
        self.conn
            .prepare_cached("DELETE FROM working_files")?
            .execute([])?;
        let mut stmt = self
            .conn
            .prepare_cached("INSERT INTO working_files (oid, filename) VALUES (?, ?)")?;
        for key in keys {
            stmt.execute((&key.oid, &key.filename))?;
        }
        Ok(())
    }
}
