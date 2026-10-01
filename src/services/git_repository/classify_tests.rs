//! Path-based language classification (issue #681 work unit 3B2).

use super::{UNKNOWN_LANGUAGE, language_for_path};

/// The stored token shape `chk_git_generation_files_language` enforces: a bounded
/// lowercase identifier. Every token the classifier can produce must satisfy it,
/// or a manifest write would be rejected by the database.
fn is_storable_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 32
        && token.chars().next().is_some_and(|first| {
            first.is_ascii_lowercase()
                || first.is_ascii_digit()
                || matches!(first, '_' | '+' | '#' | '-')
        })
        && token.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '+' | '#' | '-')
        })
}

#[test]
fn extensions_classify_to_their_language() {
    for (path, expected) in [
        ("src/db/mod.rs", "rust"),
        (
            "crates/context69-contracts-sources/src/git_files.rs",
            "rust",
        ),
        ("Cargo.toml", "toml"),
        ("frontend/src/main.ts", "typescript"),
        ("frontend/src/App.vue", "vue"),
        ("app/components/Button.tsx", "tsx"),
        ("scripts/deploy.sh", "shell"),
        ("web/app.tsx", "tsx"),
        ("services/api/main.go", "go"),
        ("lib/parser.py", "python"),
        ("app/models/user.rb", "ruby"),
        ("src/main/java/com/example/App.java", "java"),
        ("app/Http/Controller.php", "php"),
        ("db/migrate/001.sql", "sql"),
        ("proto/service.proto", "protobuf"),
        ("infra/main.tf", "terraform"),
        ("config/app.yml", "yaml"),
        ("package.json", "json"),
        ("styles/main.css", "css"),
        ("styles/theme.scss", "scss"),
        ("README.md", "markdown"),
        ("docs/guide.rst", "restructuredtext"),
        ("analysis/plot.R", "r"),
        ("kernel/module.c", "c"),
        ("kernel/module.hpp", "cpp"),
        ("lib/analysis.ex", "elixir"),
        ("lib/analysis.erl", "erlang"),
        ("lib/Analysis.hs", "haskell"),
    ] {
        assert_eq!(
            language_for_path(path),
            expected,
            "{path} must classify as {expected}"
        );
    }
}

#[test]
fn whole_file_names_classify_case_insensitively() {
    for (path, expected) in [
        ("Dockerfile", "dockerfile"),
        ("docker/Dockerfile", "dockerfile"),
        ("build/Containerfile", "dockerfile"),
        ("Makefile", "makefile"),
        ("make/GNUmakefile", "makefile"),
        ("justfile", "makefile"),
        ("CMakeLists.txt", "cmake"),
        ("Jenkinsfile", "groovy"),
        ("Gemfile", "ruby"),
        ("ios/Podfile", "ruby"),
        (".gitignore", "gitconfig"),
        (".github/.gitattributes", "gitconfig"),
        (".editorconfig", "editorconfig"),
        (".env", "dotenv"),
    ] {
        assert_eq!(
            language_for_path(path),
            expected,
            "{path} must classify as {expected}"
        );
    }
}

#[test]
fn unclassifiable_names_stay_unknown_instead_of_guessing() {
    for path in [
        "docs/notes",
        "LICENSE",
        "Cargo.lock",
        "assets/logo.unknownext",
        ".bashrc",
        "archive.tar.gz.unknown",
        "",
        "nested//",
    ] {
        assert_eq!(
            language_for_path(path),
            UNKNOWN_LANGUAGE,
            "{path} must not be guessed into a language"
        );
    }
}

#[test]
fn classification_is_stable_and_storable_for_every_representative_path() {
    let paths = [
        "src/main.rs",
        "Dockerfile",
        "docs/README.md",
        "assets/logo.png",
        "weird/na me.with space.toml",
        "unicode/ünïcode.rs",
        ".env",
    ];
    for path in paths {
        let token = language_for_path(path);
        assert!(
            is_storable_token(token),
            "{path} produced {token}, which chk_git_generation_files_language \
             would reject: tokens must match ^[a-z0-9][a-z0-9_+#-]{{0,31}}$"
        );
        assert_eq!(
            language_for_path(path),
            token,
            "classification must be deterministic: {path}"
        );
    }
}
