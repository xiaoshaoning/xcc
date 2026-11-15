//! What a compilation is asked for, and what it is allowed to spend.

/// Which C a source is allowed to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum CStandard {
    /// C99: the baseline, and the default.
    #[default]
    C99,
    /// C99 plus the C11 pieces a host meets in practice (`_Static_assert`, `_Generic`,
    /// `<stdatomic.h>`-free `_Atomic` declarations, anonymous members, `_Alignas`/`_Alignof`).
    C11,
}

/// What `__DATE__` and `__TIME__` report.
///
/// A compilation never reads a clock: the host that wants real values sets them, so that the
/// same source and the same options give the same tokens (CONSTRAINTS 4). Without one, the two
/// macros say what a compiler built in 1970 would say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    /// The date, in C's format: `"Nov 15 2026"`.
    pub date: String,
    /// The time, in C's format: `"12:34:56"`.
    pub time: String,
}

impl Stamp {
    /// A stamp that reads as C writes a date and a time.
    pub fn new(date: impl Into<String>, time: impl Into<String>) -> Self {
        Self {
            date: date.into(),
            time: time.into(),
        }
    }

    /// The stamp for a host that does not care: `"Jan  1 1970"`, `"00:00:00"`.
    pub fn epoch() -> Self {
        Self::new("Jan  1 1970", "00:00:00")
    }
}

/// Limits on one compilation, for hosts compiling code they did not write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budgets {
    /// The largest source file, in bytes, that will be looked at.
    pub max_source_bytes: usize,
    /// The most diagnostics reported before the rest are dropped.
    pub max_diagnostics: usize,
    /// How deeply macros may expand inside one another (CONSTRAINTS 5).
    pub max_macro_expansion: usize,
    /// The most tokens preprocessing may produce, macro expansion included.
    pub max_tokens: usize,
    /// How deep `#include` may nest (CONSTRAINTS 5).
    pub max_include_depth: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_source_bytes: 8 << 20,
            max_diagnostics: 100,
            max_macro_expansion: 64,
            max_tokens: 1 << 20,
            max_include_depth: 32,
        }
    }
}

/// Everything one [`crate::Compile`] needs to know.
///
/// What is not here is deliberate: the target is Windows x86-64 (ADR-001, decision 5) and
/// `long double` is 64-bit, the same as `double` (decision 3). A field that only ever holds one
/// value is a knob nobody turns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// The C the source is allowed to use.
    pub standard: CStandard,
    /// What this compilation may spend.
    pub budgets: Budgets,
    /// Macros defined before the source is read: `NAME` or `NAME=VALUE`.
    pub defines: Vec<String>,
    /// Macros undefined after those defines are installed.
    pub undefines: Vec<String>,
    /// What `__DATE__` and `__TIME__` say; the epoch when unset.
    pub stamp: Option<Stamp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_c99_with_the_stated_budgets() {
        let options = Options::default();
        assert_eq!(options.standard, CStandard::C99);
        assert_eq!(options.budgets.max_source_bytes, 8 << 20);
        assert_eq!(options.budgets.max_diagnostics, 100);
        assert_eq!(options.budgets.max_macro_expansion, 64);
        assert_eq!(options.budgets.max_tokens, 1 << 20);
        assert_eq!(options.budgets.max_include_depth, 32);
        assert!(options.defines.is_empty() && options.undefines.is_empty());
        assert_eq!(options.stamp, None);
    }

    #[test]
    fn a_stamp_can_be_the_epoch() {
        assert_eq!(Stamp::epoch().date, "Jan  1 1970");
        assert_eq!(Stamp::epoch().time, "00:00:00");
    }
}
