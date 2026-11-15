//! Initialiser lists: what `= {1, 2}` and `= "abc"` must satisfy.
//!
//! Part of the parser and nothing else: the state is `Parser`'s, which lives in `mod.rs`. The
//! file is separate because a brace list is a grammar of its own - values, nested aggregates
//! with or without their braces, and designators that move the cursor - and reading it next to
//! the expression grammar hid that.

use super::*;
use crate::types::RecordId;

impl Parser<'_> {
    /// `= <initialiser>`: one value for a scalar, a brace list for an array or aggregate.
    pub(super) fn initializer(&mut self, ty: TypeId) {
        if self.at_punct("{") {
            self.brace_initializer(ty);
            return;
        }
        let Some(token) = self.peek().copied() else {
            return;
        };
        if token.kind == crate::lex::TokenKind::Str {
            // A string literal gives a `char` array its length, NUL included (C99 6.7.8p14).
            let span = self.span();
            let (literal, bytes) = self.string_literal();
            if let Type::Array { element, count } = *self.types.kind(ty) {
                // C99 6.7.8p14: an array of character type, which `int` is not even though a
                // `char` converts to one.
                if !matches!(self.types.kind(element), Type::Int { bits: 8, .. }) {
                    let element = self.describe(element);
                    self.error(
                        span,
                        format!("a string literal initialises an array of `char`, not {element}"),
                    );
                    return;
                }
                match count {
                    None => self.types.complete_array(ty, bytes + 1),
                    Some(count) if count < bytes + 1 => {
                        self.error(
                            span,
                            format!("a {count}-byte array cannot hold this {bytes}-byte string and its NUL"),
                        );
                    }
                    Some(_) => {}
                }
                return;
            }
            // Anything else takes the literal as the `char *` it decays to.
            let value = self.decay(Value {
                ty: literal,
                constant: None,
                lvalue: true,
            });
            if !self.types.is_assignable(ty, value.ty) {
                let (from, to) = (self.describe(value.ty), self.describe(ty));
                self.error(span, format!("cannot initialise {to} with {from}"));
            }
            return;
        }

        // (C99 6.7.8p20): `int a[2][2] = {1,2,3,4}`.
        if !self.at_punct("{")
            && (self.types.record_of(ty).is_some()
                || matches!(self.types.kind(ty), Type::Array { .. }))
        {
            self.initializer_body(ty, false);
            return;
        }
        let value = self.conditional_expression();
        self.skip_until(&[",", ";"], false);
        if value.is_none() {
            let span = self.span();
            self.error(span, "expected an initialiser");
        }
    }

    /// A brace list: `{1, 2}`, `{{1,2},{3,4}}`, and `{1,2,3,4}` for a `int[2][2]` - C lets the
    /// braces of a nested aggregate be left out (C99 6.7.8p20), and so does this.
    pub(super) fn brace_initializer(&mut self, ty: TypeId) {
        self.bump(); // the `{`
        self.initializer_body(ty, true);
        self.eat(","); // a trailing comma before the closing brace is allowed
        if !self.eat("}") {
            self.error_at_current("expected `}` to end the initialiser");
        }
    }

    /// Fill an aggregate from the values at the cursor, stopping where its members end or at a
    /// `}` that closes an enclosing list.
    ///
    /// `braced` says whether this list has braces of its own: only then is a surplus value this
    /// aggregate's problem. A flattened one - `int a[2][3] = {1,2,3,4,5,6}` - hands the values
    /// after its capacity to the list around it (C99 6.7.8p20). A designator (`[3] = `, `.x = `)
    /// moves the cursor, and the list carries on from after the element it named (6.7.8p17).
    fn initializer_body(&mut self, ty: TypeId, braced: bool) {
        match self.types.kind(ty) {
            Type::Array { element, count } => {
                let (element, declared) = (*element, *count);
                self.initialize_array(ty, element, declared, braced);
            }
            Type::Record(record) => {
                let record = *record;
                self.initialize_record(record, braced);
            }
            // A scalar is one value. It does not take the comma after it: the list around it
            // owns the separators.
            _ => self.initializer_value(ty),
        }
    }

    fn initialize_array(
        &mut self,
        ty: TypeId,
        element: TypeId,
        declared: Option<u64>,
        braced: bool,
    ) {
        let span = self.span();
        let mut written = 0u64;
        let mut first = true;
        loop {
            if self.at_punct("}") || self.at_eof() {
                break;
            }
            // Capacity before the separator, so a nested list leaves the comma that belongs to
            // the list around it; a designator may come after the array is full, so it is
            // exempt.
            if !self.designator_follows() && declared.is_some_and(|declared| written >= declared) {
                break;
            }
            if !first && !self.eat(",") {
                break;
            }
            first = false;
            if self.at_punct("[") {
                written = self.designated_index(span, declared, written);
                if self.at_punct("...") {
                    self.not_implemented(
                        span,
                        "the GNU `[a ... b]` initialiser range is not implemented yet",
                    );
                    self.skip_until(&[",", "}"], false);
                    continue;
                }
                if !self.eat("]") {
                    self.error_at_current("expected `]` after a designator");
                }
                if !self.deeper_designator() && !self.eat("=") {
                    self.error_at_current("expected `=` after a designator");
                }
            }
            if self.at_punct("{") {
                self.brace_initializer(element);
            } else {
                self.initializer_body(element, false);
            }
            written += 1;
        }
        match declared {
            None => self.types.complete_array(ty, written),
            // The list stopped at the last element it had room for; a comma here means more
            // came, which is one error and then the rest is skipped.
            Some(declared) if braced && written >= declared && self.at_punct(",") => {
                self.error(
                    span,
                    format!("this array has {declared} elements, more were given"),
                );
                self.skip_until(&["}"], false);
            }
            Some(_) => {}
        }
    }

    /// `[k]` at the cursor: the index it names, and the check that the array has one.
    fn designated_index(&mut self, span: Span, declared: Option<u64>, written: u64) -> u64 {
        self.bump(); // the `[`
        match (self.constant_expression(), declared) {
            (Some(index), _) if index < 0 => {
                self.error(span, format!("`[{index}]` is not a valid index"));
                written
            }
            (Some(index), Some(declared)) if index as u64 >= declared => {
                self.error(
                    span,
                    format!("this array has {declared} elements, so `[{index}]` is past the end"),
                );
                index as u64
            }
            (Some(index), _) => index as u64,
            (None, _) => written,
        }
    }

    fn initialize_record(&mut self, record: RecordId, braced: bool) {
        let span = self.span();
        let members: Vec<(Option<Symbol>, TypeId)> = self
            .types
            .members(record)
            .iter()
            .map(|member| (member.name, member.ty))
            .collect();
        let mut at = 0;
        let mut first = true;
        loop {
            if self.at_punct("}") || self.at_eof() {
                break;
            }
            if !self.designator_follows() && at >= members.len() {
                if braced {
                    self.error(span, "too many initialisers for this struct or union");
                    self.skip_until(&["}"], false);
                }
                break;
            }
            if !first && !self.eat(",") {
                break;
            }
            first = false;
            if self.at_punct(".") {
                at = self.designated_member(span, &members, at);
                if !self.deeper_designator() && !self.eat("=") {
                    self.error_at_current("expected `=` after a designator");
                }
            }
            let Some((_, member)) = members.get(at).copied() else {
                break;
            };
            at += 1;
            if self.at_punct("{") {
                self.brace_initializer(member);
            } else {
                self.initializer_body(member, false);
            }
        }
    }

    /// `.name` at the cursor: which member that is, and the check that there is one.
    fn designated_member(
        &mut self,
        span: Span,
        members: &[(Option<Symbol>, TypeId)],
        at: usize,
    ) -> usize {
        self.bump(); // the `.`
        match self.peek().copied() {
            Some(token) if token.kind == crate::lex::TokenKind::Ident => {
                self.bump();
                match members
                    .iter()
                    .position(|(member, _)| *member == Some(token.text))
                {
                    Some(position) => position,
                    None => {
                        let name = self.spell_of(token.text).to_owned();
                        self.error(span, format!("this has no member `{name}`"));
                        at
                    }
                }
            }
            _ => {
                self.error_at_current("expected a member name");
                at
            }
        }
    }

    /// One value in a brace list, checked against what it initialises.
    fn initializer_value(&mut self, ty: TypeId) {
        let Some(value) = self.assignment_expression().map(|value| self.decay(value)) else {
            return;
        };
        if self.types.is_error(ty) || self.types.is_error(value.ty) {
            return;
        }
        let null_pointer = self.types.is_pointer(ty) && value.constant == Some(0);
        if !null_pointer && !self.types.is_assignable(ty, value.ty) {
            let (from, to) = (self.describe(value.ty), self.describe(ty));
            let span = self.span();
            self.error(span, format!("cannot initialize {to} with {from}"));
        }
    }

    /// Whether a designator comes next, now or after the comma - the reason the capacity check
    /// cannot simply end the list: `{.y = 2, .x = 1}` goes back up to a full aggregate.
    fn designator_follows(&self) -> bool {
        let at = |ahead: usize, punctuation: &str| {
            self.peek_at(ahead)
                .is_some_and(|token| self.is_punct(token, punctuation))
        };
        at(0, ".") || at(0, "[") || (at(0, ",") && (at(1, ".") || at(1, "[")))
    }

    /// A designator that names a sub-object of the element it just picked: `[1].x`, `.p.y`.
    /// Refused rather than half-read, because carrying the cursor back out through the chain
    /// is not written yet.
    fn deeper_designator(&mut self) -> bool {
        if !(self.at_punct(".") || self.at_punct("[")) {
            return false;
        }
        let span = self.span();
        self.not_implemented(
            span,
            "a designator that goes deeper than one level is not implemented yet",
        );
        self.skip_until(&[",", "}"], false);
        true
    }
}
