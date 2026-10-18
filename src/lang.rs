//! Language detection and per-language editing properties.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    Html,
    Css,
    Scss,
    Less,
    JavaScript,
    TypeScript,
    Tsx,
    Php,
    Json,
    Markdown,
    Xml,
    Python,
    Ruby,
    Rust,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Shell,
    Sql,
    Lua,
    Yaml,
    Toml,
    Ini,
    Diff,
    PlainText,
}

/// User-defined extension mapping (extension -> language), set from settings.
static OVERRIDES: LazyLock<RwLock<HashMap<String, Language>>> = LazyLock::new(Default::default);

impl Language {
    pub const ALL: [Language; 27] = [
        Language::Html,
        Language::Css,
        Language::Scss,
        Language::Less,
        Language::JavaScript,
        Language::TypeScript,
        Language::Tsx,
        Language::Php,
        Language::Json,
        Language::Markdown,
        Language::Xml,
        Language::Python,
        Language::Ruby,
        Language::Rust,
        Language::Go,
        Language::Java,
        Language::C,
        Language::Cpp,
        Language::CSharp,
        Language::Shell,
        Language::Sql,
        Language::Lua,
        Language::Yaml,
        Language::Toml,
        Language::Ini,
        Language::Diff,
        Language::PlainText,
    ];

    /// Stable identifier used in settings and as injection name.
    pub fn id(self) -> &'static str {
        match self {
            Language::Html => "html",
            Language::Css => "css",
            Language::Scss => "scss",
            Language::Less => "less",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::Php => "php",
            Language::Json => "json",
            Language::Markdown => "markdown",
            Language::Xml => "xml",
            Language::Python => "python",
            Language::Ruby => "ruby",
            Language::Rust => "rust",
            Language::Go => "go",
            Language::Java => "java",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::CSharp => "csharp",
            Language::Shell => "shell",
            Language::Sql => "sql",
            Language::Lua => "lua",
            Language::Yaml => "yaml",
            Language::Toml => "toml",
            Language::Ini => "ini",
            Language::Diff => "diff",
            Language::PlainText => "text",
        }
    }

    pub fn from_id(id: &str) -> Option<Language> {
        Language::ALL.into_iter().find(|l| l.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Language::Html => "HTML",
            Language::Css => "CSS",
            Language::Scss => "SCSS",
            Language::Less => "Less",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Tsx => "TSX",
            Language::Php => "PHP",
            Language::Json => "JSON",
            Language::Markdown => "Markdown",
            Language::Xml => "XML",
            Language::Python => "Python",
            Language::Ruby => "Ruby",
            Language::Rust => "Rust",
            Language::Go => "Go",
            Language::Java => "Java",
            Language::C => "C",
            Language::Cpp => "C++",
            Language::CSharp => "C#",
            Language::Shell => "Shell",
            Language::Sql => "SQL",
            Language::Lua => "Lua",
            Language::Yaml => "YAML",
            Language::Toml => "TOML",
            Language::Ini => "INI / Konfiguration",
            Language::Diff => "Diff",
            Language::PlainText => "Text",
        }
    }

    /// Built-in file extensions (lowercase, without dot). Whole file names
    /// like `Makefile` are matched as well.
    pub fn default_extensions(self) -> &'static [&'static str] {
        match self {
            Language::Html => &["html", "htm", "xhtml", "shtml", "tpl", "twig", "vue", "svelte", "hbs", "ejs"],
            Language::Css => &["css"],
            Language::Scss => &["scss", "sass"],
            Language::Less => &["less"],
            Language::JavaScript => &["js", "mjs", "cjs", "jsx"],
            Language::TypeScript => &["ts", "mts", "cts"],
            Language::Tsx => &["tsx"],
            Language::Php => &["php", "phtml", "php3", "php4", "php5", "php7", "php8", "inc"],
            Language::Json => &["json", "jsonc", "webmanifest", "babelrc", "eslintrc", "prettierrc"],
            Language::Markdown => &["md", "markdown", "mdown", "mkd"],
            Language::Xml => &["xml", "svg", "xsl", "xslt", "rss", "atom", "plist", "xsd", "wsdl", "csproj", "config"],
            Language::Python => &["py", "pyw", "pyi"],
            Language::Ruby => &["rb", "erb", "rake", "gemspec", "gemfile"],
            Language::Rust => &["rs"],
            Language::Go => &["go"],
            Language::Java => &["java"],
            Language::C => &["c", "h"],
            Language::Cpp => &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "ino"],
            Language::CSharp => &["cs", "csx"],
            Language::Shell => &["sh", "bash", "zsh", "bashrc", "zshrc", "profile", "command"],
            Language::Sql => &["sql", "mysql", "pgsql"],
            Language::Lua => &["lua"],
            Language::Yaml => &["yml", "yaml"],
            Language::Toml => &["toml", "lock"],
            Language::Ini => &["ini", "cfg", "conf", "env", "htaccess", "properties", "editorconfig", "gitconfig", "npmrc"],
            Language::Diff => &["diff", "patch"],
            Language::PlainText => &["txt", "log"],
        }
    }

    /// Replaces the user-defined extension mapping.
    pub fn set_overrides(map: HashMap<String, Language>) {
        *OVERRIDES.write().unwrap() = map;
    }

    pub fn from_path(path: &str) -> Language {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_ascii_lowercase();
        // ".htaccess" has no stem: use the whole name as extension.
        let ext = match name.rsplit_once('.') {
            Some((_, ext)) => ext.to_string(),
            None => name.clone(),
        };
        if let Some(l) = OVERRIDES.read().unwrap().get(&ext) {
            return *l;
        }
        match name.as_str() {
            "dockerfile" | "makefile" | "gnumakefile" => return Language::Shell,
            "gemfile" | "rakefile" => return Language::Ruby,
            _ => {}
        }
        Language::ALL
            .into_iter()
            .find(|l| l.default_extensions().contains(&ext.as_str()))
            .unwrap_or(Language::PlainText)
    }

    /// Line comment token, if the language has one.
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Language::JavaScript
            | Language::TypeScript
            | Language::Tsx
            | Language::Php
            | Language::Rust
            | Language::Go
            | Language::Java
            | Language::C
            | Language::Cpp
            | Language::CSharp
            | Language::Scss
            | Language::Less
            | Language::Json => Some("//"),
            Language::Python | Language::Ruby | Language::Shell | Language::Yaml | Language::Toml => Some("#"),
            Language::Sql | Language::Lua => Some("--"),
            Language::Ini => Some(";"),
            _ => None,
        }
    }

    /// Block comment delimiters.
    pub fn block_comment(self) -> Option<(&'static str, &'static str)> {
        match self {
            Language::Html | Language::Xml | Language::Markdown => Some(("<!--", "-->")),
            Language::Css
            | Language::Scss
            | Language::Less
            | Language::JavaScript
            | Language::TypeScript
            | Language::Tsx
            | Language::Php
            | Language::Rust
            | Language::Go
            | Language::Java
            | Language::C
            | Language::Cpp
            | Language::CSharp
            | Language::Sql => Some(("/*", "*/")),
            Language::Lua => Some(("--[[", "]]")),
            _ => None,
        }
    }

    /// Whether `</` and `>` should trigger automatic tag closing.
    pub fn is_markup(self) -> bool {
        matches!(self, Language::Html | Language::Php | Language::Xml)
    }

    /// CSS-like languages share the CSS completions and outline.
    pub fn is_css(self) -> bool {
        matches!(self, Language::Css | Language::Scss | Language::Less)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension_and_name() {
        assert_eq!(Language::from_path("/a/b/index.HTML"), Language::Html);
        assert_eq!(Language::from_path("C:\\x\\.htaccess"), Language::Ini);
        assert_eq!(Language::from_path("main.rs"), Language::Rust);
        assert_eq!(Language::from_path("Dockerfile"), Language::Shell);
        assert_eq!(Language::from_path("README"), Language::PlainText);
        for l in Language::ALL {
            assert_eq!(Language::from_id(l.id()), Some(l));
        }
    }
}
