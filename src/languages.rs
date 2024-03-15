use core::panic;
use std::fmt::Display;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use clap::ValueEnum;
use lazy_static::lazy_static;
use strum::AsRefStr;
use tree_sitter::Language;
use tree_sitter::Query;
use tree_sitter_stack_graphs::StackGraphLanguage;

lazy_static! {
    static ref C: LangConfig = LangConfig::new("c", tree_sitter_c::language(), Some("cpp"));
    static ref CPP: LangConfig = LangConfig::new("cpp", tree_sitter_cpp::language(), Some("cpp"));
    static ref GO: LangConfig = LangConfig::new("go", tree_sitter_go::language(), Some("go"));
    static ref JAVA: LangConfig =
        LangConfig::new("java", tree_sitter_java::language(), Some("java"));
    static ref JAVASCRIPT: LangConfig =
        LangConfig::new("javascript", tree_sitter_javascript::language(), None);
    static ref KOTLIN: LangConfig =
        LangConfig::new("kotlin", tree_sitter_kotlin::language(), Some("kotlin"));
    static ref PYTHON: LangConfig =
        LangConfig::new("python", tree_sitter_python::language(), Some("python"));
    static ref RUBY: LangConfig =
        LangConfig::new("ruby", tree_sitter_ruby::language(), Some("ruby"));
    static ref TYPESCRIPT: LangConfig =
        LangConfig::new("typescript", tree_sitter_typescript::language_typescript(), None);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, ValueEnum, AsRefStr)]
#[clap(rename_all = "lower")]
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
    pub fn config(&self) -> &LangConfig {
        match &self {
            Lang::C => &C,
            Lang::Cpp => &CPP,
            Lang::Go => &GO,
            Lang::Java => &JAVA,
            Lang::JavaScript => &JAVASCRIPT,
            Lang::Kotlin => &KOTLIN,
            Lang::Python => &PYTHON,
            Lang::Ruby => &RUBY,
            Lang::TypeScript => &TYPESCRIPT,
        }
    }

    pub fn from_ext<S: AsRef<str>>(ext: S) -> Option<Self> {
        match ext.as_ref().to_lowercase().as_ref() {
            "c" => Some(Lang::C),
            "c++" => Some(Lang::Cpp),
            "cc" => Some(Lang::Cpp),
            "cpp" => Some(Lang::Cpp),
            "cxx" => Some(Lang::Cpp),
            "go" => Some(Lang::Go),
            "h" => Some(Lang::C),
            "hh" => Some(Lang::Cpp),
            "hpp" => Some(Lang::Cpp),
            "hxx" => Some(Lang::Cpp),
            "java" => Some(Lang::Java),
            "js" => Some(Lang::JavaScript),
            "kt" => Some(Lang::Kotlin),
            "py" => Some(Lang::Python),
            "rb" => Some(Lang::Ruby),
            "ts" => Some(Lang::TypeScript),
            _ => None,
        }
    }

    pub fn from_filename<S: AsRef<str>>(filename: S) -> Option<Self> {
        filename.as_ref().split(".").last().and_then(Self::from_ext)
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::C,
            Self::Cpp,
            Self::Go,
            Self::Java,
            Self::JavaScript,
            Self::Kotlin,
            Self::Python,
            Self::Ruby,
            Self::TypeScript,
        ]
    }
}

impl Display for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.as_ref())
    }
}

pub struct LangConfig {
    pub language: Language,
    pub sgl: Option<StackGraphLanguage>,
    pub tag_query: Option<Query>,
    pub dep_query: Option<Query>,
    pub depends_lang: Option<&'static str>,
}

impl LangConfig {
    fn new(name: &'static str, language: Language, depends_lang: Option<&'static str>) -> Self {
        let path = PathBuf::from_str(&format!("../languages/{}", name)).unwrap();
        let sgl = try_read(path.join("stack-graphs.tsg"))
            .map(|x| StackGraphLanguage::from_str(language, &x).unwrap());
        let tag_query = try_read(path.join("tags.scm")).map(|x| Query::new(language, &x).unwrap());
        let dep_query = try_read(path.join("deps.scm")).map(|x| Query::new(language, &x).unwrap());
        Self { language, sgl, tag_query, dep_query, depends_lang }
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
