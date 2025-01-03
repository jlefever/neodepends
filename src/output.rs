use std::fs::File;
use std::io::LineWriter;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use rusqlite::params;
use rusqlite::Connection;

use crate::core::Change;
use crate::core::Content;
use crate::core::ContentId;
use crate::core::DepKind;
use crate::core::Entity;
use crate::core::EntityDep;
use crate::core::EntityId;
use crate::core::EntityKind;
use crate::core::PseudoCommitId;
use crate::core::SimpleEntityId;
use crate::matrix::dsm_v1;
use crate::matrix::dsm_v2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(strum::Display, strum::EnumIs, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "kebab-case")]
pub enum Resource {
    Entities,
    Deps,
    Changes,
    Contents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(strum::Display, strum::EnumIs, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "kebab-case")]
pub enum OutputFormat {
    Csvs,
    Jsonl,
    Sqlite,
    DsmV1,
    DsmV2,
}

impl OutputFormat {
    pub fn open<P: AsRef<Path>>(&self, path: P) -> Result<Box<dyn Writer + Sync>> {
        Ok(match self {
            OutputFormat::Csvs => Box::new(CsvsWriter::open(path)?),
            OutputFormat::Jsonl => Box::new(JsonlWriter::open(path)?),
            OutputFormat::Sqlite => Box::new(SqliteWriter::open(path)?),
            OutputFormat::DsmV1 => Box::new(DsmWriter::open(path, Dsm::V1)?),
            OutputFormat::DsmV2 => Box::new(DsmWriter::open(path, Dsm::V2)?),
        })
    }
}

pub trait Writer {
    fn supports(&self, resource: Resource) -> bool;
    fn is_single_structure(&self) -> bool;
    fn write_entity(&self, value: Entity) -> Result<()>;
    fn write_dep(&self, value: EntityDep) -> Result<()>;
    fn write_change(&self, value: Change) -> Result<()>;
    fn write_content(&self, value: Content) -> Result<()>;
    fn finalize(&mut self) -> Result<()>;
}

#[derive(Debug)]
struct CsvsWriter {
    entities: Mutex<csv::Writer<File>>,
    deps: Mutex<csv::Writer<File>>,
    changes: Mutex<csv::Writer<File>>,
    contents: Mutex<csv::Writer<File>>,
}

impl CsvsWriter {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        std::fs::create_dir_all(path.as_ref())?;
        let entities = Mutex::new(csv::Writer::from_path(path.as_ref().join("entities.csv"))?);
        let deps = Mutex::new(csv::Writer::from_path(path.as_ref().join("deps.csv"))?);
        let changes = Mutex::new(csv::Writer::from_path(path.as_ref().join("changes.csv"))?);
        let contents = Mutex::new(csv::Writer::from_path(path.as_ref().join("contents.csv"))?);
        Ok(Self { entities, deps, changes, contents })
    }
}

impl Writer for CsvsWriter {
    fn supports(&self, _: Resource) -> bool {
        true
    }

    fn is_single_structure(&self) -> bool {
        false
    }

    fn write_entity(&self, value: Entity) -> Result<()> {
        Ok(self.entities.lock().unwrap().serialize(EntityRow::from(value))?)
    }

    fn write_dep(&self, value: EntityDep) -> Result<()> {
        Ok(self.deps.lock().unwrap().serialize(EntityDepRow::from(value))?)
    }

    fn write_change(&self, value: Change) -> Result<()> {
        Ok(self.changes.lock().unwrap().serialize(value)?)
    }

    fn write_content(&self, value: Content) -> Result<()> {
        Ok(self.contents.lock().unwrap().serialize(value)?)
    }

    fn finalize(&mut self) -> Result<()> {
        self.entities.lock().unwrap().flush()?;
        self.deps.lock().unwrap().flush()?;
        self.changes.lock().unwrap().flush()?;
        self.contents.lock().unwrap().flush()?;
        Ok(())
    }
}

#[derive(Debug)]
struct JsonlWriter {
    file: Mutex<LineWriter<File>>,
}

impl JsonlWriter {
    fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(Self { file: Mutex::new(LineWriter::new(File::create(path)?)) })
    }

    fn write<S: serde::Serialize>(&self, value: S) -> Result<()> {
        Ok(write!(self.file.lock().unwrap(), "{}\n", serde_json::to_string(&value)?)?)
    }
}

impl Writer for JsonlWriter {
    fn supports(&self, _: Resource) -> bool {
        true
    }

    fn is_single_structure(&self) -> bool {
        false
    }

    fn write_entity(&self, value: Entity) -> Result<()> {
        self.write(EntityRow::from(value))
    }

    fn write_dep(&self, value: EntityDep) -> Result<()> {
        self.write(EntityDepRow::from(value))
    }

    fn write_change(&self, value: Change) -> Result<()> {
        self.write(value)
    }

    fn write_content(&self, value: Content) -> Result<()> {
        self.write(value)
    }

    fn finalize(&mut self) -> Result<()> {
        Ok(self.file.lock().unwrap().flush()?)
    }
}

#[derive(Debug)]
enum Dsm {
    V1,
    V2,
}

#[derive(Debug)]
struct DsmWriter {
    path: PathBuf,
    dsm: Dsm,
    entities: Mutex<Vec<Entity>>,
    deps: Mutex<Vec<EntityDep>>,
    changes: Mutex<Vec<Change>>,
}

impl DsmWriter {
    fn open<P: AsRef<Path>>(path: P, dsm: Dsm) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            dsm,
            entities: Default::default(),
            deps: Default::default(),
            changes: Default::default(),
        })
    }
}

impl Writer for DsmWriter {
    fn supports(&self, resource: Resource) -> bool {
        match resource {
            Resource::Entities => true,
            Resource::Deps => true,
            Resource::Changes => true,
            _ => false,
        }
    }

    fn is_single_structure(&self) -> bool {
        true
    }

    fn write_entity(&self, value: Entity) -> Result<()> {
        self.entities.lock().unwrap().push(value);
        Ok(())
    }

    fn write_dep(&self, value: EntityDep) -> Result<()> {
        self.deps.lock().unwrap().push(value);
        Ok(())
    }

    fn write_change(&self, value: Change) -> Result<()> {
        self.changes.lock().unwrap().push(value);
        Ok(())
    }

    fn write_content(&self, _: Content) -> Result<()> {
        Ok(())
    }

    fn finalize(&mut self) -> Result<()> {
        let entities = self.entities.lock().unwrap();
        let deps = self.deps.lock().unwrap();
        let changes = self.changes.lock().unwrap();

        let text = match self.dsm {
            Dsm::V1 => dsm_v1(&entities, &deps, &changes),
            Dsm::V2 => dsm_v2(&entities, &deps, &changes),
        };

        Ok(File::create(&self.path)?.write_all(text.as_bytes())?)
    }
}

#[derive(Debug)]
struct SqliteWriter {
    conn: Mutex<Connection>,
}

impl SqliteWriter {
    fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SQLITE_INIT)?;
        Ok(Self { conn: Mutex::new(conn) })
    }
}

impl Writer for SqliteWriter {
    fn supports(&self, _: Resource) -> bool {
        true
    }

    fn is_single_structure(&self) -> bool {
        false
    }

    fn write_entity(&self, value: Entity) -> Result<()> {
        let value = EntityRow::from(value);

        self.conn
            .lock()
            .unwrap()
            .prepare_cached("INSERT INTO entities VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")?
            .execute(params![
                &value.id,
                &value.parent_id,
                &value.name,
                &value.kind,
                &value.start_byte,
                &value.start_row,
                &value.start_column,
                &value.end_byte,
                &value.end_row,
                &value.end_column,
                &value.content_id,
                &value.simple_id,
            ])?;

        Ok(())
    }

    fn write_dep(&self, value: EntityDep) -> Result<()> {
        let value = EntityDepRow::from(value);

        self.conn
            .lock()
            .unwrap()
            .prepare_cached("INSERT INTO deps VALUES (?, ?, ?, ?, ?)")?
            .execute(params![&value.src, &value.tgt, &value.kind, &value.row, &value.commit_id])?;

        Ok(())
    }

    fn write_change(&self, value: Change) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .prepare_cached("INSERT INTO changes VALUES (?, ?, ?, ?, ?)")?
            .execute(params![
                &value.simple_id,
                &value.commit_id,
                &value.kind,
                &value.dels,
                &value.adds,
            ])?;

        Ok(())
    }

    fn write_content(&self, value: Content) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .prepare_cached("INSERT INTO contents VALUES (?, ?)")?
            .execute(params![&value.id, &value.content])?;

        Ok(())
    }

    fn finalize(&mut self) -> Result<()> {
        self.conn.lock().unwrap().execute_batch("VACUUM;")?;

        Ok(())
    }
}

const SQLITE_INIT: &'static str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    PRAGMA wal_checkpoint(TRUNCATE);

    CREATE TABLE IF NOT EXISTS entities (
        id BLOB NOT NULL PRIMARY KEY,
        parent_id BLOB,
        name TEXT NOT NULL,
        kind TEXT NOT NULL,
        start_byte INT NOT NULL,
        start_row INT NOT NULL,
        start_column INT NOT NULL,
        end_byte INT NOT NULL,
        end_row INT NOT NULL,
        end_column INT NOT NULL,
        content_id BLOB NOT NULL,
        simple_id BLOB NOT NULL
    );

    CREATE TABLE IF NOT EXISTS deps (
        src BLOB NOT NULL,
        tgt BLOB NOT NULL,
        kind TEXT NOT NULL,
        row INT NOT NULL,
        commit_id BLOB
    );

    CREATE TABLE IF NOT EXISTS changes (
        simple_id BLOB NOT NULL,
        commit_id BLOB NOT NULL,
        kind TEXT NOT NULL,
        dels INT NOT NULL,
        adds INT NOT NULL,
        PRIMARY KEY (simple_id, commit_id)
    );

    CREATE TABLE IF NOT EXISTS contents (
        id BLOB NOT NULL PRIMARY KEY,
        content TEXT NOT NULL
    );
";

#[derive(Debug)]
#[derive(serde::Serialize)]
struct EntityRow {
    id: EntityId,
    parent_id: Option<EntityId>,
    name: String,
    kind: EntityKind,
    start_byte: usize,
    start_row: usize,
    start_column: usize,
    end_byte: usize,
    end_row: usize,
    end_column: usize,
    content_id: ContentId,
    simple_id: SimpleEntityId,
}

impl EntityRow {
    fn from(entity: Entity) -> Self {
        let location = entity.location();
        Self {
            id: entity.id,
            parent_id: entity.parent_id,
            name: entity.name,
            kind: entity.kind,
            start_byte: location.start.byte,
            start_row: location.start.row,
            start_column: location.start.column,
            end_byte: location.end.byte,
            end_row: location.end.row,
            end_column: location.end.column,
            content_id: entity.content_id,
            simple_id: entity.simple_id,
        }
    }
}

#[derive(Debug)]
#[derive(serde::Serialize)]
struct EntityDepRow {
    src: EntityId,
    tgt: EntityId,
    kind: DepKind,
    row: usize,
    commit_id: PseudoCommitId,
}

impl EntityDepRow {
    fn from(entity_dep: EntityDep) -> Self {
        Self {
            src: entity_dep.src,
            tgt: entity_dep.tgt,
            kind: entity_dep.kind,
            row: entity_dep.position.row(),
            commit_id: entity_dep.commit_id,
        }
    }
}
