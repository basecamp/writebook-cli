use syn::visit::{self, Visit};
use syn::{Expr, ExprMethodCall, ExprTry};

use super::Refusal;
use super::hygiene::refuse;
use super::parsed::{Found, offenses};

const GUARDS: [&str; 3] = ["lock", "read", "write"];

const POISON_HANDLERS: [&str; 5] = ["expect", "unwrap", "map_err", "ok", "unwrap_or_default"];

pub(super) fn a_poisoned_lock_is_taken_back() -> Option<Refusal> {
    refuse(
        &offenses(|source| {
            let mut locks = Locks {
                found: Found::new(&source.path),
            };
            for item in source.production() {
                locks.visit_item(&item);
            }
            locks.found.lines
        }),
        "lines take a std lock and fail or drop on its poison: take it with \
         `lock().unwrap_or_else(PoisonError::into_inner)`",
    )
}

struct Locks<'a> {
    found: Found<'a>,
}

impl<'ast> Visit<'ast> for Locks<'_> {
    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if POISON_HANDLERS.contains(&call.method.to_string().as_str())
            && let Some(guard) = takes_a_guard(&call.receiver)
        {
            self.found.at(&call.method, format!(".{guard}().{}(..)", call.method));
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_try(&mut self, tried: &'ast ExprTry) {
        if let Some(guard) = takes_a_guard(&tried.expr) {
            self.found.at(&tried.question_token, format!(".{guard}()?"));
        }
        visit::visit_expr_try(self, tried);
    }
}

fn takes_a_guard(expression: &Expr) -> Option<String> {
    if let Expr::MethodCall(call) = expression
        && call.args.is_empty()
        && GUARDS.contains(&call.method.to_string().as_str())
    {
        Some(call.method.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
