use std::collections::HashMap;
use std::path::Path;

use anyhow::bail;
use anyhow::Result;
use rusqlite::Connection;
use rusqlite::Transaction;

use crate::core::FileKey;
use crate::resolution::StackGraphCtx;

const PRAGMAS: &str = r#"
    PRAGMA journal_mode = WAL;
"#;

const TABLES: &str = r#"
    CREATE TABLE IF NOT EXISTS files (
        filename TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        graph BLOB,
        failure TEXT,
        PRIMARY KEY (filename, content_hash),
        CHECK ((graph IS NULL OR failure IS NULL) AND
               (graph IS NOT NULL OR failure IS NOT NULL))
    ) STRICT;
"#;

const TEMP_TABLES: &str = r#"
    CREATE TEMP TABLE working_files (
        filename TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        PRIMARY KEY (filename, content_hash)
    ) STRICT;
"#;

pub struct Store {
    conn: Connection,
}

pub struct LoadResponse {
    pub ctx: StackGraphCtx,
    pub failures: HashMap<FileKey, String>,
}

impl Store {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Store> {
        let conn = Connection::open(path)?;
        conn.execute_batch(PRAGMAS)?;
        conn.execute_batch(TABLES)?;
        conn.execute_batch(TEMP_TABLES)?;
        Ok(Self { conn })
    }

    pub fn save(&self, key: &FileKey, value: Result<StackGraphCtx, String>) -> Result<()> {
        match value {
            Ok(ctx) => self.save_ctx(key, ctx),
            Err(failure) => self.save_failure(key, failure),
        }
    }

    fn save_ctx(&self, key: &FileKey, mut ctx: StackGraphCtx) -> Result<()> {
        let sql = "INSERT INTO files (filename, content_hash, graph) VALUES (?, ?, ?)";
        self.conn.prepare_cached(sql)?.execute((
            &key.filename,
            &key.content_hash.to_string(),
            ctx.encode()?,
        ))?;
        Ok(())
    }

    fn save_failure(&self, key: &FileKey, failure: String) -> Result<()> {
        let sql = "INSERT INTO files (filename, content_hash, failure) VALUES (?, ?, ?)";
        self.conn.prepare_cached(sql)?.execute((
            &key.filename,
            &key.content_hash.to_string(),
            failure,
        ))?;
        Ok(())
    }

    pub fn load<'a, K>(&self, keys: K) -> Result<LoadResponse>
    where
        K: IntoIterator<Item = &'a FileKey>,
    {
        let tx = self.conn.unchecked_transaction()?;
        Self::load_inner(&tx, keys)
    }

    fn load_inner<'a, K>(tx: &Transaction, keys: K) -> Result<LoadResponse>
    where
        K: IntoIterator<Item = &'a FileKey>,
    {
        Self::prepare_working_files(tx, keys)?;

        let mut stmt = tx.prepare_cached(
            r#"SELECT W.filename, W.content_hash, F.graph, F.failure
            FROM working_files W
            LEFT JOIN files F ON F.filename = W.filename AND F.content_hash = W.content_hash"#,
        )?;

        let mut rows = stmt.query([])?;
        let mut bytes: Vec<Vec<u8>> = Vec::new();
        let mut failures = HashMap::new();

        while let Some(row) = rows.next()? {
            let key = FileKey::from_string(row.get(0)?, row.get(1)?)?;
            let ctx: Option<Vec<u8>> = row.get(2)?;
            let failure: Option<String> = row.get(3)?;

            if let Some(failure) = failure {
                failures.insert(key, failure);
            } else if let Some(ctx) = ctx {
                bytes.push(ctx);
            } else {
                bail!("no value found for {}", key);
            }
        }

        let ctx = StackGraphCtx::decode_many(bytes.iter().map(|b| &b[..]))?;
        Ok(LoadResponse { ctx, failures })
    }

    pub fn find_missing<'a, K>(&self, keys: K) -> Result<Vec<FileKey>>
    where
        K: IntoIterator<Item = &'a FileKey>,
    {
        let tx = self.conn.unchecked_transaction()?;
        Self::find_missing_inner(&tx, keys)
    }

    fn find_missing_inner<'a, K>(tx: &Transaction, keys: K) -> Result<Vec<FileKey>>
    where
        K: IntoIterator<Item = &'a FileKey>,
    {
        Self::prepare_working_files(tx, keys)?;

        let mut stmt = tx.prepare_cached(
            r#"SELECT W.filename, W.content_hash
            FROM working_files W
            LEFT JOIN files F ON F.filename = W.filename AND F.content_hash = W.content_hash
            WHERE F.graph IS NULL AND F.failure IS NULL"#,
        )?;

        let mut rows = stmt.query([])?;
        let mut keys = Vec::new();

        while let Some(row) = rows.next()? {
            keys.push(FileKey::from_string(row.get(0)?, row.get(1)?)?);
        }

        keys.sort();
        Ok(keys)
    }

    fn prepare_working_files<'a, K>(tx: &Transaction<'_>, keys: K) -> Result<()>
    where
        K: IntoIterator<Item = &'a FileKey>,
    {
        tx.prepare_cached("DELETE FROM working_files")?
            .execute([])?;
        let mut stmt =
            tx.prepare_cached("INSERT INTO working_files (filename, content_hash) VALUES (?, ?)")?;
        for key in keys {
            stmt.execute((&key.filename, &key.content_hash.to_string()))?;
        }
        Ok(())
    }
}
