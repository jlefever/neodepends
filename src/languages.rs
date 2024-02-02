use std::collections::HashMap;

use lazy_static::lazy_static;
use tree_sitter_stack_graphs::loader::LanguageConfiguration;
use tree_sitter_stack_graphs::NoCancellation;

lazy_static! {
    static ref JAVA_SG: LanguageConfiguration =
        tree_sitter_stack_graphs_java::language_configuration(&NoCancellation);
    static ref JAVASCRIPT_SG: LanguageConfiguration =
        tree_sitter_stack_graphs_javascript::language_configuration(&NoCancellation);
    static ref PYTHON_SG: LanguageConfiguration =
        tree_sitter_stack_graphs_python::language_configuration(&NoCancellation);
    static ref TYPESCRIPT_SG: LanguageConfiguration =
        tree_sitter_stack_graphs_typescript::language_configuration(&NoCancellation);
    static ref FILE_TYPES: HashMap<String, Lang> = {
        let mut map = HashMap::new();
        insert_file_types(&mut map, &JAVA_SG.file_types, Lang::Java);
        insert_file_types(&mut map, &JAVASCRIPT_SG.file_types, Lang::JavaScript);
        insert_file_types(&mut map, &PYTHON_SG.file_types, Lang::Python);
        insert_file_types(&mut map, &TYPESCRIPT_SG.file_types, Lang::TypeScript);
        map
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Java,
    JavaScript,
    Python,
    TypeScript,
}

impl Lang {
    pub fn from_filename<S: AsRef<str>>(filename: S) -> Option<Lang> {
        let ext = filename.as_ref().split(".").last()?.to_lowercase();
        FILE_TYPES.get(&ext).map(|&l| l)
    }

    pub fn sg_config(&self) -> &LanguageConfiguration {
        match &self {
            Lang::Java => &*JAVA_SG,
            Lang::JavaScript => &*JAVASCRIPT_SG,
            Lang::Python => &*PYTHON_SG,
            Lang::TypeScript => &*TYPESCRIPT_SG,
        }
    }
}

fn insert_file_types(map: &mut HashMap<String, Lang>, file_types: &[String], lang: Lang) {
    for file_type in file_types {
        map.insert(file_type.clone(), lang);
    }
}
