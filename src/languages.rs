use core::panic;
use std::collections::HashMap;
use std::fmt::Debug;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use itertools::Itertools;
use lazy_static::lazy_static;
use tree_sitter::Language;
use tree_sitter_c::language as c_language;
use tree_sitter_cpp::language as cpp_language;
use tree_sitter_go::language as go_language;
use tree_sitter_java::language as java_language;
use tree_sitter_javascript::language as js_language;
use tree_sitter_kotlin::language as kt_language;
use tree_sitter_python::language as py_language;
use tree_sitter_ruby::language as rb_language;
use tree_sitter_stack_graphs::StackGraphLanguage;
use tree_sitter_typescript::language_typescript as ts_language;

use crate::spec::Pathspec;
use crate::tagging::Tagger;

/// Each programming language supported by Neodepends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(strum::Display, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "lowercase")]
pub enum Lang {
    C,
    Cpp,
    Go,
    Java,
    JavaScript,
    Kotlin,
    Python,
    Ruby,
    TypeScript,
}

impl Lang {
    /// Get the language for a filename.
    pub fn of<S: AsRef<str>>(filename: S) -> Option<Self> {
        LANG_TABLE.get_lang(filename)
    }

    /// Get the [Pathspec] associated with this language.
    #[allow(dead_code)]
    pub fn pathspec(&self) -> &Pathspec {
        &self.config().pathspec
    }

    /// Create a [Pathspec] that matches many languages.
    pub fn pathspec_many<I: IntoIterator<Item = Lang>>(langs: I) -> Pathspec {
        Pathspec::new(LANG_TABLE.patterns(langs))
    }

    /// Get the [Tagger] associated with this language.
    pub fn tagger(&self) -> &Tagger {
        &self.config().tagger
    }

    /// Get the [StackGraphLanguage] associated with this language.
    pub fn sgl(&self) -> Option<Arc<StackGraphLanguage>> {
        self.config().sgl.clone()
    }

    /// Get the name of this language according to Depends.
    ///
    /// Intended to be passed to Depends as a command-line argument.
    pub fn depends_lang(&self) -> Option<&str> {
        self.config().depends_lang
    }

    fn config(&self) -> &LangConfig {
        match &self {
            Lang::C => &C,
            Lang::Cpp => &CPP,
            Lang::Go => &GO,
            Lang::Java => &JAVA,
            Lang::JavaScript => &JS,
            Lang::Kotlin => &KT,
            Lang::Python => &PY,
            Lang::Ruby => &RB,
            Lang::TypeScript => &TS,
        }
    }
}

struct LangConfig {
    pathspec: Pathspec,
    tagger: Tagger,
    sgl: Option<Arc<StackGraphLanguage>>,
    depends_lang: Option<&'static str>,
}

impl LangConfig {
    fn new(
        name: &'static str,
        language: Language,
        pathspec: Pathspec,
        depends_lang: Option<&'static str>,
    ) -> Self {
        let path = PathBuf::from_str(&format!("../languages/{}", name)).unwrap();
        let tagger = Tagger::new(Some(language), try_read(path.join("tags.scm")).as_deref());
        let sgl = try_read(path.join("stack-graphs.tsg"))
            .map(|x| Arc::new(StackGraphLanguage::from_str(language, &x).unwrap()));
        Self { pathspec, tagger, sgl, depends_lang }
    }
}

fn try_read<P: AsRef<Path>>(path: P) -> Option<String> {
    match File::open(path) {
        Ok(mut file) => {
            let mut buf = String::new();
            file.read_to_string(&mut buf).unwrap();
            Some(buf)
        }
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound => None,
            _ => panic!(),
        },
    }
}

#[derive(Debug, Default, Clone)]
struct LangLookupTable {
    special_files: HashMap<String, Lang>,
    extensions: HashMap<String, Lang>,
    patterns: HashMap<Lang, Vec<String>>,
}

impl LangLookupTable {
    fn new() -> Self {
        Self::default()
    }

    fn get_lang<S: AsRef<str>>(&self, filename: S) -> Option<Lang> {
        self.special_files
            .get(filename.as_ref())
            .or_else(|| {
                filename
                    .as_ref()
                    .to_lowercase()
                    .split(".")
                    .last()
                    .and_then(|e| self.extensions.get(e))
            })
            .copied()
    }

    fn pathspec(&self, lang: Lang) -> Pathspec {
        Pathspec::from_vec(self.patterns.get(&lang).unwrap().clone())
    }

    fn patterns<I>(&self, langs: I) -> Vec<&String>
    where
        I: IntoIterator<Item = Lang>,
    {
        let res = langs.into_iter().flat_map(|l| self.patterns.get(&l)).flatten().collect_vec();

        match res.is_empty() {
            true => self.patterns.values().flatten().unique().collect(),
            false => res,
        }
    }

    fn insert_special_file(&mut self, lang: Lang, special: &str) {
        self.special_files.insert(special.to_lowercase(), lang);
        self.patterns.entry(lang).or_default().push(special.to_string());
    }

    fn insert_extension(&mut self, lang: Lang, ext: &str) {
        self.extensions.insert(ext.to_lowercase(), lang);
        self.patterns.entry(lang).or_default().push(format!("*.{}", ext));
    }
}

lazy_static! {
    static ref LANG_TABLE: LangLookupTable = {
        let mut table = LangLookupTable::new();
        table.insert_extension(Lang::C, "c");
        table.insert_extension(Lang::Cpp, "c++");
        table.insert_extension(Lang::Cpp, "cc");
        table.insert_extension(Lang::Cpp, "cpp");
        table.insert_extension(Lang::Cpp, "cxx");
        table.insert_extension(Lang::Cpp, "h++");
        table.insert_extension(Lang::Cpp, "hh");
        table.insert_extension(Lang::Cpp, "hpp");
        table.insert_extension(Lang::Cpp, "hxx");
        table.insert_extension(Lang::Go, "go");
        table.insert_extension(Lang::Java, "java");
        table.insert_extension(Lang::JavaScript, "js");
        table.insert_extension(Lang::Kotlin, "kt");
        table.insert_extension(Lang::Python, "py");
        table.insert_extension(Lang::Ruby, "rb");
        table.insert_extension(Lang::TypeScript, "ts");
        table.insert_special_file(Lang::TypeScript, "tsconfig.json");
        table
    };
    static ref C: LangConfig =
        LangConfig::new("c", c_language(), LANG_TABLE.pathspec(Lang::C), Some("cpp"));
    static ref CPP: LangConfig =
        LangConfig::new("cpp", cpp_language(), LANG_TABLE.pathspec(Lang::Cpp), Some("cpp"));
    static ref GO: LangConfig =
        LangConfig::new("go", go_language(), LANG_TABLE.pathspec(Lang::Go), Some("go"));
    static ref JAVA: LangConfig =
        LangConfig::new("java", java_language(), LANG_TABLE.pathspec(Lang::Java), Some("java"));
    static ref JS: LangConfig =
        LangConfig::new("javascript", js_language(), LANG_TABLE.pathspec(Lang::JavaScript), None);
    static ref KT: LangConfig =
        LangConfig::new("kotlin", kt_language(), LANG_TABLE.pathspec(Lang::Kotlin), Some("kotlin"));
    static ref PY: LangConfig =
        LangConfig::new("python", py_language(), LANG_TABLE.pathspec(Lang::Python), Some("python"));
    static ref RB: LangConfig =
        LangConfig::new("ruby", rb_language(), LANG_TABLE.pathspec(Lang::Ruby), Some("ruby"));
    static ref TS: LangConfig =
        LangConfig::new("typescript", ts_language(), LANG_TABLE.pathspec(Lang::TypeScript), None);
}
