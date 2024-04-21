use std::path::Path;
use std::path::PathBuf;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(strum::Display, strum::EnumIs, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "snake_case")]
pub enum Table {
    Entities,
    Deps,
    Changes,
    Contents,
}

pub trait TableWriter {
    fn write_entities<I>(&self, entities: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = Entity>;

    fn write_deps<I>(&self, deps: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = EntityDep>;

    fn write_changes<I>(&self, changes: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = Change>;

    fn write_contents<I>(&self, contents: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = Content>;
}

pub struct CsvWriter {
    dir: PathBuf,
}

impl CsvWriter {
    pub fn open<P: AsRef<Path>>(dir: P) -> anyhow::Result<CsvWriter> {
        std::fs::create_dir_all(dir.as_ref())?;
        Ok(Self { dir: dir.as_ref().to_owned() })
    }

    fn write<P, S, I>(path: P, values: I) -> anyhow::Result<()>
    where
        P: AsRef<Path>,
        S: serde::Serialize,
        I: IntoIterator<Item = S>,
    {
        let mut writer = csv::Writer::from_path(path)?;

        for value in values {
            writer.serialize(value)?;
        }

        Ok(())
    }
}

impl TableWriter for CsvWriter {
    fn write_entities<I>(&self, entities: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = Entity>,
    {
        let entities = entities.into_iter().map(EntityRow::from);
        Self::write(self.dir.join("entities.csv"), entities)
    }

    fn write_deps<I>(&self, deps: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = EntityDep>,
    {
        let deps = deps.into_iter().map(EntityDepRow::from);
        Self::write(self.dir.join("deps.csv"), deps)
    }

    fn write_changes<I>(&self, changes: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = Change>,
    {
        Self::write(self.dir.join("changes.csv"), changes)
    }

    fn write_contents<I>(&self, contents: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = Content>,
    {
        Self::write(self.dir.join("contents.csv"), contents)
    }
}

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
        Self {
            id: entity.id,
            parent_id: entity.parent_id,
            name: entity.name,
            kind: entity.kind,
            start_byte: entity.location.start.byte,
            start_row: entity.location.start.row,
            start_column: entity.location.start.column,
            end_byte: entity.location.end.byte,
            end_row: entity.location.end.row,
            end_column: entity.location.end.column,
            content_id: entity.content_id,
            simple_id: entity.simple_id,
        }
    }
}

#[derive(serde::Serialize)]
struct EntityDepRow {
    src: EntityId,
    tgt: EntityId,
    kind: DepKind,
    byte: Option<usize>,
    row: usize,
    column: Option<usize>,
    commit_id: PseudoCommitId,
}

impl EntityDepRow {
    fn from(entity_dep: EntityDep) -> Self {
        Self {
            src: entity_dep.src,
            tgt: entity_dep.tgt,
            kind: entity_dep.kind,
            byte: entity_dep.position.byte(),
            row: entity_dep.position.row(),
            column: entity_dep.position.column(),
            commit_id: entity_dep.commit_id,
        }
    }
}
