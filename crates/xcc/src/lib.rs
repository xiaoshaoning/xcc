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
//!     Err(diagnostics) => {
//!         let text = xcc::render_all(&diagnostics, &[], xcc::Format::Human);
//!         eprintln!("{text}");
//!     }
//! }
//! ```
//!
//! Diagnostics come back as data ([`Diagnostic`]) so a host can show them in its own editor;
//! [`render`] and [`render_all`] turn them into text if it would rather not.
//!
//! # Status
//!
//! Milestone M1 of [PLAN.md]: the options, the diagnostic plumbing, rendering and the CLI.
//! [`Compile::compile`] answers with a "not implemented" diagnostic until the front end lands
//! (M2 and M3).

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(missing_debug_implementations)]

mod compile;
mod diag;
mod options;
mod render;
mod source;

pub use compile::{Artifact, Compile};
pub use diag::{Diagnostic, Severity, Span};
pub use options::{Budgets, CStandard, Options};
pub use render::{render, render_all, Format};
pub use source::{Position, SourceFile};
