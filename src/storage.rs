use std::collections::HashSet;
use std::path::Path;

use anyhow::bail;
use anyhow::Result;
use rusqlite::Connection;

use crate::core::FileKey;
use crate::resolution::ResolutionCtx;

const TABLES: &'static str = r#"
    CREATE TABLE files (
        filename TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        value BLOB NOT NULL,
        PRIMARY KEY (filename, content_hash)
    ) STRICT;
"#;

const TEMP_TABLES: &'static str = r#"
    CREATE TEMP TABLE working_files (
        filename TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        PRIMARY KEY (filename, content_hash)
    ) STRICT;
"#;

const PRAGMAS: &str = r#"
    PRAGMA journal_mode = WAL;
"#;

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

    pub fn save(&mut self, key: &FileKey, ctx: &mut ResolutionCtx) -> Result<()> {
        self.conn
            .prepare_cached("INSERT INTO files (filename, content_hash, value) VALUES (?, ?, ?)")?
            .execute((&key.filename, &key.content_hash.to_string(), ctx.encode()?))?;
        Ok(())
    }

    pub fn load<'a, K>(&mut self, keys: K) -> Result<ResolutionCtx>
    where
        K: IntoIterator<Item = &'a FileKey>,
    {
        self.prepare_working_files(keys)?;

        let mut stmt = self.conn.prepare_cached(
            r#"SELECT W.filename, W.content_hash, F.value
            FROM working_files W
            LEFT JOIN files F ON F.filename = W.filename AND F.content_hash = W.content_hash"#,
        )?;

        let mut rows = stmt.query([])?;
        let mut bytes: Vec<Vec<u8>> = Vec::new();

        while let Some(row) = rows.next()? {
            let value: Option<Vec<u8>> = row.get(2)?;

            match value {
                Some(v) => bytes.push(v),
                None => {
                    let key = FileKey::from_string(row.get(0)?, row.get(1)?)?;
                    bail!("no value found for {}", key);
                }
            };
        }

        Ok(ResolutionCtx::decode_many(bytes.iter().map(|b| &b[..]))?)
    }

    pub fn find_missing<'a, K>(&mut self, keys: K) -> Result<HashSet<FileKey>>
    where
        K: IntoIterator<Item = &'a FileKey>,
    {
        self.prepare_working_files(keys)?;

        let mut stmt = self.conn.prepare_cached(
            r#"SELECT W.filename, W.content_hash
            FROM working_files W
            LEFT JOIN files F ON F.filename = W.filename AND F.content_hash = W.content_hash
            WHERE F.value IS NULL"#,
        )?;

        let mut rows = stmt.query([])?;
        let mut keys = HashSet::new();

        while let Some(row) = rows.next()? {
            keys.insert(FileKey::from_string(row.get(0)?, row.get(1)?)?);
        }

        Ok(keys)
    }

    fn prepare_working_files<'a, K>(&mut self, keys: K) -> Result<()>
    where
        K: IntoIterator<Item = &'a FileKey>,
    {
        self.conn
            .prepare_cached("DELETE FROM working_files")?
            .execute([])?;
        let mut stmt = self
            .conn
            .prepare_cached("INSERT INTO working_files (filename, content_hash) VALUES (?, ?)")?;
        for key in keys {
            stmt.execute((&key.filename, &key.content_hash.to_string()))?;
        }
        Ok(())
    }
}
