//! Running what the back end made, without a linker.
//!
//! The object writer and the JIT want the same thing from the encoder - sections and a list of
//! places that name a symbol - and differ only in who fills those places in. A linker does it in a
//! file the operating system loads later; this does it in memory the process can already jump to.
//!
//! Two sections, one allocation: `.text` at the start, `.data` after it, aligned. Every relocation
//! is resolved before the memory is made executable, so the region is never writable *and*
//! executable at once.

use super::coff::{self, Image};
use super::{lower, NotYet};
use crate::cir::Module;
use crate::intern::Interner;

/// What a loaded translation unit is: executable memory, and where each of its names landed.
pub(crate) struct Loaded {
    /// The allocation: `.text` at offset zero, `.data` after it.
    memory: *mut u8,
    /// How many bytes were allocated, for the release.
    size: usize,
    /// The names this unit defines, and where they are in the allocation.
    symbols: Vec<(String, usize)>,
}

// SAFETY: the memory is owned by this value and freed once; nothing else in the process has its
// address until `address` is called, and the code in it does not hold Rust references.
#[allow(unsafe_code)]
unsafe impl Send for Loaded {}

impl Loaded {
    /// The **address** of a name this unit defines: its offset added to where the allocation landed.
    /// The offset alone is what the relocations are written in terms of, and using it as an address
    /// is a jump to somewhere inside the first pages of the process - which is how this was found.
    pub(crate) fn address(&self, name: &str) -> Option<usize> {
        self.symbols
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, offset)| self.memory as usize + offset)
    }

    /// Where a name sits *within* the allocation, which is what a relocation inside it wants.
    fn offset(&self, name: &str) -> Option<usize> {
        self.symbols
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, offset)| *offset)
    }
}

#[allow(unsafe_code)]
impl Drop for Loaded {
    fn drop(&mut self) {
        if !self.memory.is_null() {
            // SAFETY: the pointer came from `VirtualAlloc` with this size and is released once.
            unsafe {
                VirtualFree(self.memory.cast(), 0, MEM_RELEASE);
            }
        }
    }
}

/// `size` bytes of read-write memory within a `REL32` of `near`, or null.
///
/// `VirtualAlloc` honours an address hint when the range is free, so the hint walks outwards in
/// sixteen-megabyte steps - a hundred and twenty-eight of them is two gigabytes, the whole reach of
/// the call this back end emits - and the first one granted wins.
#[allow(unsafe_code)]
fn allocate_near(size: usize, near: usize) -> *mut u8 {
    const STEP: usize = 0x100_0000;
    const REACH: usize = 0x7000_0000; // well inside two gigabytes, leaving room for the image
    for step in 0..(REACH / STEP) {
        for direction in [1usize, usize::MAX] {
            let offset = step.wrapping_mul(STEP).wrapping_mul(direction);
            let hint = near.wrapping_add(offset);
            // SAFETY: a hint the allocator may ignore; the result is checked before use.
            let memory = unsafe {
                VirtualAlloc(
                    hint as *mut u8,
                    size,
                    MEM_COMMIT | MEM_RESERVE,
                    PAGE_READWRITE,
                )
            };
            if !memory.is_null() {
                return memory;
            }
        }
    }
    std::ptr::null_mut()
}

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(address: *mut u8, size: usize, allocation: u32, protect: u32) -> *mut u8;
    fn VirtualFree(address: *mut u8, size: usize, free_type: u32) -> i32;
    fn VirtualProtect(address: *mut u8, size: usize, protect: u32, previous: *mut u32) -> i32;
}

/// One page of memory, which has one protection for everything in it.
const PAGE: usize = 0x1000;
const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_EXECUTE_READ: u32 = 0x20;

/// Load a translation unit: allocate, copy, resolve every relocation, then make it executable.
///
/// `resolve` answers where a name this unit does not define lives - a C runtime function, a host's
/// symbol, or a name an earlier unit loaded. The two sections are filled in one allocation, so a
/// `REL32` from the code to an object here is a subtraction and an absolute address in the data
/// section is an addition.
#[allow(unsafe_code)]
pub(crate) fn load(
    module: &Module,
    names: &Interner,
    resolve: &dyn Fn(&str) -> Option<usize>,
    near: usize,
) -> Result<Loaded, NotYet> {
    let functions = lower::lower(module, names)?;
    let data = super::data_section(module)?;
    let image = coff::assemble(&functions, data);
    let Image {
        text,
        functions,
        data,
    } = image;
    // The data section starts at the next *page*: the two are protected differently - code is
    // executable and read-only, data is writable and not executable - and a page has one protection
    // for everything in it. Sixteen bytes would do for alignment; four kilobytes is what keeps
    // `static int n; n++` from faulting on the page holding the code that ran it.
    let data_start = text.len().div_ceil(PAGE) * PAGE;
    let size = (data_start + data.bytes.len()).max(1);

    // **The allocation has to be near what the code calls.** A call this back end emits is
    // `E8 rel32`, which reaches two gigabytes either way, and a region the allocator puts a hundred
    // terabytes from the C runtime cannot reach it: the first version of this loader jumped into
    // nothing for exactly that reason. So the address is hinted, stepping away from a known symbol
    // until Windows grants a range that is in reach.
    let memory = allocate_near(size, near);
    if memory.is_null() {
        return Err(NotYet {
            message: format!("{size} bytes of executable memory"),
            span: module
                .funcs
                .first()
                .map_or_else(|| crate::diag::Span::at(0), |func| func.entry_span()),
        });
    }
    let mut loaded = Loaded {
        memory,
        size,
        symbols: Vec::new(),
    };
    let base = memory as usize;
    // SAFETY: the region was just allocated with this size and is not yet executable.
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), memory, text.len());
        std::ptr::copy_nonoverlapping(
            data.bytes.as_ptr(),
            memory.add(data_start),
            data.bytes.len(),
        );
    }
    for function in &functions {
        loaded
            .symbols
            .push((function.name.clone(), function.start as usize));
    }
    for object in &data.symbols {
        loaded
            .symbols
            .push((object.name.clone(), data_start + object.offset as usize));
    }

    // Where a name is, whether it is ours or somebody else's: an address, because the code and the
    // data are wherever the allocator put them.
    let place_of = |name: &str, loaded: &Loaded| -> Option<usize> {
        loaded
            .offset(name)
            .map(|offset| base + offset)
            .or_else(|| resolve(name))
    };

    for function in &functions {
        for relocation in &function.relocations {
            let Some(target) = place_of(&relocation.symbol, &loaded) else {
                return Err(NotYet {
                    message: format!(
                        "`{}` is not defined here and cannot be resolved: a unit loaded into memory can only call what it has",
                        relocation.symbol
                    ),
                    span: module.funcs.first().map_or_else(
                        || crate::diag::Span::at(0),
                        |func| func.entry_span(),
                    ),
                });
            };
            let place = base + function.start as usize + relocation.offset as usize;
            let next = place + 4;
            let relative = (target as isize) - (next as isize);
            // SAFETY: `place` is inside the text we just copied, and four bytes wide, which is what
            // `REL32` names.
            unsafe {
                std::ptr::write_unaligned(place as *mut i32, relative as i32);
            }
        }
    }
    for relocation in &data.relocations {
        let Some(target) = place_of(&relocation.symbol, &loaded) else {
            return Err(NotYet {
                message: format!(
                    "`{}` is not defined here and cannot be resolved: an address in the data section has to point somewhere",
                    relocation.symbol
                ),
                span: module.funcs.first().map_or_else(
                    || crate::diag::Span::at(0),
                    |func| func.entry_span(),
                ),
            });
        };
        let place = base + data_start + relocation.offset as usize;
        // `ADDR64` means `symbol + addend`, and the addend is whatever the field already holds -
        // zero today, because the front end only writes plain addresses, and added rather than
        // overwritten so that an address *plus an offset* cannot become a silent wrong answer later.
        // SAFETY: the place is inside the data we just copied, and eight bytes wide, which is what
        // `ADDR64` names.
        unsafe {
            let addend = std::ptr::read_unaligned(place as *const u64);
            std::ptr::write_unaligned(place as *mut u64, target as u64 + addend);
        }
    }

    let mut previous = 0u32;
    let span = module
        .funcs
        .first()
        .map_or_else(|| crate::diag::Span::at(0), |func| func.entry_span());
    // SAFETY: both ranges are inside the allocation, fully written, and each is given the protection
    // its contents need: code that may run, data that may be written.
    let code = unsafe {
        VirtualProtect(
            memory,
            data_start,
            PAGE_EXECUTE_READ,
            &mut previous as *mut u32,
        )
    };
    let data_ok = data.bytes.is_empty()
        || unsafe {
            VirtualProtect(
                memory.add(data_start),
                size - data_start,
                PAGE_READWRITE,
                &mut previous as *mut u32,
            )
        } != 0;
    if code == 0 || !data_ok {
        return Err(NotYet {
            message: String::from("the loaded code could not be made executable"),
            span,
        });
    }
    Ok(loaded)
}
