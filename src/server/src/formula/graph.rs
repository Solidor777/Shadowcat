//! Memoized lazy resolution over named nodes. Twin of the client package's
//! `graph.ts`. Dependencies are discovered dynamically: `eval_node` calls the
//! injected `get` for each dependency, and a cycle is detected on the active
//! path. INVARIANT: the result is a pure function of the key SET — roots
//! iterate in sorted order so the traversal (and therefore which member of a
//! short-circuitable cycle reports `Cycle`) never depends on caller order.
//! Stack bound: the driver runs on an explicit heap `stack`, never JS/Rust
//! recursion. The first time `get` meets a key that is neither memoized nor
//! on the active path it records the key as `needed` and returns a placeholder
//! error; the driver discards that `eval_node` result, resolves the dependency,
//! and re-invokes `eval_node` for the same key from scratch. Memoized
//! dependencies make each retry a series of O(1) lookups. `MAX_GRAPH_VISITS`
//! is charged once per key at first attempt.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{BTreeMap, HashSet};

use super::types::{finite, FormulaError, FormulaErrorKind, FormulaValue, MAX_GRAPH_VISITS};

/// Resolves every key in `keys` plus every transitive dependency discovered.
/// A dependency a node short-circuits past is never requested and never
/// appears in the map. Never panics.
pub fn resolve_all<F>(keys: &[String], mut eval_node: F) -> BTreeMap<String, FormulaValue>
where
    F: FnMut(&str, &mut dyn FnMut(&str) -> FormulaValue) -> FormulaValue,
{
    let mut memo: BTreeMap<String, FormulaValue> = BTreeMap::new();
    let mut visiting: HashSet<String> = HashSet::new();
    // The active dependency path (a linear chain: each entry depends on the next).
    let mut stack: Vec<String> = Vec::new();
    let mut visits = 0usize;

    let mut roots: Vec<&String> = keys.iter().collect();
    roots.sort();
    roots.dedup();

    for root in roots {
        if memo.contains_key(root) {
            continue;
        }
        stack.clear();
        stack.push(root.clone());
        while let Some(key) = stack.last().cloned() {
            if memo.contains_key(&key) {
                stack.pop();
                continue;
            }
            if !visiting.contains(&key) {
                visiting.insert(key.clone());
                visits += 1;
                if visits > MAX_GRAPH_VISITS {
                    memo.insert(
                        key.clone(),
                        Err(FormulaError::new(
                            FormulaErrorKind::Cap,
                            "graph resolution exceeded visit cap",
                        )),
                    );
                    visiting.remove(&key);
                    stack.pop();
                    continue;
                }
            }
            let mut needed: Option<String> = None;
            let result = {
                let memo_ref = &memo;
                let visiting_ref = &visiting;
                let stack_ref = &stack;
                let needed_ref = &mut needed;
                let key_ref = &key;
                let mut get = |dep: &str| -> FormulaValue {
                    if let Some(v) = memo_ref.get(dep) {
                        return v.clone();
                    }
                    if visiting_ref.contains(dep) {
                        // Re-entering a key on the active path closes a cycle;
                        // the path slice from that key to the top IS the cycle.
                        // Name its lexicographically smallest member so the
                        // detail is canonical regardless of traversal.
                        let start = stack_ref.iter().position(|k| k == dep);
                        return match start {
                            Some(s) => {
                                let canonical =
                                    stack_ref[s..].iter().min().cloned().unwrap_or_default();
                                Err(FormulaError::new(
                                    FormulaErrorKind::Cycle,
                                    format!("reference cycle involving '{canonical}'"),
                                ))
                            }
                            // Every visiting key is on the stack (add/push and
                            // remove/pop are paired); a miss means that pairing
                            // broke. Surface it as a value, never a panic — with
                            // the wording the client's outer catch produces for
                            // the node under evaluation.
                            None => Err(FormulaError::new(
                                FormulaErrorKind::ResolverError,
                                format!("evalNode threw for '{key_ref}'"),
                            )),
                        };
                    }
                    if needed_ref.is_none() {
                        *needed_ref = Some(dep.to_string());
                    }
                    // Restart placeholder, never observable: the driver discards
                    // the whole `eval_node` result once `needed` is set. Tagged
                    // `RefError` because that kind has no producer of its own on
                    // either side (see its doc); nothing downstream can mistake
                    // a leaked one for a real failure because none can leak.
                    Err(FormulaError::new(
                        FormulaErrorKind::RefError,
                        format!("unresolved dependency '{dep}'"),
                    ))
                };
                eval_node(&key, &mut get)
            };
            if let Some(dep) = needed {
                // Leave `key` on the stack (and in `visiting`); retry it once
                // `dep` is memoized.
                stack.push(dep);
                continue;
            }
            memo.insert(key.clone(), result.and_then(finite));
            visiting.remove(&key);
            stack.pop();
        }
    }
    memo
}

#[cfg(test)]
mod tests;
