use std::collections::HashSet;
use std::path::Path;

use anyhow::bail;
use anyhow::Result;
use itertools::Itertools;
use rocksdb::DB;

use crate::core::FileKey;
use crate::resolution::StackGraphCtx;

static BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

#[derive(Debug)]
pub struct Store {
    db: DB,
}

pub struct LoadResponse {
    pub ctx: StackGraphCtx,
    pub failures: HashSet<FileKey>,
}

impl Store {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Store> {
        Ok(Self { db: DB::open_default(path)? })
    }

    pub fn save(&self, key: &FileKey, value: Option<StackGraphCtx>) -> Result<()> {
        let value = value.map(|mut c| c.encode().unwrap());
        let value = bincode::encode_to_vec(value, BINCODE_CONFIG)?;
        Ok(self.db.put(key.to_sha1_hash(), value)?)
    }

    pub fn load<'a, K>(&self, keys: K) -> Result<LoadResponse>
    where
        K: IntoIterator<Item = &'a FileKey>,
    {
        let keys = keys.into_iter().map(|k| k.clone()).collect_vec();
        let encoded_keys = keys.iter().map(|k| k.to_sha1_hash());

        let mut bytes: Vec<Vec<u8>> = Vec::new();
        let mut failures = HashSet::new();

        for (i, value) in self.db.multi_get(encoded_keys).into_iter().enumerate() {
            let key = &keys[i];

            if let Some(value) = value? {
                let value: Option<Vec<u8>> =
                    bincode::decode_from_slice(&value, BINCODE_CONFIG).unwrap().0;

                if let Some(value) = value {
                    bytes.push(value);
                } else {
                    failures.insert(key.clone());
                }
            } else {
                bail!("no value found for {}", key);
            }
        }

        let ctx = StackGraphCtx::decode_many(bytes.iter().map(|b| &b[..]))?;
        Ok(LoadResponse { ctx, failures })
    }

    pub fn find_missing<'a, K>(&self, keys: K) -> Vec<FileKey>
    where
        K: IntoIterator<Item = &'a FileKey>,
    {
        keys.into_iter()
            .filter(|k| !self.db.key_may_exist(k.to_sha1_hash()))
            .map(|k| k.clone())
            .collect()
    }
}
