use std::sync::LazyLock;

use regex::Regex;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    AttrStyle, Attribute, Expr, ExprLit, ImplItemConst, ImplItemFn, ImplItemType, ItemConst, ItemEnum, ItemFn,
    ItemImpl, ItemMod, ItemStatic, ItemStruct, ItemTrait, ItemType, Lit, Meta, MetaNameValue, Visibility,
};

use super::hygiene::{is_rust, read_text, refuse, relative};
use super::parsed::{Source, is_test, offenses};
use super::{Refusal, root, scanned};

const SELF: &str = "crates/writebook-dev/src/lint/comments";

const MARKERS: [&str; 4] = ["TODO", "FIXME", "XXX", "HACK"];

const BANNER_RULES: [&str; 4] = ["---", "===", "***", "###"];

const NOT_SCANNED: &str = "/fixtures/";

static HISTORY_OR_EXCUSE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:for now|previously|used to|temporary|workaround)\b")
        .expect("HISTORY_OR_EXCUSE is a valid regex")
});

const CODE_STARTS: [&str; 5] = ["let ", "fn ", "impl ", "pub fn ", "pub(crate) fn "];

pub(super) fn no_comment_is_a_banner_a_marker_or_code() -> Option<Refusal> {
    let root = root();
    let mut comments = Vec::new();
    for file in scanned() {
        let path = relative(&root, &file);
        if is_rust(&file) && !path.starts_with(SELF) {
            for (index, line) in read_text(&file).lines().enumerate() {
                if let Some(kind) = refused_comment(line) {
                    comments.push(format!("{path}:{}: {kind}: {}", index + 1, line.trim()));
                }
            }
        }
    }
    refuse(
        &comments,
        "comments are banners, to-do markers or code: split a banner's subjects into files, \
         record unfinished work outside the code, and delete commented-out code",
    )
}

fn refused_comment(line: &str) -> Option<&'static str> {
    let line = line.trim();
    if let Some(text) = line.strip_prefix("//") {
        let documentation = text.starts_with('/') || text.starts_with('!');
        let text = text.trim_start_matches(['/', '!']).trim();
        if is_banner(text) {
            Some("banner")
        } else if MARKERS.iter().any(|marker| text.contains(marker)) {
            Some("marker")
        } else if !documentation && is_code(text) {
            Some("code")
        } else {
            None
        }
    } else {
        None
    }
}

fn is_banner(text: &str) -> bool {
    BANNER_RULES
        .iter()
        .any(|rule| text.starts_with(rule) || text.ends_with(rule))
}

fn is_code(text: &str) -> bool {
    text.ends_with(';') || text.ends_with('{') || text == "}" || CODE_STARTS.iter().any(|start| text.starts_with(start))
}

pub(super) fn no_test_holds_a_comment() -> Option<Refusal> {
    refuse(
        &offenses(|source| {
            let text = read_text(&root().join(&source.path));
            let lines: Vec<&str> = text.lines().collect();
            test_comment_lines(&source.file, &text)
                .into_iter()
                .map(|line| format!("{}:{line}: {}", source.path, lines[line - 1].trim()))
                .collect()
        }),
        "comments inside a test: delete them; the test's name says the behaviour, a named setup \
         says the arrangement, and the assertion says what is checked",
    )
}

pub(super) fn no_test_is_documented_without_a_link() -> Option<Refusal> {
    refuse(
        &offenses(|source| at_lines(source, unlinked_test_docs(&source.file))),
        "doc comments on a test that link nothing: delete them; the name says the behaviour, and \
         a doc comment is only the link to the issue the test holds",
    )
}

pub(super) fn no_private_item_is_documented() -> Option<Refusal> {
    refuse(
        &offenses(|source| at_lines(source, private_docs(&source.file))),
        "doc comments on items that aren't pub: delete them; rename the item until it needs none, \
         or make it pub if callers outside the module read it",
    )
}

pub(super) fn no_comment_tells_history_or_makes_an_excuse() -> Option<Refusal> {
    refuse(
        &comment_violations(|_| true, tells_history_or_excuses),
        "comments telling history or making an excuse (for now, previously, used to, temporary, \
         workaround): delete them; git holds the history, and a workaround becomes code that \
         needs no excuse",
    )
}

fn at_lines(source: &Source, lines: Vec<usize>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| format!("{}:{line}", source.path))
        .collect()
}

fn comment_violations(
    is_scanned: impl Fn(&str) -> bool,
    refused: impl Fn(&str) -> Option<&'static str>,
) -> Vec<String> {
    let root = root();
    let mut violations = Vec::new();
    for file in scanned() {
        let path = relative(&root, &file);
        if is_rust(&file) && !path.starts_with(SELF) && !path.contains(NOT_SCANNED) && is_scanned(&path) {
            for (index, line) in read_text(&file).lines().enumerate() {
                if let Some(text) = comment_text(line)
                    && let Some(kind) = refused(text)
                {
                    violations.push(format!("{path}:{}: {kind}: {}", index + 1, line.trim()));
                }
            }
        }
    }
    violations
}

fn comment_text(line: &str) -> Option<&str> {
    line.trim_start()
        .strip_prefix("//")
        .map(|text| text.trim_start_matches(['/', '!']).trim())
}

fn tells_history_or_excuses(text: &str) -> Option<&'static str> {
    HISTORY_OR_EXCUSE.is_match(text).then_some("history or an excuse")
}

fn test_comment_lines(file: &syn::File, text: &str) -> Vec<usize> {
    let mut bodies = TestBodies::default();
    bodies.visit_file(file);
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim_start()))
        .filter(|(line, comment)| {
            comment.starts_with("//")
                && !comment.starts_with("///")
                && !comment.starts_with("//!")
                && bodies
                    .lines
                    .iter()
                    .any(|&(first, last)| first <= *line && *line <= last)
        })
        .map(|(line, _)| line)
        .collect()
}

fn unlinked_test_docs(file: &syn::File) -> Vec<usize> {
    let mut docs = TestDocs::default();
    docs.visit_file(file);
    docs.lines
}

fn private_docs(file: &syn::File) -> Vec<usize> {
    let mut docs = PrivateDocs::default();
    docs.visit_file(file);
    docs.lines
}

fn docs_of(attributes: &[Attribute]) -> Vec<(usize, String)> {
    attributes
        .iter()
        .filter(|attribute| matches!(attribute.style, AttrStyle::Outer))
        .filter_map(|attribute| match &attribute.meta {
            Meta::NameValue(MetaNameValue {
                path,
                value: Expr::Lit(ExprLit {
                    lit: Lit::Str(text), ..
                }),
                ..
            }) if path.is_ident("doc") => Some((attribute.span().start().line, text.value())),
            _ => None,
        })
        .collect()
}

fn is_test_function(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "test")
    })
}

#[derive(Default)]
struct TestBodies {
    inside_test_module: bool,
    lines: Vec<(usize, usize)>,
}

impl TestBodies {
    fn body(&mut self, block: &syn::Block) {
        let span = block.span();
        self.lines.push((span.start().line, span.end().line));
    }
}

impl<'ast> Visit<'ast> for TestBodies {
    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        let before = self.inside_test_module;
        self.inside_test_module |= is_test(&module.attrs);
        visit::visit_item_mod(self, module);
        self.inside_test_module = before;
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        if self.inside_test_module || is_test(&function.attrs) {
            self.body(&function.block);
        } else {
            visit::visit_item_fn(self, function);
        }
    }

    fn visit_impl_item_fn(&mut self, function: &'ast ImplItemFn) {
        if self.inside_test_module || is_test(&function.attrs) {
            self.body(&function.block);
        } else {
            visit::visit_impl_item_fn(self, function);
        }
    }
}

#[derive(Default)]
struct TestDocs {
    lines: Vec<usize>,
}

impl TestDocs {
    fn check(&mut self, attributes: &[Attribute]) {
        let docs = docs_of(attributes);
        if is_test_function(attributes)
            && let Some((line, _)) = docs.first()
            && !docs
                .iter()
                .any(|(_, text)| text.contains("http://") || text.contains("https://"))
        {
            self.lines.push(*line);
        }
    }
}

impl<'ast> Visit<'ast> for TestDocs {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.check(&function.attrs);
        visit::visit_item_fn(self, function);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast ImplItemFn) {
        self.check(&function.attrs);
        visit::visit_impl_item_fn(self, function);
    }
}

#[derive(Default)]
struct PrivateDocs {
    inside_trait_impl: bool,
    lines: Vec<usize>,
}

impl PrivateDocs {
    fn check(&mut self, attributes: &[Attribute], visibility: &Visibility) {
        if matches!(visibility, Visibility::Inherited)
            && !is_test_function(attributes)
            && let Some((line, _)) = docs_of(attributes).first()
        {
            self.lines.push(*line);
        }
    }
}

impl<'ast> Visit<'ast> for PrivateDocs {
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        self.check(&item.attrs, &item.vis);
        visit::visit_item_fn(self, item);
    }

    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        self.check(&item.attrs, &item.vis);
        for field in &item.fields {
            self.check(&field.attrs, &field.vis);
        }
    }

    fn visit_item_enum(&mut self, item: &'ast ItemEnum) {
        self.check(&item.attrs, &item.vis);
        for variant in &item.variants {
            self.check(&variant.attrs, &item.vis);
        }
    }

    fn visit_item_const(&mut self, item: &'ast ItemConst) {
        self.check(&item.attrs, &item.vis);
    }

    fn visit_item_static(&mut self, item: &'ast ItemStatic) {
        self.check(&item.attrs, &item.vis);
    }

    fn visit_item_type(&mut self, item: &'ast ItemType) {
        self.check(&item.attrs, &item.vis);
    }

    fn visit_item_trait(&mut self, item: &'ast ItemTrait) {
        self.check(&item.attrs, &item.vis);
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        self.check(&item.attrs, &item.vis);
        visit::visit_item_mod(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        let before = self.inside_trait_impl;
        self.inside_trait_impl = item.trait_.is_some();
        visit::visit_item_impl(self, item);
        self.inside_trait_impl = before;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        if !self.inside_trait_impl {
            self.check(&item.attrs, &item.vis);
        }
        visit::visit_impl_item_fn(self, item);
    }

    fn visit_impl_item_const(&mut self, item: &'ast ImplItemConst) {
        if !self.inside_trait_impl {
            self.check(&item.attrs, &item.vis);
        }
    }

    fn visit_impl_item_type(&mut self, item: &'ast ImplItemType) {
        if !self.inside_trait_impl {
            self.check(&item.attrs, &item.vis);
        }
    }
}

#[cfg(test)]
mod tests;
