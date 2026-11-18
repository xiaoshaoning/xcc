def edit(path, pairs):
    s = open(path, encoding='utf-8', newline='').read()
    nl = '\r\n' if '\r\n' in s else '\n'
    for old, new in pairs:
        o = old.replace('\n', nl); n = new.replace('\n', nl)
        assert o in s, (path, old[:80])
        s = s.replace(o, n, 1)
    open(path, 'w', encoding='utf-8', newline='').write(s)


# 1. whether a function calls anything is a fact about the function, decided once when it is lowered:
# the allocator needs it (what to save) and the encoder needs it (whether to reserve a staging area).
edit('crates/xcc/src/back/x64/mod.rs', [(
"""    /// Bytes of frame the lowering needed, block parameters included; spills are added after.
    pub frame_bytes: i32,""",
"""    /// Bytes of frame the lowering needed, block parameters included; spills are added after.
    pub frame_bytes: i32,
    /// Whether anything in this function calls another. A function that calls has to keep the
    /// callee-saved registers intact for its caller and can keep its own values in them; a function
    /// that calls nothing has neither obligation, and can use caller-saved registers instead - which
    /// are destroyed by a call, and there is no call.
    pub makes_call: bool,""")])

edit('crates/xcc/src/back/x64/lower.rs', [
("""    Ok(MachineFunction {
        name: names.resolve(func.name).to_string(),
        frame_bytes: lowerer.frame_bytes,
        blocks: lowerer.blocks,
        param_offsets: lowerer.param_offsets,
    })""",
 """    Ok(MachineFunction {
        name: names.resolve(func.name).to_string(),
        frame_bytes: lowerer.frame_bytes,
        makes_call,
        blocks: lowerer.blocks,
        param_offsets: lowerer.param_offsets,
    })"""),
("""    lowerer.run(func)?;""",
 """    lowerer.run(func)?;
    let makes_call = func.blocks.iter().any(|block| {
        block
            .insts
            .iter()
            .any(|inst| matches!(inst, Inst::Call { .. }))
    });"""),
])

# 2. the allocator picks the set that costs nothing to save, and saves only what the ABI asks it to
edit('crates/xcc/src/back/x64/regalloc.rs', [
("""/// The registers this allocator may use: the callee-saved ones.
pub(crate) const ALLOCATABLE: [Reg; 7] = [
    Reg(3),  // rbx
    Reg(6),  // rsi
    Reg(7),  // rdi
    Reg(12), // r12
    Reg(13), // r13
    Reg(14), // r14
    Reg(15), // r15
];""",
 """/// What a function that calls another may allocate: the callee-saved registers. A call cannot
/// destroy them, so a value in one is safe across the call for free, and the price is a `mov` to
/// save and another to restore it, twice per function.
pub(crate) const CALLEE_SAVED: [Reg; 7] = [
    Reg(3),  // rbx
    Reg(6),  // rsi
    Reg(7),  // rdi
    Reg(12), // r12
    Reg(13), // r13
    Reg(14), // r14
    Reg(15), // r15
];

/// What a function that calls nothing may allocate: caller-saved registers nothing has to preserve.
/// Fewer of them, because the encoder's scratch registers and the ones x86 fixes for shifts and
/// division are not available - but the prologue and epilogue then save nothing, which is where most
/// of the frame discipline in this back end went.
pub(crate) const CALLER_SAVED: [Reg; 2] = [
    Reg(8), // r8
    Reg(9), // r9
];"""),
("""/// Allocate registers for one function.
pub(crate) fn allocate(func: &MachineFunction) -> Allocation {""",
 """/// Allocate registers for one function.
pub(crate) fn allocate(func: &MachineFunction) -> Allocation {
    let allocatable = if func.makes_call {
        &CALLEE_SAVED[..]
    } else {
        &CALLER_SAVED[..]
    };"""),
("""    let mut active: Vec<Interval> = Vec::new();
    let mut free: Vec<Reg> = ALLOCATABLE.to_vec();""",
 """    let mut active: Vec<Interval> = Vec::new();
    let mut free: Vec<Reg> = allocatable.to_vec();"""),
("""        free.sort();
        // Coalescing:""",
 """        free.sort();
        free.sort();
        // Coalescing:"""),
# `saved` is what the prologue must preserve: nothing, when the set is caller-saved.
("""            if !saved.contains(&register) {
                saved.push(register);
            }
            active.push(interval);
            active.sort_by_key(|other| other.end);
        }
    }""",
 """            if func.makes_call && !saved.contains(&register) {
                saved.push(register);
            }
            active.push(interval);
            active.sort_by_key(|other| other.end);
        }
    }"""),
("""                if !saved.contains(&register) {
                    saved.push(register);
                }
                active.push(interval);
                active.sort_by_key(|other| other.end);
                continue;""",
 """                if func.makes_call && !saved.contains(&register) {
                    saved.push(register);
                }
                active.push(interval);
                active.sort_by_key(|other| other.end);
                continue;"""),
("""/// Whether a value already lives in a register, if it has been given an allocatable one.""",
 """/// Whether a value already lives in a register, if it has been given an allocatable one."""),
("""        Some(Location::Register(register)) if ALLOCATABLE.contains(register) => Some(*register),""",
 """        Some(Location::Register(register)) if CALLEE_SAVED.contains(register)
            || CALLER_SAVED.contains(register) =>
        {
            Some(*register)
        }"""),
])

# 3. the encoder asks the function instead of rescanning it
edit('crates/xcc/src/back/x64/encode.rs', [(
"""    let has_call = func.blocks.iter().any(|block| {
        block
            .ops
            .iter()
            .any(|op| matches!(op, Op::Call { .. }))
    });
    let raw = staging_base + if has_call { 32 } else { 0 };""",
 """    let raw = staging_base + if func.makes_call { 32 } else { 0 };""")])
print('ok')
