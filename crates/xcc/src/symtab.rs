//! Scopes and the names in them.
//!
//! One table per scope, pushed and popped as the parser enters and leaves blocks, with the file
//! scope at the bottom. Lookup walks inwards, so shadowing is what the stack does rather than
//! something each caller has to remember.
//!
//! What a name can be is a small closed set for now: a variable, a function, a `typedef`, or an
//! enumeration constant. Each is one variant of [`Kind`], so a caller that only wants typedefs
//! asks for typedefs and does not have to test three other things.

use std::collections::HashMap;

use crate::diag::Span;
use crate::intern::Symbol;
use crate::types::TypeId;

/// What a name means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind {
    /// An object: a variable, a parameter, an array.
    Variable,
    /// A function.
    Function,
    /// A type name.
    Typedef,
    /// An enumeration constant, with its value.
    EnumConst(i64),
}

/// One declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// The type the name was declared with.
    pub ty: TypeId,
    /// What kind of thing it is.
    pub kind: Kind,
    /// Where it was declared, for the diagnostic about a redeclaration.
    pub span: Span,
    /// The address of the variable, when it has one in the function being built: a slot pointer
    /// from `cir`. Declaration and storage are one fact, so they are recorded together.
    pub storage: Option<crate::cir::Value>,
}

/// One scope: the names declared in it, and the `struct`/`union` tags, which live in a
/// namespace of their own because C lets a tag and a variable share a name.
#[derive(Debug, Default)]
struct Scope {
    names: HashMap<Symbol, Entry>,
    tags: HashMap<Symbol, TypeId>,
}

/// The scopes currently open, outermost first.
#[derive(Debug)]
pub(crate) struct Symbols {
    scopes: Vec<Scope>,
}

impl Symbols {
    /// One file scope, and nothing else.
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::default()],
        }
    }

    /// Enter a block.
    pub fn push(&mut self) {
        self.scopes.push(Scope::default());
    }

    /// Leave a block, dropping its names and tags.
    pub fn pop(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn innermost(&mut self) -> &mut Scope {
        self.scopes.last_mut().expect("a file scope always exists")
    }

    /// Declare a struct or union tag in the innermost scope.
    pub fn declare_tag(&mut self, name: Symbol, ty: TypeId) {
        self.innermost().tags.insert(name, ty);
    }

    /// The type a tag names, if this scope or an enclosing one has it.
    pub fn tag(&self, name: Symbol) -> Option<TypeId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.tags.get(&name).copied())
    }

    /// The tag declared in the innermost scope only.
    pub fn tag_here(&self, name: Symbol) -> Option<TypeId> {
        self.scopes
            .last()
            .and_then(|scope| scope.tags.get(&name).copied())
    }

    /// Declare `name` in the innermost scope.
    ///
    /// Returns the declaration already there when the name is taken *in this scope*: shadowing
    /// an outer scope is legal C, redeclaring in the same one is not (for the kinds that exist
    /// so far).
    pub fn declare(&mut self, name: Symbol, entry: Entry) -> Result<(), Entry> {
        let scope = self.innermost();
        if let Some(previous) = scope.names.get(&name) {
            // A `typedef` and a variable of the same name are one mistake, not two.
            return Err(previous.clone());
        }
        scope.names.insert(name, entry);
        Ok(())
    }

    /// The declaration of `name` in the innermost scope, to be updated once its storage exists.
    pub fn here_mut(&mut self, name: Symbol) -> Option<&mut Entry> {
        self.innermost().names.get_mut(&name)
    }

    /// The declaration of `name` in the innermost scope, if this scope has one: what a
    /// redeclaration has to be compared with.
    pub fn here(&self, name: Symbol) -> Option<&Entry> {
        self.scopes.last().and_then(|scope| scope.names.get(&name))
    }

    /// The innermost declaration of `name`.
    pub fn lookup(&self, name: Symbol) -> Option<&Entry> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.names.get(&name))
    }

    /// The type `name` was declared as a `typedef`, if it was.
    pub fn typedef(&self, name: Symbol) -> Option<TypeId> {
        match self.lookup(name) {
            Some(entry) if matches!(entry.kind, Kind::Typedef) => Some(entry.ty),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intern::Interner;
    use crate::types::Types;

    fn entry(ty: TypeId, kind: Kind) -> Entry {
        Entry {
            ty,
            kind,
            span: Span::default(),
            storage: None,
        }
    }

    #[test]
    fn a_name_is_found_in_the_scope_it_was_declared_in() {
        let mut interner = Interner::new();
        let types = Types::new();
        let int = types.fundamental("int").unwrap();
        let mut symbols = Symbols::new();
        let x = interner.intern("x");

        assert!(symbols.declare(x, entry(int, Kind::Variable)).is_ok());
        assert_eq!(symbols.lookup(x).map(|found| found.ty), Some(int));
        symbols.push();
        assert_eq!(
            symbols.lookup(x).map(|found| found.ty),
            Some(int),
            "outer scopes stay visible"
        );
        symbols.pop();
    }

    #[test]
    fn shadowing_is_allowed_and_redeclaring_is_not() {
        let mut interner = Interner::new();
        let types = Types::new();
        let int = types.fundamental("int").unwrap();
        let long = types.fundamental("long long").unwrap();
        let mut symbols = Symbols::new();
        let x = interner.intern("x");

        symbols.declare(x, entry(int, Kind::Variable)).unwrap();
        assert!(
            symbols.declare(x, entry(int, Kind::Variable)).is_err(),
            "twice in one scope"
        );

        symbols.push();
        symbols.declare(x, entry(long, Kind::Variable)).unwrap();
        assert_eq!(
            symbols.lookup(x).map(|found| found.ty),
            Some(long),
            "the inner one wins"
        );
        symbols.pop();
        assert_eq!(symbols.lookup(x).map(|found| found.ty), Some(int));
    }

    #[test]
    fn a_typedef_is_asked_for_by_kind() {
        let mut interner = Interner::new();
        let types = Types::new();
        let int = types.fundamental("int").unwrap();
        let mut symbols = Symbols::new();
        let t = interner.intern("T");
        let v = interner.intern("v");

        symbols.declare(t, entry(int, Kind::Typedef)).unwrap();
        symbols.declare(v, entry(int, Kind::Variable)).unwrap();
        assert_eq!(symbols.typedef(t), Some(int));
        assert_eq!(symbols.typedef(v), None);
    }
}
