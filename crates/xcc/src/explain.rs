//! `--explain`: what one compilation made of each function, and why.
//!
//! Two views of the same function, in the order the compiler made them. First the `cir`
//! instructions - what the C said, after the promotions, decays and conversions that C's rules
//! imply - each under the source line it came from, so an instruction whose reason is not obvious
//! can be read next to the expression that asked for it. Then the Cranelift function, where the
//! virtual registers appear: `v3 = iadd v1, v2` is the answer to "which registers were used".
//!
//! The two are not paired line by line, and that is honest: one C instruction becomes zero, one or
//! several machine instructions (a constant folds away, a call becomes a load and a call), and
//! pretending otherwise would need a mapping the back end does not keep. The order is the same on
//! both sides, which is enough to read them together.

use std::fmt::Write;

use crate::back::Explained;
use crate::source::SourceFile;

/// Write the explanation of every function in `explained`.
pub(crate) fn render(explained: &[Explained], files: &[SourceFile]) -> String {
    let mut out = String::new();
    for (index, function) in explained.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let _ = writeln!(out, "function {}", function.name);
        if function.cir.is_empty() {
            let _ = writeln!(out, "  (no instructions)");
        }
        // The source line is printed once per run of instructions from it: a return statement
        // emits several, and repeating the line four times hides the shape instead of showing it.
        let mut shown: Option<u32> = None;
        for (span, instruction) in &function.cir {
            if let Some(line) = line_of(*span, files) {
                if shown != Some(line) {
                    if let Some(text) = files.first().and_then(|file| file.line(line)) {
                        let _ = writeln!(out, "{line:>6} | {text}");
                    }
                    shown = Some(line);
                }
            }
            let _ = writeln!(out, "       |   {instruction}");
        }
        let _ = writeln!(out, "  cranelift:");
        for line in function.clif.lines() {
            let _ = writeln!(out, "    {line}");
        }
    }
    out
}

/// The line a span starts on, when the span is a real one in the unit's own file.
///
/// A `Span` is an offset and a length, without the file it is an offset into: the translation
/// unit's own source is the file every instruction of a one-file compile came from, and until the
/// span carries a file (M8, when headers make the question real) an offset past the end of it is
/// an instruction listed without a line rather than one shown against the wrong line.
fn line_of(span: crate::diag::Span, files: &[SourceFile]) -> Option<u32> {
    let file = files.first()?;
    if span.start as usize > file.len() {
        return None;
    }
    Some(file.position(span.start).line)
}

#[cfg(test)]
mod tests {
    use crate::options::Options;

    fn explanation(source: &str) -> String {
        let mut compiler = crate::compile::Compile::new(Options::default());
        compiler
            .explain("t.c", source)
            .expect("the explanation of a program that compiles")
    }

    #[test]
    fn each_instruction_sits_under_the_line_it_came_from() {
        let text = explanation(
            "int add(int a, int b) {\n\
             \x20   return a + b;\n\
             }\n",
        );
        // The function, the source line, its instruction, and the registers the back end chose.
        assert!(text.contains("function add"), "{text}");
        assert!(text.contains("     2 |     return a + b;"), "{text}");
        assert!(text.contains("v6 = add int v4, v5"), "{text}");
        assert!(text.contains("  cranelift:"), "{text}");
        assert!(text.contains("iadd"), "{text}");
    }

    #[test]
    fn a_line_that_emits_several_instructions_is_printed_once() {
        // `s = a * b + c;` is a multiply, an add and a store, all from one line.
        let text = explanation(
            "int f(int a, int b, int c) {\n\
             \x20   int s;\n\
             \x20   s = a * b + c;\n\
             \x20   return s;\n\
             }\n",
        );
        assert_eq!(text.matches("s = a * b + c;").count(), 1, "{text}");
        assert_eq!(text.matches("return s;").count(), 1, "{text}");
        assert!(text.contains("v10 = mul int v8, v9"), "{text}");
        assert!(text.contains("v12 = add int v10, v11"), "{text}");
        assert!(text.contains("store v12, v6"), "{text}");
    }

    #[test]
    fn what_a_promotion_added_is_visible_as_an_instruction() {
        // `char + char` is an `int` addition: the conversion is an instruction of its own, and
        // this is the feature's whole reason - it is what the C rules added to a plain `+`.
        let text = explanation(
            "int f(char a, char b) {\n\
             \x20   return a + b;\n\
             }\n",
        );
        assert!(text.contains("v6 = sext char to int v4"), "{text}");
        assert!(text.contains("v8 = add int v6, v7"), "{text}");
        // And what the back end made of each: the C-level sign extension is one instruction.
        assert!(text.contains("sextend.i32"), "{text}");
    }
}
