//! Names, interned once.
//!
//! A token's spelling is a [`Symbol`], not a slice of the source: the same name written twice
//! is the same symbol, comparing names is comparing integers, and a map keyed by a symbol
//! cannot be perturbed by bytes that only look alike.
//!
//! Ids are handed out in first-seen order, so they depend on the source text and on nothing
//! else: no addresses, no hash order, no memory of a previous compilation (CONSTRAINTS 4).
//! The map exists for lookup only - nothing iterates it, and a `Symbol` is only meaningful
//! together with the interner that minted it.

use std::collections::HashMap;

/// A name, interned. Only an [`Interner`] makes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(u32);

/// The names seen so far, in the order they were first seen.
#[derive(Debug, Clone, Default)]
pub struct Interner {
    names: Vec<Box<str>>,
    index: HashMap<Box<str>, Symbol>,
}

impl Interner {
    /// An interner that knows no names.
    pub fn new() -> Self {
        Self::default()
    }

    /// The symbol for `name`, minting one if this is the first time it has been seen.
    pub fn intern(&mut self, name: &str) -> Symbol {
        if let Some(&symbol) = self.index.get(name) {
            return symbol;
        }
        let symbol = Symbol(self.names.len() as u32);
        self.names.push(name.into());
        self.index.insert(name.into(), symbol);
        symbol
    }

    /// The symbol for `name` if it has been seen.
    pub fn get(&self, name: &str) -> Option<Symbol> {
        self.index.get(name).copied()
    }

    /// The name a symbol stands for.
    ///
    /// # Panics
    ///
    /// Panics for a symbol this interner did not mint: a symbol from another interner is a bug
    /// in the compiler, not in its input (CONSTRAINTS 6).
    pub fn resolve(&self, symbol: Symbol) -> &str {
        &self.names[symbol.0 as usize]
    }

    /// How many names are known.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether no names are known.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_handed_out_in_first_seen_order() {
        let mut interner = Interner::new();
        assert_eq!(interner.intern("b"), Symbol(0));
        assert_eq!(interner.intern("a"), Symbol(1));
        assert_eq!(
            interner.intern("b"),
            Symbol(0),
            "the same name, the same symbol"
        );
        assert_eq!(interner.len(), 2);
        assert_eq!(interner.resolve(Symbol(1)), "a");
    }

    #[test]
    fn a_fresh_interner_gives_the_same_ids() {
        let ids = |names: &[&str]| {
            let mut interner = Interner::new();
            names.iter().map(|n| interner.intern(n)).collect::<Vec<_>>()
        };
        assert_eq!(ids(&["x", "y", "x"]), ids(&["x", "y", "x"]));
    }

    #[test]
    fn lookup_does_not_mint() {
        let mut interner = Interner::new();
        interner.intern("x");
        assert_eq!(interner.get("x"), Some(Symbol(0)));
        assert_eq!(interner.get("y"), None);
        assert_eq!(interner.len(), 1);
    }
}
