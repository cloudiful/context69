//! Path-based language classification for Git content (issue #681 work unit
//! 3B2).
//!
//! Classification is pure and lexical: nothing is opened, no provider is asked,
//! and the same path always yields the same token, so a stored manifest is
//! reproducible from the tree listing alone. Unknown or unrecognised names
//! classify as [`UNKNOWN_LANGUAGE`] rather than guessing.
//!
//! Tokens are bounded, lowercase identifiers of at most 32 characters, which is
//! the shape `chk_git_generation_files_language` stores: a new language is
//! classified without a migration, and the classifier test asserts every token
//! it can produce satisfies that shape.

/// Classification result for a path no rule recognises.
pub(crate) const UNKNOWN_LANGUAGE: &str = "unknown";

/// Classifies one repository-relative path from its file name and extension.
pub(crate) fn language_for_path(path: &str) -> &'static str {
    let Some(name) = path.rsplit('/').next().filter(|name| !name.is_empty()) else {
        return UNKNOWN_LANGUAGE;
    };
    if let Some(language) = language_for_file_name(name) {
        return language;
    }
    // A leading dot belongs to the name (`.bashrc`), not to an extension, so a
    // dotfile has no extension to classify.
    match name.rfind('.') {
        Some(dot) if dot > 0 => {
            language_for_extension(&name[dot + 1..]).unwrap_or(UNKNOWN_LANGUAGE)
        }
        _ => UNKNOWN_LANGUAGE,
    }
}

/// Whole file names that carry their language in the name rather than an
/// extension. Matched case-insensitively, because repositories do spell these
/// both ways.
fn language_for_file_name(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    Some(match lower.as_str() {
        "dockerfile" | "containerfile" => "dockerfile",
        "makefile" | "gnumakefile" | "justfile" | "kbuild" => "makefile",
        "cmakelists.txt" => "cmake",
        "jenkinsfile" => "groovy",
        "rakefile" | "gemfile" | "podfile" | "cartfile" | "fastfile" | "brewfile"
        | "vagrantfile" | "berksfile" | "guardfile" | "capfile" | "thorfile" | "appfile"
        | "snapfile" => "ruby",
        ".gitignore" | ".gitattributes" | ".gitmodules" => "gitconfig",
        ".editorconfig" => "editorconfig",
        ".env" => "dotenv",
        _ => return None,
    })
}

fn language_for_extension(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "rs" => "rust",
        "toml" => "toml",
        "py" | "pyi" => "python",
        "rb" => "ruby",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "vue" => "vue",
        "svelte" => "svelte",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "scala" | "sc" => "scala",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hxx" => "cpp",
        "cs" => "csharp",
        "m" => "objc",
        "mm" => "objcpp",
        "php" => "php",
        "pl" | "pm" => "perl",
        "lua" => "lua",
        "r" | "R" => "r",
        "dart" => "dart",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "erlang",
        "hs" => "haskell",
        "ml" | "mli" => "ocaml",
        "fs" | "fsx" => "fsharp",
        "clj" | "cljs" | "cljc" => "clojure",
        "groovy" => "groovy",
        "zig" => "zig",
        "nim" => "nim",
        "cr" => "crystal",
        "sol" => "solidity",
        "elm" => "elm",
        "purs" => "purescript",
        "re" | "res" => "reason",
        "gd" => "gdscript",
        "sql" => "sql",
        "graphql" | "gql" => "graphql",
        "proto" => "protobuf",
        "prisma" => "prisma",
        "tf" | "tfvars" | "tftpl" => "terraform",
        "hcl" => "hcl",
        "nix" => "nix",
        "yaml" | "yml" => "yaml",
        "json" | "jsonc" | "json5" => "json",
        "ini" | "cfg" | "conf" | "properties" => "ini",
        "xml" | "xsd" | "xsl" => "xml",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" | "sass" => "scss",
        "less" => "less",
        "md" | "mdx" => "markdown",
        "rst" => "restructuredtext",
        "txt" => "text",
        "po" | "pot" => "gettext",
        "svg" => "svg",
        "ipynb" => "jupyter",
        "sh" | "bash" | "zsh" | "ksh" => "shell",
        "fish" => "fish",
        "ps1" | "psm1" => "powershell",
        "bat" | "cmd" => "batch",
        "asm" | "s" => "assembly",
        "f90" | "f95" | "f03" => "fortran",
        "pas" => "pascal",
        "v" | "sv" | "svh" => "verilog",
        "vhd" | "vhdl" => "vhdl",
        "tex" => "latex",
        "diff" | "patch" => "diff",
        "env" => "dotenv",
        "mk" => "makefile",
        _ => return None,
    })
}

#[cfg(test)]
#[path = "classify_tests.rs"]
mod tests;
