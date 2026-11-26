use std::collections::BTreeMap;

use syn::visit::{self, Visit};
use syn::{ImplItem, Item, ItemImpl, ItemMod, Type, Visibility};

use super::Refusal;
use super::hygiene::refuse;
use super::parsed::{Found, offenses};

pub(super) fn constants_come_before_any_other_code() -> Option<Refusal> {
    refuse(
        &offenses(|source| {
            let mut modules = Modules {
                found: Found::new(&source.path),
                check: constants_after_code,
            };
            modules.visit_file(&source.file);
            modules.found.lines
        }),
        "constants and statics come after other code: move each to the top of its module, after \
         the imports",
    )
}

pub(super) fn an_impl_starts_with_new_then_associated_functions_then_public_then_private_methods() -> Option<Refusal> {
    refuse(
        &offenses(|source| {
            let mut impls = Impls {
                found: Found::new(&source.path),
            };
            impls.visit_file(&source.file);
            impls.found.lines
        }),
        "functions in an inherent `impl` are out of order: `new` first, then the other public \
         associated functions, then public methods, then private ones",
    )
}

pub(super) fn trait_impls_come_after_the_inherent_impl() -> Option<Refusal> {
    refuse(
        &offenses(|source| {
            let mut modules = Modules {
                found: Found::new(&source.path),
                check: trait_impls_before_the_inherent_one,
            };
            modules.visit_file(&source.file);
            modules.found.lines
        }),
        "trait impls come before their type's inherent `impl`: move them after it",
    )
}

struct Modules<'a> {
    found: Found<'a>,
    check: fn(&[Item], &mut Found),
}

impl<'ast> Visit<'ast> for Modules<'_> {
    fn visit_file(&mut self, file: &'ast syn::File) {
        (self.check)(&file.items, &mut self.found);
        visit::visit_file(self, file);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if let Some((_, items)) = &module.content {
            (self.check)(items, &mut self.found);
        }
        visit::visit_item_mod(self, module);
    }
}

struct Impls<'a> {
    found: Found<'a>,
}

impl<'ast> Visit<'ast> for Impls<'_> {
    fn visit_item_impl(&mut self, implemented: &'ast ItemImpl) {
        if implemented.trait_.is_none() {
            let mut reached = 0;
            for item in &implemented.items {
                if let ImplItem::Fn(function) = item {
                    let rank = rank_of(function);
                    if rank < reached {
                        self.found
                            .at(&function.sig.ident, format!("{} out of order", function.sig.ident));
                    } else {
                        reached = rank;
                    }
                }
            }
        }
        visit::visit_item_impl(self, implemented);
    }
}

fn constants_after_code(items: &[Item], found: &mut Found) {
    let mut code = false;
    for item in items {
        match item {
            Item::Use(_) | Item::ExternCrate(_) | Item::Macro(_) | Item::Verbatim(_) => {}
            Item::Mod(module) if module.content.is_none() => {}
            Item::Const(constant) => {
                if code {
                    found.at(&constant.ident, format!("{} after code", constant.ident));
                }
            }
            Item::Static(constant) => {
                if code {
                    found.at(&constant.ident, format!("{} after code", constant.ident));
                }
            }
            _ => code = true,
        }
    }
}

fn trait_impls_before_the_inherent_one(items: &[Item], found: &mut Found) {
    let mut inherent = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        if let Item::Impl(implemented) = item
            && implemented.trait_.is_none()
            && let Some(name) = type_name(implemented)
        {
            inherent.entry(name).or_insert(index);
        }
    }
    for (index, item) in items.iter().enumerate() {
        if let Item::Impl(implemented) = item
            && implemented.trait_.is_some()
            && let Some(name) = type_name(implemented)
            && inherent.get(&name).is_some_and(|at| index < *at)
        {
            found.at(
                &implemented.impl_token,
                format!("{name}'s trait impl before its inherent impl"),
            );
        }
    }
}

fn rank_of(function: &syn::ImplItemFn) -> u8 {
    let associated = function.sig.receiver().is_none();
    let public = !matches!(function.vis, Visibility::Inherited);
    if associated && function.sig.ident == "new" {
        0
    } else if associated && public {
        1
    } else if public {
        2
    } else {
        3
    }
}

fn type_name(implemented: &ItemImpl) -> Option<String> {
    if let Type::Path(path) = &*implemented.self_ty {
        path.path.segments.last().map(|segment| segment.ident.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
