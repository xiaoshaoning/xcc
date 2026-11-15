//! What a compilation is asked for, and what it is allowed to spend.

/// What the compiled program is meant to run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Target {
    /// Windows x86-64: COFF objects, Windows calling convention, `long double` as `f64`.
    /// The only target that produces code today.
    #[default]
    Win64,
    /// The 16-bit DOS compiler from `borland-reborn`, through `libbcc20`. Not produced by this
    /// crate: it exists so that the same source can be compiled by a 1991 compiler and by this
    /// one and compared.
    Dos16,
}

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

/// How wide `long double` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum LongDouble {
    /// 64-bit: the same type as `double`, as MSVC has it on x86-64. The only option: the back
    /// end emits SSE, and there is no 80-bit arithmetic to be had from it.
    #[default]
    F64,
}

/// Limits on one compilation, for hosts compiling code they did not write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budgets {
    /// The largest source file, in bytes, that will be looked at.
    pub max_source_bytes: usize,
    /// The most diagnostics reported before the rest are dropped.
    pub max_diagnostics: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_source_bytes: 8 << 20,
            max_diagnostics: 100,
        }
    }
}

/// Everything one [`crate::Compile`] needs to know.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// The target machine.
    pub target: Target,
    /// The C the source is allowed to use.
    pub standard: CStandard,
    /// The width of `long double`.
    pub long_double: LongDouble,
    /// What this compilation may spend.
    pub budgets: Budgets,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_c99_on_windows() {
        let options = Options::default();
        assert_eq!(options.target, Target::Win64);
        assert_eq!(options.standard, CStandard::C99);
        assert_eq!(options.long_double, LongDouble::F64);
        assert_eq!(options.budgets.max_source_bytes, 8 << 20);
        assert_eq!(options.budgets.max_diagnostics, 100);
    }
}
