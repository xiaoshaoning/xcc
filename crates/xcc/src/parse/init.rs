//! Initialiser lists: what `= {1, 2}` and `= "abc"` must satisfy.
//!
//! Part of the parser and nothing else: the state is `Parser`'s, which lives in `mod.rs`. The
//! file is separate because a brace list is a grammar of its own - values, nested aggregates
//! with or without their braces, and designators that move the cursor - and reading it next to
//! the expression grammar hid that.

use super::*;
use crate::types::{RecordId, RecordKind};

/// What the walker needs to know about one member: its name when it has one, its type, where it
/// sits in the aggregate, and the bits it owns of its storage unit when it is a bitfield.
type Member = (Option<Symbol>, TypeId, usize, Option<(u8, u8)>);

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
            let (literal, name, literal_bytes) = self.string_literal();
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
                    None => self.types.complete_array(ty, literal_bytes.len() as u64),
                    Some(count) if count < literal_bytes.len() as u64 => {
                        let string = literal_bytes.len() as u64 - 1;
                        self.error(
                            span,
                            format!("a {count}-byte array cannot hold this {string}-byte string and its NUL"),
                        );
                    }
                    Some(_) => {}
                }
                let Some(filled) =
                    crate::parse::bytes::string_literal_bytes(&self.types, ty, &literal_bytes)
                else {
                    return;
                };
                // One destination: a file-scope object's bytes, or a local's stores. The tail
                // of a sized array is the zeroes those bytes already hold (C99 6.7.8p21).
                if let Some(target) = self.init_target {
                    let char = self.types.fundamental("char").unwrap();
                    for (offset, byte) in filled.into_iter().enumerate() {
                        let value = self.constant_of(char, i64::from(byte), span);
                        self.store_leaf(target, offset, char, value, span);
                    }
                } else if let Some(out) = self.bytes.as_mut() {
                    let at = self.offset;
                    let end = at + filled.len();
                    // The array's size may have come from this literal, so the buffer can
                    // still be the empty one it started as.
                    if out.len() < end {
                        out.resize(end, 0);
                    }
                    out[at..end].copy_from_slice(&filled);
                }
                return;
            }
            // Anything else takes the literal as the `char *` it decays to.
            let pointer = self.types.decay(literal);
            if !self.types.is_assignable(ty, pointer) {
                let (from, to) = (self.describe(pointer), self.describe(ty));
                self.error(span, format!("cannot initialise {to} with {from}"));
                return;
            }
            // In a body the value is the address of the anonymous object the literal became
            // (ADR-004). At file scope a pointer to an object is a relocation, which the bytes
            // ADR-004 gives a global cannot express: refused, because leaving it out made the
            // pointer null rather than the literal's address.
            if self.init_target.is_none() {
                self.error(
                    span,
                    "a pointer to a string literal in a file-scope initialiser is not lowered yet",
                );
                return;
            }
            // A block that already ended writes nothing, and says nothing: dead code is not an
            // initialiser this compiler cannot lower.
            if let Some(address) = self.emit(|builder| builder.global_address(name, pointer, span))
            {
                let value = Value {
                    ty: pointer,
                    constant: None,
                    lvalue: false,
                    cir: Some(address),
                    place: None,
                    bits: None,
                };
                self.write_leaf(ty, value);
            }
            return;
        }

        // C99 6.7.8p1: an initialiser is a brace list or an assignment-expression. `{1,2,3,4}`
        // reached the list walker above, and brace *elision* happens inside a list; here what
        // follows is one expression, which is how `struct P q = p;` copies a struct.
        let value = self.conditional_expression();
        self.skip_until(&[",", ";"], false);
        if value.is_none() {
            let span = self.span();
            self.error(span, "expected an initialiser");
        }
        // A file-scope object's bytes are written from the same value the type checker sees;
        // a local's slot is written the same way.
        match value {
            Some(value) => {
                let value = self.decay(value);
                self.write_leaf(ty, value);
            }
            None => self.bytes = None,
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
        let base = self.offset;
        // Every element is a leaf (or an aggregate of them) at its own offset. An element with
        // no size - an incomplete record in an array - gets offset zero and its own diagnostic.
        let size = self.types.size_of(element).unwrap_or(0) as usize;
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
            self.offset = base + written as usize * size;
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

    /// Fill a struct or union from the values at the cursor.
    ///
    /// C99 6.7.8p9: an unnamed bitfield is padding and takes no initialiser, and a union takes
    /// one member and no more. A *named* bitfield's value goes into its bits of the unit, not over
    /// the unit: `self.bits` carries it down to the write, wherever the object lives.
    fn initialize_record(&mut self, record: RecordId, braced: bool) {
        let span = self.span();
        if !self.types.record_complete(record) {
            // No members to fill, and the object's own declaration has already said it has no
            // size: walking it would only add "too many initialisers" to that. The values are
            // skipped rather than left, so the cursor is where the enclosing list expects it.
            self.skip_until(&[",", "}"], false);
            return;
        }
        let base = self.offset;
        let union = matches!(self.types.record_kind(record), RecordKind::Union);
        let members: Vec<Member> = self
            .types
            .members(record)
            .iter()
            .map(|member| (member.name, member.ty, member.offset as usize, member.bits))
            .collect();
        let past_padding = |mut at: usize| {
            while members
                .get(at)
                .is_some_and(|(name, _, _, bits)| bits.is_some() && name.is_none())
            {
                at += 1;
            }
            at
        };
        let mut at = 0;
        let mut first = true;
        loop {
            if self.at_punct("}") || self.at_eof() {
                break;
            }
            // Padding takes no value, so step over it before asking whether there is room.
            at = past_padding(at);
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
                at = past_padding(at);
                if !self.deeper_designator() && !self.eat("=") {
                    self.error_at_current("expected `=` after a designator");
                }
            }
            let Some((_, member, offset, bits)) = members.get(at).copied() else {
                break;
            };
            self.offset = base + offset;
            self.bits = bits;
            at += 1;
            if self.at_punct("{") {
                self.brace_initializer(member);
            } else {
                self.initializer_body(member, false);
            }
            self.bits = None;
            if union {
                // One member, one value: a second is one error, not a second store over the
                // same bytes.
                if !self.at_punct("}") && !self.at_eof() {
                    self.error(span, "a union takes one initialiser");
                    self.skip_until(&["}"], false);
                }
                break;
            }
        }
    }

    /// `.name` at the cursor: which member that is, and the check that there is one.
    fn designated_member(&mut self, span: Span, members: &[Member], at: usize) -> usize {
        self.bump(); // the `.`
        match self.peek().copied() {
            Some(token) if token.kind == crate::lex::TokenKind::Ident => {
                self.bump();
                match members
                    .iter()
                    .position(|(member, _, _, _)| *member == Some(token.text))
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

    /// One value initialising one object of type `ty`: the check C asks for, and then the write,
    /// which is a store into a local's slot or bytes into a file-scope object's buffer, at the
    /// offset the walker has reached.
    ///
    /// One place, because a leaf in a brace list and a value that is the whole initialiser are
    /// the same question about the same object. The check comes first: a value of the wrong
    /// type is one diagnostic, not a failed conversion and a missing store on top of it. A
    /// partly-filled object is a wrong program, not a smaller one, so anything unwritable
    /// clears everything (ADR-004).
    fn write_leaf(&mut self, ty: TypeId, value: Value) {
        if !self.types.is_error(ty) && !self.types.is_error(value.ty) {
            let null_pointer = self.null_pointer_constant(&value);
            if !null_pointer && !self.types.is_assignable(ty, value.ty) {
                let (from, to) = (self.describe(value.ty), self.describe(ty));
                let span = self.span();
                self.error(span, format!("cannot initialise {to} with {from}"));
                return;
            }
        }
        if let Some(target) = self.init_target {
            let (offset, span) = (self.offset, self.span());
            self.store_leaf(target, offset, ty, value, span);
            return;
        }
        if self.bytes.is_none() {
            return;
        }
        let at = self.offset;
        if value.constant.is_none() || value.cir.is_some() {
            self.bytes = None;
            return;
        }
        let written = crate::parse::bytes::constant_bytes(&self.types, ty, value);
        match (self.bytes.as_mut(), written) {
            (Some(out), Some(bytes)) if at + bytes.len() <= out.len() => match self.bits {
                None => out[at..at + bytes.len()].copy_from_slice(&bytes),
                // A global's bytes start as zeroes, but two bitfields share one unit: the
                // second merges into what the first left there.
                Some((start, width)) => {
                    crate::parse::bytes::merge_bits(out, at, &bytes, start, width);
                }
            },
            _ => self.bytes = None,
        }
    }

    /// One value in a brace list, checked against what it initialises.
    fn initializer_value(&mut self, ty: TypeId) {
        let Some(value) = self.assignment_expression().map(|value| self.decay(value)) else {
            return;
        };
        self.write_leaf(ty, value);
    }

    /// A local's initialiser: the leaves become stores through its slot, and what the list
    /// does not name is zero (C99 6.7.8p21).
    pub(super) fn initialize_local(&mut self, ty: TypeId, target: Option<(cir::Value, TypeId)>) {
        let Some(target) = target else {
            self.initializer(ty);
            return;
        };
        // A declared size can have elements or members no list reaches, and C says they are
        // zero. An array whose size came from its initialiser has none, and a string literal
        // writes every byte including the tail.
        if self.types.value_is_address(ty) && self.types.is_complete(ty) {
            self.zero_object(target, ty);
        }
        let saved_target = self.init_target;
        let saved_offset = self.offset;
        let saved_bits = self.bits;
        self.init_target = Some(target);
        self.offset = 0;
        self.bits = None;
        self.initializer(ty);
        self.init_target = saved_target;
        self.offset = saved_offset;
        self.bits = saved_bits;
    }

    /// C99 6.7.8p21: what an initialiser list does not name is zero, and a frame slot holds
    /// whatever the frame held. A byte at a time: a byte store is always aligned, and a
    /// wider one is M10's optimiser's job.
    fn zero_object(&mut self, target: (cir::Value, TypeId), ty: TypeId) {
        let Some(size) = self.types.size_of(ty) else {
            return;
        };
        let char = self.types.fundamental("char").unwrap();
        let span = self.span();
        for offset in 0..size as usize {
            let zero = self.constant_of(char, 0, span);
            self.store_leaf(target, offset, char, zero, span);
        }
    }

    /// One leaf's store into the object being initialised: the value converted to the leaf's
    /// type, at `offset` bytes from the object's address - or, for a bitfield `self.bits` names,
    /// merged into the bits it owns of the storage unit there.
    fn store_leaf(
        &mut self,
        target: (cir::Value, TypeId),
        offset: usize,
        ty: TypeId,
        value: Value,
        span: Span,
    ) {
        let (base, object) = target;
        let Some(address) = self.address_within(base, object, offset, ty, span) else {
            return;
        };
        let before = self.error_count();
        let converted = self.convert(value, ty, span);
        if converted.cir.is_none() {
            // A value with neither code nor a constant would store nothing and leave the slot
            // holding the frame's bytes. A conversion that already said why is one diagnostic,
            // and this does not add a second.
            if converted.constant.is_none()
                && self.error_count() == before
                && !self.types.is_error(ty)
            {
                self.not_lowered(span, "this initialiser value");
            }
            return;
        }
        match self.bits {
            None => self.store_at(address, ty, converted, span),
            // A bitfield shares its unit with its neighbours, so its value is merged into their
            // bytes rather than written over them.
            Some(bits) => {
                self.write_bitfield_at(address, ty, converted, bits, span);
            }
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
