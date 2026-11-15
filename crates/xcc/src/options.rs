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
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_source_bytes: 8 << 20,
            max_diagnostics: 100,
            max_macro_expansion: 64,
            max_tokens: 1 << 20,
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
    }
}
