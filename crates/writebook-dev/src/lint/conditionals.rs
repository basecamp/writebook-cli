use syn::visit::{self, Visit};
use syn::{Block, Expr, ExprClosure, ExprMethodCall, ImplItemFn, ItemFn, Stmt, TraitItemFn};

use super::Refusal;
use super::hygiene::refuse;
use super::parsed::{Found, offenses};

const DEFAULTS: [&str; 2] = ["unwrap_or", "unwrap_or_else"];

const BRANCHES: [&str; 3] = ["map", "then", "then_some"];

pub(super) fn no_combinator_chain_stands_in_for_a_branch() -> Option<Refusal> {
    refuse(
        &offenses(|source| {
            let mut chains = Chains {
                found: Found::new(&source.path),
            };
            chains.visit_file(&source.file);
            chains.found.lines
        }),
        "chains end a `map`, `then` or `then_some` with `unwrap_or` or `unwrap_or_else`, which \
         picks between two values: write the `if` or the `match`",
    )
}

pub(super) fn a_guard_clause_is_only_the_first_statement_of_a_function() -> Option<Refusal> {
    refuse(
        &offenses(|source| {
            let mut guards = Guards {
                found: Found::new(&source.path),
                body: None,
            };
            guards.visit_file(&source.file);
            guards.found.lines
        }),
        "guard clauses (`let … else`, or an `if` without an `else` that returns) are not the first \
         statement of a function: make the rest of the function the \
         `else` of an `if` that produces a value",
    )
}

struct Chains<'a> {
    found: Found<'a>,
}

impl<'ast> Visit<'ast> for Chains<'_> {
    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if DEFAULTS.contains(&call.method.to_string().as_str())
            && let Expr::MethodCall(inner) = &*call.receiver
            && BRANCHES.contains(&inner.method.to_string().as_str())
        {
            self.found
                .at(&call.method, format!(".{}(..).{}(..)", inner.method, call.method));
        }
        visit::visit_expr_method_call(self, call);
    }
}

struct Guards<'a> {
    found: Found<'a>,
    body: Option<*const Block>,
}

impl Guards<'_> {
    fn function(&mut self, body: &Block) {
        let enclosing = self.body.replace(body);
        self.visit_block(body);
        self.body = enclosing;
    }
}

impl<'ast> Visit<'ast> for Guards<'_> {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.function(&function.block);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast ImplItemFn) {
        self.function(&function.block);
    }

    fn visit_trait_item_fn(&mut self, function: &'ast TraitItemFn) {
        if let Some(body) = &function.default {
            self.function(body);
        }
    }

    fn visit_expr_closure(&mut self, closure: &'ast ExprClosure) {
        if let Expr::Block(body) = &*closure.body {
            self.function(&body.block);
        } else {
            visit::visit_expr_closure(self, closure);
        }
    }

    fn visit_block(&mut self, block: &'ast Block) {
        let is_the_body = self.body.is_some_and(|body| std::ptr::eq(body, block));
        for (index, statement) in block.stmts.iter().enumerate() {
            if is_guard(statement) && !(is_the_body && index == 0) {
                self.found.at(statement, "a guard clause");
            }
        }
        visit::visit_block(self, block);
    }
}

fn is_guard(statement: &Stmt) -> bool {
    match statement {
        Stmt::Local(local) => local.init.as_ref().is_some_and(|init| {
            init.diverge
                .as_ref()
                .is_some_and(|(_, otherwise)| !skips_an_iteration(otherwise))
        }),
        Stmt::Expr(Expr::If(branch), _) => branch.else_branch.is_none() && returns(&branch.then_branch),
        _ => false,
    }
}

fn returns(block: &Block) -> bool {
    block
        .stmts
        .last()
        .is_some_and(|last| matches!(last, Stmt::Expr(Expr::Return(_), _)))
}

fn skips_an_iteration(otherwise: &Expr) -> bool {
    if let Expr::Block(block) = otherwise {
        block
            .block
            .stmts
            .last()
            .is_some_and(|last| matches!(last, Stmt::Expr(Expr::Continue(_) | Expr::Break(_), _)))
    } else {
        matches!(otherwise, Expr::Continue(_) | Expr::Break(_))
    }
}

#[cfg(test)]
mod tests;
