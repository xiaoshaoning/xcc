//! A small, embeddable C compiler for Windows x86-64.
//!
//! `xcc` compiles C99 (with C11 pieces where a host meets them) to native code in process, to
//! be linked or called as a function pointer. It is designed to be embedded in another
//! program, so it has no global state: every compilation owns its own [`Compile`], and two of
//! them can run on two threads.
//!
//! ```no_run
//! use xcc::{Compile, Options};
//!
//! let mut compiler = Compile::new(Options::default());
//! match compiler.compile("t.c", "int add(int a, int b) { return a + b; }\n") {
//!     Ok(artifact) => println!("{} bytes of object", artifact.object().len()),
//!     Err(diagnostics) => eprintln!("{}", diagnostics.render(xcc::Format::Human)),
//! }
//! ```
//!
//! Diagnostics come back as data ([`Diagnostic`]) so a host can show them in its own editor;
//! [`render`] and [`render_all`] turn them into text if it would rather not.
//!
//! # Status
//!
//! M6 of [PLAN.md]: the preprocessor, the C99 front end, and code generation for scalars, floats,
//! pointers, aggregates by value and bitfields are all in place, and a corpus of C programs runs
//! with the same results as `clang -O0`. Varargs is the one M6 item still refused, with the reason
//! in [docs/ADR-006-varargs.md]; [docs/C99-SUPPORT.md] is the line-by-line answer to "can this
//! compiler do X".

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(missing_debug_implementations)]

// `cir` is built before the parser that fills it: M4.1 is its shape, its verifier and its dump,
// and nothing constructs a module until M4.2. The allow leaves with that commit.
#[allow(dead_code)]
mod back;
mod cir;
mod compile;
mod diag;
mod explain;
mod headers;
mod intern;
mod lex;
mod literal;
mod options;
mod parse;
pub mod pp;
mod render;
mod source;
mod symtab;
mod types;

pub use back::jit::Jit;
pub use compile::{Artifact, Compile};
pub use diag::{Diagnostic, Diagnostics, Severity, Span};
pub use intern::{Interner, Symbol};
pub use lex::{lex, Lexed, Prepared, Token, TokenKind};
pub use options::{Budgets, CStandard, Options, Stamp};
pub use pp::{preprocess, Includes, PpToken, Preprocessed};
pub use render::{render, render_all, Format};
pub use source::{Position, SourceFile};
