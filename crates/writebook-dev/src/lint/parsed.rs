use std::fmt::Display;
use std::sync::OnceLock;

use syn::spanned::Spanned;
use syn::{Attribute, Item};

use super::hygiene::{is_rust, is_test_code, read_text, relative};
use super::{root, scanned};

const FIXTURES: &str = "/fixtures/";

pub(super) struct Source {
    pub(super) path: String,
    pub(super) file: syn::File,
}

impl Source {
    pub(super) fn production(&self) -> Vec<Item> {
        if is_test_code(&self.path) {
            Vec::new()
        } else {
            without_tests(&self.file.items)
        }
    }

    pub(super) fn tests(&self) -> Vec<Item> {
        if is_test_code(&self.path) {
            self.file.items.clone()
        } else {
            tests_among(&self.file.items)
        }
    }
}

pub(super) struct Found<'a> {
    path: &'a str,
    pub(super) lines: Vec<String>,
}

impl<'a> Found<'a> {
    pub(super) fn new(path: &'a str) -> Self {
        Self {
            path,
            lines: Vec::new(),
        }
    }

    pub(super) fn at(&mut self, spanned: &impl Spanned, what: impl Display) {
        let line = spanned.span().start().line;
        self.lines.push(format!("{}:{line}: {what}", self.path));
    }
}

pub(super) fn sources() -> Vec<Source> {
    let root = root();
    scanned()
        .into_iter()
        .map(|file| (relative(&root, &file), file))
        .filter(|(path, file)| is_rust(file) && !path.contains(FIXTURES))
        .map(|(path, file)| {
            let file = syn::parse_file(&read_text(&file)).unwrap_or_else(|error| panic!("{path} parses: {error}"));
            Source { path, file }
        })
        .collect()
}

pub(super) fn offenses(check: impl Fn(&Source) -> Vec<String>) -> Vec<String> {
    sources().iter().flat_map(check).collect()
}

pub(super) fn is_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        let path = attribute.path();
        if path.is_ident("cfg") {
            attribute
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string() == "test")
        } else {
            path.segments.last().is_some_and(|segment| segment.ident == "test")
        }
    })
}

pub(super) fn lines_of(spanned: &impl Spanned) -> usize {
    let span = spanned.span();
    span.end().line - span.start().line + 1
}

pub(super) fn is_declared_by_a_test_module(path: &str) -> bool {
    static TEST_MODULES: OnceLock<Vec<String>> = OnceLock::new();
    TEST_MODULES
        .get_or_init(test_modules)
        .iter()
        .any(|module| declares(module, path))
}

fn test_modules() -> Vec<String> {
    let mut modules = Vec::new();
    for source in sources() {
        for item in &source.file.items {
            if let Item::Mod(module) = item
                && module.content.is_none()
                && is_test(&module.attrs)
            {
                modules.push(format!("{}/{}", module_directory(&source.path), module.ident));
            }
        }
    }
    modules
}

fn module_directory(path: &str) -> &str {
    let (directory, name) = path.rsplit_once('/').expect("a source lives under a scanned directory");
    if matches!(name, "main.rs" | "lib.rs" | "mod.rs") {
        directory
    } else {
        path.strip_suffix(".rs").expect("a source is a .rs file")
    }
}

fn declares(module: &str, path: &str) -> bool {
    path == format!("{module}.rs") || path.starts_with(&format!("{module}/"))
}

fn without_tests(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .filter(|item| !is_test(attributes_of(item)))
        .cloned()
        .map(|mut item| {
            if let Item::Mod(module) = &mut item
                && let Some((_, content)) = &mut module.content
            {
                *content = without_tests(content);
            }
            item
        })
        .collect()
}

fn tests_among(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .flat_map(|item| {
            if is_test(attributes_of(item)) {
                vec![item.clone()]
            } else if let Item::Mod(module) = item
                && let Some((_, content)) = &module.content
            {
                tests_among(content)
            } else {
                Vec::new()
            }
        })
        .collect()
}

fn attributes_of(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

#[cfg(test)]
mod tests;
