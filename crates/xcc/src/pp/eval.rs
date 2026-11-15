//! The constant-expression evaluator behind `#if` and `#elif`.
//!
//! C99 6.10.1: the controlling expression is evaluated as an integer constant expression in
//! `intmax_t` and `uintmax_t`. Two consequences are not optional:
//!
//! - `&&` and `||` do not evaluate what they do not need, so `#if 0 && 1/0` is not an error.
//! - `#if` may not divide by zero, or overflow a signed value: those are diagnosed rather than
//!   produced (CONSTRAINTS 3 - the alternative is a wrong answer nobody can see).
//!
//! Nothing here knows about macros: `pp` has already replaced `defined`, expanded what was left,
//! and turned the remaining identifiers into `0`, which is exactly what the standard says the
//! tokens mean by the time an expression is evaluated.

use crate::diag::Span;
use crate::intern::Interner;
use crate::lex::TokenKind;
use crate::pp::PpToken;

/// A value in `#if`: an `intmax_t` or a `uintmax_t`, told apart by `signed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Value {
    /// Whether the value is signed (`intmax_t`) rather than unsigned (`uintmax_t`).
    pub signed: bool,
    /// The bits, interpreted as `i64` when signed and as `u64` when unsigned.
    pub bits: u64,
}

impl Value {
    fn signed(value: i64) -> Self {
        Self {
            signed: true,
            bits: value as u64,
        }
    }

    fn unsigned(value: u64) -> Self {
        Self {
            signed: false,
            bits: value,
        }
    }

    fn int(self) -> i64 {
        self.bits as i64
    }

    fn uint(self) -> u64 {
        self.bits
    }

    /// Whether this value is true: non-zero, whatever its type.
    pub fn is_true(self) -> bool {
        self.bits != 0
    }
}

/// Why an expression could not be evaluated: a place, and what is wrong there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Failure {
    /// Where in the source.
    pub span: Span,
    /// What is wrong.
    pub message: String,
    /// Whether the expression is well formed but its value is not (`1/0`), as opposed to
    /// malformed. Only the former may be skipped by a short-circuit.
    pub arithmetic: bool,
}

impl Failure {
    fn at(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            arithmetic: false,
        }
    }

    fn value(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            arithmetic: true,
        }
    }
}

/// Evaluate the tokens of a `#if` expression.
pub(super) fn evaluate(tokens: &[PpToken], interner: &Interner) -> Result<Value, Failure> {
    let mut parser = Parser {
        tokens,
        at: 0,
        interner,
        span: tokens.first().map_or(Span::default(), |token| token.span),
    };
    let value = parser.conditional()?;
    match parser.peek() {
        None => Ok(value),
        Some(token) => Err(Failure::at(
            token.span,
            format!(
                "unexpected `{}` at the end of the expression",
                parser.spell(token)
            ),
        )),
    }
}

/// An integer constant as `#if` sees it: parsed, and typed by its suffix and magnitude.
pub(super) fn integer_literal(text: &str) -> Option<Value> {
    let lower = text.to_ascii_lowercase();
    let (digits, suffix) = split_suffix(&lower)?;
    let unsigned = suffix.contains('u');
    let bits = if let Some(hex) = digits.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()?
    } else if digits.len() > 1 && digits.starts_with('0') {
        u64::from_str_radix(&digits[1..], 8).ok()?
    } else {
        digits.parse::<u64>().ok()?
    };
    // A decimal constant that does not fit `intmax_t` is unsigned; a suffixed one always is.
    let signed = !unsigned && bits <= i64::MAX as u64;
    Some(Value { signed, bits })
}

/// Split `1u`, `0x1fULL` and `42` into digits and the suffix letters that are not digits.
fn split_suffix(text: &str) -> Option<(&str, String)> {
    let end = text.find(['u', 'l', 'U', 'L']).unwrap_or(text.len());
    let (digits, suffix) = text.split_at(end);
    let suffix = suffix.to_ascii_lowercase();
    if digits.is_empty()
        || !suffix
            .chars()
            .all(|character| matches!(character, 'u' | 'l'))
    {
        return None;
    }
    // `l`, `ll`, `u`, `ul`, `ull` and their permutations, but not `lll` or `uu`.
    let (u, l) = (suffix.matches('u').count(), suffix.matches('l').count());
    if u > 1 || l > 2 {
        return None;
    }
    Some((digits, suffix))
}

/// A character constant's value, for the escapes `#if` can meet.
pub(super) fn character_constant(text: &str) -> Option<Value> {
    let body = text.strip_prefix('L').unwrap_or(text);
    let body = body.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut characters = body.chars();
    let first = characters.next()?;
    if characters.next().is_some() && !body.starts_with('\\') {
        return None; // multi-character constant: not an integer constant expression we answer
    }
    let value = if first != '\\' {
        first as u32
    } else {
        let escape = body.strip_prefix('\\')?;
        match escape {
            "n" => 10,
            "t" => 9,
            "r" => 13,
            "0" => 0,
            "a" => 7,
            "b" => 8,
            "f" => 12,
            "v" => 11,
            "\\" => 92,
            "'" => 39,
            "\"" => 34,
            "?" => 63,
            _ => {
                if let Some(hex) = escape.strip_prefix('x') {
                    u32::from_str_radix(hex, 16).ok()?
                } else if escape.chars().all(|character| character.is_digit(8))
                    && !escape.is_empty()
                {
                    u32::from_str_radix(escape, 8).ok()?
                } else {
                    return None;
                }
            }
        }
    };
    Some(Value::signed(i64::from(value)))
}

struct Parser<'a> {
    tokens: &'a [PpToken],
    at: usize,
    interner: &'a Interner,
    span: Span,
}

impl Parser<'_> {
    fn spell(&self, token: &PpToken) -> &str {
        self.interner.resolve(token.text)
    }

    fn peek(&self) -> Option<&PpToken> {
        self.tokens.get(self.at)
    }

    fn span_here(&self) -> Span {
        self.peek().map_or(self.span, |token| token.span)
    }

    fn punct(&self) -> Option<&str> {
        let token = self.peek()?;
        if token.kind == TokenKind::Punct {
            Some(self.spell(token))
        } else {
            None
        }
    }

    fn eat(&mut self, spelling: &str) -> bool {
        if self.punct() == Some(spelling) {
            self.at += 1;
            return true;
        }
        false
    }

    /// `condition ? a : b`, right-associative, evaluating only the branch that is taken.
    fn conditional(&mut self) -> Result<Value, Failure> {
        let condition = self.binary(0)?;
        if !self.eat("?") {
            return Ok(condition);
        }
        let taken = condition.is_true();
        // C99 6.10.1 evaluates only the chosen arm. The other one is still parsed - it has to
        // be a constant expression - but a value failure in it (division by zero, overflow) is
        // not this expression's problem, so `#if 1 ? 1 : 1/0` is fine and `#if 1 ? 1/0 : 1` is
        // not.
        let when_true = self.arm(taken)?;
        if !self.eat(":") {
            return Err(Failure::at(self.span_here(), "expected `:`"));
        }
        let when_false = self.arm(!taken)?;
        Ok(if taken { when_true } else { when_false })
    }

    /// One arm of `?:`: a value failure in an arm that is not evaluated is not reported.
    fn arm(&mut self, evaluated: bool) -> Result<Value, Failure> {
        match self.conditional() {
            Ok(value) => Ok(value),
            Err(failure) if !evaluated && failure.arithmetic => Ok(Value::signed(0)),
            Err(failure) => Err(failure),
        }
    }

    /// Binary operators by precedence, left-associative (C99 6.5.5 to 6.5.12), with `&&` and
    /// `||` short-circuiting so that a division by zero they skip is not reported.
    fn binary(&mut self, minimum: usize) -> Result<Value, Failure> {
        const LEVELS: [&[&str]; 10] = [
            &["||"],
            &["&&"],
            &["|"],
            &["^"],
            &["&"],
            &["==", "!="],
            &["<", ">", "<=", ">="],
            &["<<", ">>"],
            &["+", "-"],
            &["*", "/", "%"],
        ];

        let mut left = self.unary()?;
        loop {
            let Some(operator) = self.punct() else {
                return Ok(left);
            };
            let Some(level) = LEVELS
                .iter()
                .position(|operators| operators.contains(&operator))
            else {
                return Ok(left);
            };
            if level < minimum {
                return Ok(left);
            }
            let spelling = operator.to_owned();
            let span = self.span_here();
            self.at += 1;
            // `&&` and `||` decide first: what they do not evaluate must not be diagnosed,
            // so `#if 0 && 1/0` is fine while `#if 1/0` is not.
            let skipped = match spelling.as_str() {
                "&&" => Some(!left.is_true()),
                "||" => Some(left.is_true()),
                _ => None,
            };
            if let Some(skipped) = skipped {
                let right = match self.binary(level + 1) {
                    Ok(value) => value,
                    Err(failure) if failure.arithmetic => Value::signed(0),
                    Err(failure) => return Err(failure),
                };
                let taken = if skipped {
                    spelling == "||"
                } else {
                    right.is_true()
                };
                left = Value::signed(i64::from(taken));
                continue;
            }
            // A left-associative operator parses its right side one level tighter.
            let right = self.binary(level + 1)?;
            left = apply(&spelling, left, right, span)?;
        }
    }

    fn unary(&mut self) -> Result<Value, Failure> {
        let Some(operator) = self.punct() else {
            return self.primary();
        };
        if !matches!(operator, "+" | "-" | "~" | "!") {
            return self.primary();
        }
        let spelling = operator.to_owned();
        let span = self.span_here();
        self.at += 1;
        let value = self.unary()?;
        Ok(match spelling.as_str() {
            "+" => value,
            "-" => {
                if value.signed {
                    match value.int().checked_neg() {
                        Some(negated) => Value::signed(negated),
                        None => return Err(Failure::value(span, "integer overflow in `-`")),
                    }
                } else {
                    Value::unsigned(value.uint().wrapping_neg())
                }
            }
            "~" => Value {
                signed: value.signed,
                bits: !value.bits,
            },
            _ => Value::signed(i64::from(!value.is_true())),
        })
    }

    fn primary(&mut self) -> Result<Value, Failure> {
        let Some(token) = self.peek() else {
            return Err(Failure::at(self.span, "expected a value"));
        };
        let span = token.span;
        let kind = token.kind;
        let text = self.spell(token).to_owned();
        self.at += 1;
        match kind {
            TokenKind::Punct if text == "(" => {
                let value = self.conditional()?;
                if !self.eat(")") {
                    return Err(Failure::at(self.span_here(), "expected `)`"));
                }
                Ok(value)
            }
            TokenKind::Number => integer_literal(&text).ok_or_else(|| {
                Failure::at(
                    span,
                    format!("`{text}` is not an integer constant that `#if` can use"),
                )
            }),
            TokenKind::Chr => character_constant(&text)
                .ok_or_else(|| Failure::at(span, format!("`{text}` is not a character constant"))),
            _ => Err(Failure::at(
                span,
                format!("`{text}` is not allowed in a constant expression"),
            )),
        }
    }
}

/// One binary operation, with the promotions `#if` needs and the errors C forbids.
fn apply(operator: &str, left: Value, right: Value, span: Span) -> Result<Value, Failure> {
    let signed = left.signed && right.signed;
    let arithmetic = |signed_left: i64, signed_right: i64| -> Result<Value, Failure> {
        let outcome = match operator {
            "+" => signed_left.checked_add(signed_right),
            "-" => signed_left.checked_sub(signed_right),
            "*" => signed_left.checked_mul(signed_right),
            "/" => {
                if signed_right == 0 {
                    return Err(Failure::value(span, "division by zero"));
                }
                signed_left.checked_div(signed_right)
            }
            _ => {
                if signed_right == 0 {
                    return Err(Failure::value(span, "division by zero"));
                }
                signed_left.checked_rem(signed_right)
            }
        };
        outcome
            .map(Value::signed)
            .ok_or_else(|| Failure::value(span, format!("integer overflow in `{operator}`")))
    };
    let unsigned_arithmetic = |left: u64, right: u64| -> Result<Value, Failure> {
        Ok(Value::unsigned(match operator {
            "+" => left.wrapping_add(right),
            "-" => left.wrapping_sub(right),
            "*" => left.wrapping_mul(right),
            "/" => {
                if right == 0 {
                    return Err(Failure::value(span, "division by zero"));
                }
                left / right
            }
            _ => {
                if right == 0 {
                    return Err(Failure::value(span, "division by zero"));
                }
                left % right
            }
        }))
    };

    match operator {
        "+" | "-" | "*" | "/" | "%" => {
            if signed {
                arithmetic(left.int(), right.int())
            } else {
                unsigned_arithmetic(left.uint(), right.uint())
            }
        }
        "<<" | ">>" => {
            let count = right.uint();
            if count >= 64 {
                return Err(Failure::value(
                    span,
                    format!("shift by {count} does not fit in 64 bits"),
                ));
            }
            let bits = if operator == "<<" {
                left.uint().wrapping_shl(count as u32)
            } else if left.signed {
                (left.int() >> count) as u64
            } else {
                left.uint() >> count
            };
            Ok(Value {
                signed: left.signed,
                bits,
            })
        }
        "<" | ">" | "<=" | ">=" => {
            let outcome = if signed {
                match operator {
                    "<" => left.int() < right.int(),
                    ">" => left.int() > right.int(),
                    "<=" => left.int() <= right.int(),
                    _ => left.int() >= right.int(),
                }
            } else {
                match operator {
                    "<" => left.uint() < right.uint(),
                    ">" => left.uint() > right.uint(),
                    "<=" => left.uint() <= right.uint(),
                    _ => left.uint() >= right.uint(),
                }
            };
            Ok(Value::signed(i64::from(outcome)))
        }
        "==" => Ok(Value::signed(i64::from(left.bits == right.bits))),
        "!=" => Ok(Value::signed(i64::from(left.bits != right.bits))),
        "&" | "|" | "^" => {
            let bits = match operator {
                "&" => left.bits & right.bits,
                "|" => left.bits | right.bits,
                _ => left.bits ^ right.bits,
            };
            Ok(Value { signed, bits })
        }
        other => Err(Failure::at(
            span,
            format!("`{other}` is not an operator `#if` knows"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> Value {
        integer_literal(text).unwrap_or_else(|| panic!("{text} is not an integer constant"))
    }

    #[test]
    fn integer_literals_know_their_bases_and_suffixes() {
        assert_eq!(value("42"), Value::signed(42));
        assert_eq!(value("0x1f"), Value::signed(31));
        assert_eq!(value("010"), Value::signed(8));
        assert_eq!(value("42u"), Value::unsigned(42));
        assert_eq!(value("42ULL"), Value::unsigned(42));
        assert_eq!(value("42l"), Value::signed(42));
        // one past intmax_t is unsigned, as C says for a decimal constant
        assert_eq!(value("18446744073709551615"), Value::unsigned(u64::MAX));
        assert!(integer_literal("1.5").is_none());
        assert!(integer_literal("0x").is_none());
        assert!(integer_literal("42uu").is_none());
        assert!(integer_literal("42lll").is_none());
    }

    #[test]
    fn character_constants_decode() {
        assert_eq!(character_constant("'a'"), Some(Value::signed(97)));
        assert_eq!(character_constant("'\\n'"), Some(Value::signed(10)));
        assert_eq!(character_constant("'\\0'"), Some(Value::signed(0)));
        assert_eq!(character_constant("'\\x41'"), Some(Value::signed(65)));
        assert_eq!(character_constant("L'z'"), Some(Value::signed(122)));
        assert_eq!(
            character_constant("'ab'"),
            None,
            "multi-character is not answered"
        );
    }
}
