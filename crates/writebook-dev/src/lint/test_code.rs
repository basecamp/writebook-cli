use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprClosure, ExprMethodCall, ImplItemFn, Item, ItemFn, ItemMod, Pat, Type};

use super::Refusal;
use super::hygiene::refuse;
use super::parsed::{Found, is_test, lines_of, offenses, sources};

const POLLER: &str = "eventually";

const SLEEPS: [&str; 2] = ["sleep", "sleep_until"];

const CLOCK_READINGS: [&str; 3] = ["elapsed", "duration_since", "saturating_duration_since"];

const LONGEST_INLINE_TESTS: usize = 200;

pub(super) fn no_test_sleeps() -> Option<Refusal> {
    refuse(
        &test_offenses(|found| Calls {
            found,
            names: &SLEEPS,
            function: None,
            routes_may_call: true,
            in_a_route: false,
        }),
        "tests sleep: wait on a state with `eventually`, never on a fixed time",
    )
}

pub(super) fn no_test_reads_how_long_something_took() -> Option<Refusal> {
    refuse(
        &test_offenses(|found| Calls {
            found,
            names: &CLOCK_READINGS,
            function: None,
            routes_may_call: false,
            in_a_route: false,
        }),
        "tests read how long something took: compare two moments from one reading, read the \
         timeout the code asked for, or count the work done",
    )
}

pub(super) fn a_long_tests_module_has_its_own_file() -> Option<Refusal> {
    refuse(
        &offenses(|source| {
            let mut found = Found::new(&source.path);
            for item in &source.file.items {
                if let Item::Mod(module) = item
                    && is_inline_tests(module)
                    && lines_of(module) > LONGEST_INLINE_TESTS
                {
                    found.at(&module.ident, format!("{} lines", lines_of(module)));
                }
            }
            found.lines
        }),
        "inline `mod tests` are over two hundred lines: move each to a `tests.rs` beside its file",
    )
}

fn test_offenses(visitor: impl for<'a> Fn(Found<'a>) -> Calls<'a>) -> Vec<String> {
    let mut lines = Vec::new();
    for source in sources() {
        let mut calls = visitor(Found::new(&source.path));
        for item in source.tests() {
            calls.visit_item(&item);
        }
        lines.extend(calls.found.lines);
    }
    lines
}

fn is_inline_tests(module: &ItemMod) -> bool {
    module.content.is_some() && is_test(&module.attrs)
}

struct Calls<'a> {
    found: Found<'a>,
    names: &'static [&'static str],
    function: Option<String>,
    routes_may_call: bool,
    in_a_route: bool,
}

impl Calls<'_> {
    fn note(&mut self, name: &syn::Ident) {
        if self.function.as_deref() != Some(POLLER)
            && !(self.routes_may_call && self.in_a_route)
            && self.names.contains(&name.to_string().as_str())
        {
            self.found.at(name, name);
        }
    }
}

impl<'ast> Visit<'ast> for Calls<'_> {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let enclosing = self.function.replace(function.sig.ident.to_string());
        visit::visit_item_fn(self, function);
        self.function = enclosing;
    }

    fn visit_impl_item_fn(&mut self, function: &'ast ImplItemFn) {
        let enclosing = self.function.replace(function.sig.ident.to_string());
        visit::visit_impl_item_fn(self, function);
        self.function = enclosing;
    }

    fn visit_expr_closure(&mut self, closure: &'ast ExprClosure) {
        let enclosing = self.in_a_route;
        self.in_a_route |= is_a_route(closure);
        visit::visit_expr_closure(self, closure);
        self.in_a_route = enclosing;
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(called) = &*call.func
            && let Some(segment) = called.path.segments.last()
        {
            self.note(&segment.ident);
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        self.note(&call.method);
        visit::visit_expr_method_call(self, call);
    }
}

fn is_a_route(closure: &ExprClosure) -> bool {
    closure.inputs.iter().any(|input| match input {
        Pat::Ident(name) => name.ident == "request",
        Pat::Type(typed) => is_a_request(&typed.ty),
        _ => false,
    })
}

fn is_a_request(typed: &Type) -> bool {
    if let Type::Reference(reference) = typed
        && let Type::Path(path) = &*reference.elem
    {
        path.path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Request")
    } else {
        false
    }
}

#[cfg(test)]
mod tests;
