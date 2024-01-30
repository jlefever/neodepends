use std::collections::HashMap;

use lazy_static::lazy_static;
use tree_sitter_stack_graphs::loader::LanguageConfiguration;
use tree_sitter_stack_graphs::NoCancellation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Java,
    JavaScript,
    Python,
    TypeScript,
}

lazy_static! {
    static ref SG_JAVA: LanguageConfiguration =
        tree_sitter_stack_graphs_java::language_configuration(&NoCancellation);
    static ref SG_JAVASCRIPT: LanguageConfiguration =
        tree_sitter_stack_graphs_javascript::language_configuration(&NoCancellation);
    static ref SG_PYTHON: LanguageConfiguration =
        tree_sitter_stack_graphs_python::language_configuration(&NoCancellation);
    static ref SG_TYPESCRIPT: LanguageConfiguration =
        tree_sitter_stack_graphs_typescript::language_configuration(&NoCancellation);
    static ref SG_LANGUAGES: HashMap<String, Lang> = {
        let mut map = HashMap::new();
        for t in &SG_JAVA.file_types {
            map.insert(t.clone(), Lang::Java);
        }
        for t in &SG_JAVASCRIPT.file_types {
            map.insert(t.clone(), Lang::JavaScript);
        }
        for t in &SG_PYTHON.file_types {
            map.insert(t.clone(), Lang::Python);
        }
        for t in &SG_TYPESCRIPT.file_types {
            map.insert(t.clone(), Lang::TypeScript);
        }
        map
    };
}

impl Lang {
    pub fn from_filename<S: AsRef<str>>(filename: S) -> Option<Lang> {
        let ext = filename.as_ref().split(".").last()?.to_lowercase();
        SG_LANGUAGES.get(&ext).map(|&l| l)
    }

    pub fn config(&self) -> &'static LanguageConfiguration {
        match &self {
            Lang::Java => &*SG_JAVA,
            Lang::JavaScript => &*SG_JAVASCRIPT,
            Lang::Python => &*SG_PYTHON,
            Lang::TypeScript => &*SG_TYPESCRIPT,
        }
    }
}
