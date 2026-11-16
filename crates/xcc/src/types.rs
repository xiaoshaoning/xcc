//! C types: one arena, one id per type.
//!
//! A [`TypeId`] is a small integer, so types are cheap to copy, compare and store in a symbol
//! table, and the arena is the only owner. Two types are the same when their *structure* is
//! the same: there is no name-based identity, so a `typedef` and the type it names are one
//! type, which is what C means and what stops a typedef from becoming a special case
//! everywhere.
//!
//! Sizes and alignments are the Win64 ones, which is the only ABI this compiler has: `int` is
//! 32 bits, `long` is 32, `long long` is 64, pointers are 8, and `long double` is 64 because the
//! back end emits SSE (ADR-001, decision 3).

use std::collections::HashMap;

use crate::intern::Symbol;

/// A type, by index into [`Types`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct TypeId(u32);

/// What a type is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Type {
    /// No value: `void`, and the return type of a function that returns nothing.
    Void,
    /// `_Bool`: 1 byte, and every conversion to it gives 0 or 1.
    Bool,
    /// A signed or unsigned integer of 8, 16, 32 or 64 bits.
    Int {
        /// Whether the type is signed.
        signed: bool,
        /// How many bits it has: 8, 16, 32 or 64.
        bits: u8,
    },
    /// An IEEE-754 binary floating type of 32 or 64 bits.
    Float {
        /// 32 for `float`, 64 for `double` and `long double`.
        bits: u8,
    },
    /// A pointer to another type.
    Pointer(TypeId),
    /// An array of `element`. `count` is `None` while the array is still incomplete.
    Array {
        /// What the elements are.
        element: TypeId,
        /// How many there are, once known.
        count: Option<u64>,
    },
    /// A function type: what it returns and what it takes.
    Function {
        /// The return type.
        result: TypeId,
        /// The parameter types, in order; `(void)` is an empty list.
        params: Vec<TypeId>,
        /// Whether the parameter list ended in `...`.
        variadic: bool,
    },
    /// A struct or a union, by index into [`Types`]'s records.
    Record(RecordId),
    /// A type that could not be determined. Everything it touches is this, so one mistake in a
    /// declaration is one diagnostic rather than a cascade of follow-on errors.
    Error,
}

/// A struct or union, by index into the records the arena holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RecordId(u32);

/// Which of the two an aggregate is: members side by side, or members on top of each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordKind {
    /// `struct`: members are laid out one after another.
    Struct,
    /// `union`: members are laid out at offset zero, and the size is the largest.
    Union,
}

/// One member of a struct or union.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Member {
    /// Its name, absent for an anonymous bitfield used only as padding.
    pub name: Option<Symbol>,
    /// Its type.
    pub ty: TypeId,
    /// Where it starts, in bytes from the start of the aggregate.
    pub offset: u64,
    /// For a bitfield: the bit it starts at within its unit, and how many bits it has.
    pub bits: Option<(u8, u8)>,
}

/// A struct or union: its members once it is complete, and its layout.
#[derive(Debug, Clone)]
struct Record {
    kind: RecordKind,
    tag: Option<Symbol>,
    members: Vec<Member>,
    size: u64,
    align: u64,
    complete: bool,
}

/// The type arena.
#[derive(Debug)]
pub(crate) struct Types {
    nodes: Vec<Type>,
    /// The fundamental types, made once so that `int` is always the same id.
    fundamentals: HashMap<&'static str, TypeId>,
    /// The one error type, shared by every malformed declaration.
    error: TypeId,
    /// The structs and unions, complete or not.
    records: Vec<Record>,
}

// `align_of`, `is_pointer` and `pointee` have no caller *yet*: they belong to the layout work
// and to M3.3's structs and function pointers. Everything else here earns its keep in the
// parser now (`same`, `describe`, `usual_arithmetic`, `is_assignable`, `decay`, `is_error`).
// The attribute narrows to those three with the first caller.
#[allow(dead_code)]
impl Types {
    /// An arena holding the fundamental types.
    pub fn new() -> Self {
        let mut types = Self {
            nodes: Vec::new(),
            fundamentals: HashMap::new(),
            error: TypeId(0),
            records: Vec::new(),
        };
        types.error = types.push(Type::Error);
        for (name, ty) in [
            ("void", Type::Void),
            ("_Bool", Type::Bool),
            (
                "char",
                Type::Int {
                    signed: true,
                    bits: 8,
                },
            ),
            (
                "signed char",
                Type::Int {
                    signed: true,
                    bits: 8,
                },
            ),
            (
                "unsigned char",
                Type::Int {
                    signed: false,
                    bits: 8,
                },
            ),
            (
                "short",
                Type::Int {
                    signed: true,
                    bits: 16,
                },
            ),
            (
                "unsigned short",
                Type::Int {
                    signed: false,
                    bits: 16,
                },
            ),
            (
                "int",
                Type::Int {
                    signed: true,
                    bits: 32,
                },
            ),
            (
                "unsigned int",
                Type::Int {
                    signed: false,
                    bits: 32,
                },
            ),
            (
                "long",
                Type::Int {
                    signed: true,
                    bits: 32,
                },
            ),
            (
                "unsigned long",
                Type::Int {
                    signed: false,
                    bits: 32,
                },
            ),
            (
                "long long",
                Type::Int {
                    signed: true,
                    bits: 64,
                },
            ),
            (
                "unsigned long long",
                Type::Int {
                    signed: false,
                    bits: 64,
                },
            ),
            ("float", Type::Float { bits: 32 }),
            ("double", Type::Float { bits: 64 }),
            ("long double", Type::Float { bits: 64 }),
        ] {
            let id = types.push(ty);
            types.fundamentals.insert(name, id);
        }
        types
    }

    fn push(&mut self, ty: Type) -> TypeId {
        self.nodes.push(ty);
        TypeId(self.nodes.len() as u32 - 1)
    }

    /// A struct or union type, incomplete until [`Types::complete_record`] is given its
    /// members. Declaring one twice with the same tag gets the same record, which is what makes
    /// a forward declaration work.
    pub fn record(&mut self, kind: RecordKind, tag: Option<Symbol>) -> (TypeId, RecordId) {
        let id = RecordId(self.records.len() as u32);
        self.records.push(Record {
            kind,
            tag,
            members: Vec::new(),
            size: 0,
            align: 1,
            complete: false,
        });
        (self.push(Type::Record(id)), id)
    }

    /// The record a type is, if it is one.
    pub fn record_of(&self, id: TypeId) -> Option<RecordId> {
        match self.kind(id) {
            Type::Record(record) => Some(*record),
            _ => None,
        }
    }

    /// Whether a record has its members yet.
    pub fn record_complete(&self, record: RecordId) -> bool {
        self.records[record.0 as usize].complete
    }

    /// The members of a record, once it has them.
    pub fn members(&self, record: RecordId) -> &[Member] {
        &self.records[record.0 as usize].members
    }

    /// The tag a record was declared with.
    pub fn record_tag(&self, record: RecordId) -> Option<Symbol> {
        self.records[record.0 as usize].tag
    }

    /// The kind of aggregate: struct or union.
    pub fn record_kind(&self, record: RecordId) -> RecordKind {
        self.records[record.0 as usize].kind
    }

    /// Where a member starts, and what it is.
    pub fn member(&self, record: RecordId, name: Symbol) -> Option<&Member> {
        self.records[record.0 as usize]
            .members
            .iter()
            .find(|member| member.name == Some(name))
    }

    /// Give a record its members and lay it out.
    ///
    /// The layout is the target's own: each member starts at the next offset that satisfies its
    /// alignment, a bitfield is packed into its unit, and a union's members all start at zero.
    pub fn complete_record(&mut self, record: RecordId, members: Vec<Member>) {
        let kind = self.records[record.0 as usize].kind;
        let mut offset = 0u64; // end of what has been laid out
        let mut size = 0u64; // the largest end, which is the size for a union
        let mut align = 1u64;
        // The bitfield unit being filled: where it starts, how many bits are left, how big it is.
        let mut unit_start = 0u64;
        let mut unit_left = 0u16;
        let mut unit_size = 0u64;

        let mut laid_out: Vec<Member> = Vec::with_capacity(members.len());
        for mut member in members {
            let member_align = self.align_of(member.ty).unwrap_or(1);
            align = align.max(member_align);
            match member.bits {
                Some((_, width)) => {
                    let member_size = self.size_of(member.ty).unwrap_or(1);
                    let unit_bits = (member_size * 8) as u16;
                    if kind == RecordKind::Union {
                        // Every member of a union starts at zero, and a bitfield is the same.
                        member.offset = 0;
                        member.bits = Some((0, width));
                        size = size.max(member_size);
                        laid_out.push(member);
                        continue;
                    }
                    let fits = unit_left >= u16::from(width) && unit_size == member_size;
                    if !fits {
                        // a new unit, aligned for the member's own type
                        offset = align_up(offset, member_align);
                        unit_start = offset;
                        unit_left = unit_bits;
                        unit_size = member_size;
                    }
                    let start_bit = unit_left.saturating_sub(u16::from(width)) as u8;
                    member.offset = unit_start;
                    // Bits are numbered from the least significant, as the target stores them.
                    member.bits = Some((start_bit, width));
                    unit_left -= u16::from(width);
                    if unit_left == 0 {
                        offset = unit_start + unit_size;
                        unit_size = 0;
                    }
                    size = size.max(offset.max(unit_start + member_size));
                }
                None => {
                    let start = if kind == RecordKind::Union {
                        0
                    } else {
                        align_up(offset, member_align)
                    };
                    member.offset = start;
                    let end = start + self.size_of(member.ty).unwrap_or(0);
                    offset = end;
                    unit_size = 0;
                    size = size.max(end);
                }
            }
            laid_out.push(member);
        }

        let record = &mut self.records[record.0 as usize];
        record.size = align_up(size, align.max(1));
        record.align = align.max(1);
        record.members = laid_out;
        record.complete = true;
    }

    /// The fundamental type with this spelling, if there is one.
    pub fn fundamental(&self, name: &str) -> Option<TypeId> {
        self.fundamentals.get(name).copied()
    }

    /// What a type is.
    pub fn kind(&self, id: TypeId) -> &Type {
        &self.nodes[id.0 as usize]
    }

    /// Give an incomplete array its count, once an initialiser says how many elements it has.
    pub fn complete_array(&mut self, id: TypeId, count: u64) {
        if let Type::Array { count: slot, .. } = &mut self.nodes[id.0 as usize] {
            *slot = Some(count);
        }
    }

    /// A pointer to `to`.
    pub fn pointer(&mut self, to: TypeId) -> TypeId {
        if matches!(self.kind(to), Type::Error) {
            return to;
        }
        self.push(Type::Pointer(to))
    }

    /// An array of `count` elements of `element`.
    pub fn array(&mut self, element: TypeId, count: Option<u64>) -> TypeId {
        if matches!(self.kind(element), Type::Error) {
            return element;
        }
        self.push(Type::Array { element, count })
    }

    /// A function type.
    pub fn function(&mut self, result: TypeId, params: Vec<TypeId>, variadic: bool) -> TypeId {
        self.push(Type::Function {
            result,
            params,
            variadic,
        })
    }

    /// Whether two types are the same type.
    pub fn same(&self, left: TypeId, right: TypeId) -> bool {
        if left == right {
            return true;
        }
        match (self.kind(left), self.kind(right)) {
            (Type::Error, _) | (_, Type::Error) => true, // poisoned types match to stay quiet
            (Type::Pointer(a), Type::Pointer(b)) => self.same(*a, *b),
            (
                Type::Array {
                    element: a,
                    count: ca,
                },
                Type::Array {
                    element: b,
                    count: cb,
                },
            ) => self.same(*a, *b) && (ca == cb || ca.is_none() || cb.is_none()),
            (Type::Function { .. }, Type::Function { .. }) => {
                self.describe(left) == self.describe(right)
            }
            (a, b) => a == b,
        }
    }

    /// Whether a value of this type is an integer.
    pub fn is_integer(&self, id: TypeId) -> bool {
        matches!(self.kind(id), Type::Bool | Type::Int { .. } | Type::Error)
    }

    /// Whether a value of this type is a floating-point number.
    pub fn is_float(&self, id: TypeId) -> bool {
        matches!(self.kind(id), Type::Float { .. } | Type::Error)
    }

    /// Whether a value of this type is arithmetic.
    pub fn is_arithmetic(&self, id: TypeId) -> bool {
        self.is_integer(id) || self.is_float(id)
    }

    /// Whether a constant of this type is C's zero: an integer 0, a null pointer, or a float
    /// whose value is 0.0 - `-0.0` included, which the bits do not say on their own.
    pub fn constant_is_zero(&self, ty: TypeId, bits: i64) -> bool {
        if !self.is_float(ty) {
            return bits == 0;
        }
        if self.size_of(ty) == Some(4) {
            f32::from_bits(bits as u32) == 0.0
        } else {
            f64::from_bits(bits as u64) == 0.0
        }
    }

    /// Whether a value of this type is a scalar: a number or a pointer (C99 6.2.5p21). What a
    /// condition, a logical operator and a conversion to `_Bool` all accept.
    pub fn is_scalar(&self, id: TypeId) -> bool {
        self.is_arithmetic(id) || self.is_pointer(id)
    }

    /// Integer promotion (C99 6.3.1.1): anything narrower than `int` becomes `int`. Every
    /// arithmetic operator does this to its operands first, so the rule lives with the
    /// conversions rather than with the parser that asks.
    pub fn promote(&self, id: TypeId) -> TypeId {
        match self.kind(id) {
            Type::Int { bits, .. } if *bits < 32 => self.fundamental("int").unwrap(),
            Type::Bool => self.fundamental("int").unwrap(),
            _ => id,
        }
    }

    /// Whether a value of this type *is* the address of the thing rather than bytes read from
    /// it: an array (C99 6.3.2.1p3) and a function designator (6.3.2.1p4). These are the cases
    /// where loading the place would read the wrong thing, or nothing at all (ADR-004).
    pub fn value_is_address(&self, id: TypeId) -> bool {
        matches!(self.kind(id), Type::Array { .. } | Type::Function { .. })
    }

    /// Whether a value of this type is a pointer.
    pub fn is_pointer(&self, id: TypeId) -> bool {
        matches!(self.kind(id), Type::Pointer(_) | Type::Error)
    }

    /// The type a pointer points to, or the error type.
    pub fn pointee(&self, id: TypeId) -> TypeId {
        match self.kind(id) {
            Type::Pointer(to) => *to,
            _ => self.error(),
        }
    }

    /// The error type: what a malformed declaration gets, so that one mistake is one
    /// diagnostic rather than a cascade.
    pub fn error(&self) -> TypeId {
        self.error
    }

    /// How many bytes a value of this type takes, if it has a size yet.
    pub fn size_of(&self, id: TypeId) -> Option<u64> {
        Some(match self.kind(id) {
            Type::Void => return None,
            Type::Bool => 1,
            Type::Int { bits, .. } => u64::from(*bits) / 8,
            Type::Float { bits } => u64::from(*bits) / 8,
            Type::Pointer(_) => 8,
            Type::Record(record) => {
                let record = &self.records[record.0 as usize];
                if !record.complete {
                    return None;
                }
                record.size
            }
            Type::Array {
                element,
                count: Some(count),
            } => self.size_of(*element)? * count,
            Type::Array { count: None, .. } => return None,
            Type::Function { .. } => return None,
            Type::Error => return None,
        })
    }

    /// The alignment of a value of this type, in bytes.
    pub fn align_of(&self, id: TypeId) -> Option<u64> {
        match self.kind(id) {
            Type::Record(record) => Some(self.records[record.0 as usize].align.max(1)),
            Type::Array { element, .. } => self.align_of(*element),
            _ => self.size_of(id).map(|size| size.clamp(1, 8)),
        }
    }

    /// Whether the type is complete enough to hold a value.
    pub fn is_complete(&self, id: TypeId) -> bool {
        match self.kind(id) {
            Type::Void | Type::Function { .. } | Type::Array { count: None, .. } | Type::Error => {
                false
            }
            Type::Record(record) => self.records[record.0 as usize].complete,
            _ => true,
        }
    }

    /// How a type is written, for a diagnostic: `pointer to unsigned int`.
    pub fn describe(&self, id: TypeId) -> String {
        self.describe_in(id, None)
    }

    /// The same, with the name table, so a `struct` tag is written by its name. The parser has
    /// both and calls this; the arena cannot, because it never saw a name.
    pub fn describe_with(&self, id: TypeId, names: &crate::intern::Interner) -> String {
        self.describe_in(id, Some(names))
    }

    fn describe_in(&self, id: TypeId, names: Option<&crate::intern::Interner>) -> String {
        match self.kind(id) {
            Type::Void => "void".to_owned(),
            Type::Bool => "_Bool".to_owned(),
            Type::Int { signed, bits } => {
                let width = match bits {
                    8 => "char",
                    16 => "short",
                    32 => "int",
                    _ => "long long",
                };
                if *signed {
                    width.to_owned()
                } else {
                    format!("unsigned {width}")
                }
            }
            Type::Float { bits } => {
                if *bits == 32 {
                    "float".to_owned()
                } else {
                    "double".to_owned()
                }
            }
            Type::Pointer(to) => format!("pointer to {}", self.describe_in(*to, names)),
            Type::Array { element, count } => match count {
                Some(count) => format!("array[{count}] of {}", self.describe_in(*element, names)),
                None => format!("array of {}", self.describe_in(*element, names)),
            },
            Type::Function {
                result,
                params,
                variadic,
            } => {
                let mut text = format!("function returning {}", self.describe_in(*result, names));
                if !params.is_empty() || *variadic {
                    let mut spelled: Vec<String> = params
                        .iter()
                        .map(|param| self.describe_in(*param, names))
                        .collect();
                    if *variadic {
                        spelled.push("...".to_owned());
                    }
                    text.push_str(&format!("({})", spelled.join(", ")));
                }
                text
            }
            Type::Record(record) => {
                let record = &self.records[record.0 as usize];
                let word = match record.kind {
                    RecordKind::Struct => "struct",
                    RecordKind::Union => "union",
                };
                match (record.tag, names) {
                    (Some(tag), Some(names)) => format!("{word} {}", names.resolve(tag)),
                    (Some(_), None) => word.to_owned(),
                    (None, _) => format!("{word} (untagged)"),
                }
            }
            Type::Error => "<error>".to_owned(),
        }
    }

    /// The arithmetic conversions: the common type two arithmetic operands are converted to
    /// (C99 6.3.1.8), simplified to what an integer-only front end meets first.
    pub fn usual_arithmetic(&self, left: TypeId, right: TypeId) -> TypeId {
        if matches!(self.kind(left), Type::Error) {
            return left;
        }
        if matches!(self.kind(right), Type::Error) {
            return right;
        }
        if self.is_float(left) && self.is_float(right) {
            return if self.size_of(left) >= self.size_of(right) {
                left
            } else {
                right
            };
        }
        if self.is_float(left) {
            return left;
        }
        if self.is_float(right) {
            return right;
        }
        // The integer promotions come first, before the two types are compared: `char +
        // char` is `int + int` and cannot wrap at 8 bits (C99 6.3.1.8).
        let (left, right) = (self.promote(left), self.promote(right));
        let (left_bits, left_signed) = self.int_parts(left);
        let (right_bits, right_signed) = self.int_parts(right);
        let bits = left_bits.max(right_bits);
        // A signed type of the same width beats unsigned only if it can hold every value of
        // the unsigned one; otherwise the result is unsigned, as C says.
        let signed = if left_bits == right_bits {
            left_signed && right_signed
        } else if left_bits > right_bits {
            left_signed
        } else {
            right_signed
        };
        self.integer_with(signed, bits)
    }

    /// Whether a value of `from` can be converted to `to` implicitly, with no diagnostic.
    pub fn is_assignable(&self, to: TypeId, from: TypeId) -> bool {
        if matches!(self.kind(to), Type::Error) || matches!(self.kind(from), Type::Error) {
            return true;
        }
        if self.same(to, from) {
            return true;
        }
        match (self.kind(to), self.kind(from)) {
            // C99 6.3.1.2: *any* scalar converts to `_Bool`, a pointer included. Nothing comes
            // the other way by this arm - a `_Bool` is an integer, so the arithmetic arms above
            // already take it, and a `_Bool` *value* is not a null pointer constant.
            (Type::Bool, _) => self.is_scalar(from),
            (Type::Int { .. } | Type::Float { .. }, _) => {
                self.is_arithmetic(from) && self.is_arithmetic(to)
            }
            (Type::Pointer(_), Type::Pointer(_)) => true,
            // Not yet right: what converts to a pointer is the *null pointer constant*, which
            // is a property of the value, not of the type. `1: ... => false` until the
            // expression checker can test the value (M3.2), so this cannot accept `p = 42;`.
            (Type::Pointer(_), Type::Int { bits, .. }) => *bits == 64,
            _ => false,
        }
    }

    /// Whether this is the error type: a poisoned value, which must never produce a second
    /// diagnostic of its own.
    pub fn is_error(&self, id: TypeId) -> bool {
        matches!(self.kind(id), Type::Error)
    }

    /// The type an expression of this type has when its value is used rather than assigned to:
    /// an array becomes a pointer to its first element, a function becomes a pointer to it
    /// (C99 6.3.2.1). This is what makes `f(a)` with `int a[10]` a pointer argument.
    pub fn decay(&mut self, id: TypeId) -> TypeId {
        match self.kind(id) {
            Type::Array { element, .. } => {
                let element = *element;
                self.pointer(element)
            }
            Type::Function { .. } => self.pointer(id),
            _ => id,
        }
    }

    /// The integer type of these bits and signedness.
    pub fn integer_with(&self, signed: bool, bits: u8) -> TypeId {
        let name = match (signed, bits) {
            (true, 8) => "char",
            (false, 8) => "unsigned char",
            (true, 16) => "short",
            (false, 16) => "unsigned short",
            (true, 32) => "int",
            (false, 32) => "unsigned int",
            (true, _) => "long long",
            (false, _) => "unsigned long long",
        };
        self.fundamentals[name]
    }

    fn int_parts(&self, id: TypeId) -> (u8, bool) {
        match self.kind(id) {
            Type::Bool => (8, false),
            Type::Int { bits, signed } => (*bits, *signed),
            _ => (32, true),
        }
    }
}

/// Round `value` up to the next multiple of `alignment`.
fn align_up(value: u64, alignment: u64) -> u64 {
    if alignment <= 1 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fundamental_types_are_made_once() {
        let types = Types::new();
        let int = types.fundamental("int").unwrap();
        assert_eq!(int, types.fundamental("int").unwrap());
        assert!(!types.same(int, types.fundamental("unsigned int").unwrap()));
        assert_eq!(types.describe(int), "int");
        assert_eq!(
            types.describe(types.fundamental("unsigned long").unwrap()),
            "unsigned int"
        );
    }

    #[test]
    fn sizes_are_the_win64_ones() {
        let types = Types::new();
        let size = |name| types.size_of(types.fundamental(name).unwrap()).unwrap();
        assert_eq!(size("char"), 1);
        assert_eq!(size("short"), 2);
        assert_eq!(size("int"), 4);
        assert_eq!(size("long"), 4);
        assert_eq!(size("long long"), 8);
        assert_eq!(size("float"), 4);
        assert_eq!(size("double"), 8);
        assert_eq!(size("long double"), 8);
        assert_eq!(types.size_of(types.fundamental("void").unwrap()), None);
        assert_eq!(types.size_of(types.error()), None);
    }

    #[test]
    fn pointers_and_arrays_are_built_from_their_parts() {
        let mut types = Types::new();
        let int = types.fundamental("int").unwrap();
        let pointer = types.pointer(int);
        assert_eq!(types.describe(pointer), "pointer to int");
        assert_eq!(types.size_of(pointer), Some(8));
        assert_eq!(types.pointee(pointer), int);

        let array = types.array(int, Some(3));
        assert_eq!(types.describe(array), "array[3] of int");
        assert_eq!(types.size_of(array), Some(12));
        let open = types.array(int, None);
        assert_eq!(types.size_of(open), None);
        assert!(!types.is_complete(open));
    }

    #[test]
    fn arrays_of_the_same_element_are_compatible_when_one_is_incomplete() {
        let mut types = Types::new();
        let char = types.fundamental("char").unwrap();
        let three = types.array(char, Some(3));
        let open = types.array(char, None);
        assert!(types.same(three, open));
        let four = types.array(char, Some(4));
        assert!(!types.same(three, four));
    }

    #[test]
    fn the_usual_arithmetic_conversions_promote_the_way_c_does() {
        let types = Types::new();
        let int = types.fundamental("int").unwrap();
        let uint = types.fundamental("unsigned int").unwrap();
        let long_long = types.fundamental("long long").unwrap();
        let double = types.fundamental("double").unwrap();

        assert_eq!(types.usual_arithmetic(int, int), int);
        assert_eq!(
            types.usual_arithmetic(int, uint),
            uint,
            "same width, one unsigned"
        );
        assert_eq!(
            types.usual_arithmetic(int, long_long),
            long_long,
            "wider signed wins"
        );
        assert_eq!(types.usual_arithmetic(int, double), double, "a float wins");
    }
}
