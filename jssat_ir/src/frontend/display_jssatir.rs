use crate::{frontend::ir::ControlFlowInstruction, isa::ISAInstruction};

use super::ir::{FFIValueType, IR};
use std::fmt::Write;

/// Infallible write
macro_rules! iw {
    ($($e:tt)+) => {
        write!($($e)+).unwrap()
    };
}
/// Infallible writeln
macro_rules! iwl {
    ($($e:tt)+) => {
        writeln!($($e)+).unwrap()
    };
}

pub fn display(program: &IR) -> String {
    let mut text = String::new();

    for (id, ext_fn) in program.external_functions.iter() {
        iw!(text, "ext fn @@{}(", id);

        for arg in ext_fn.parameters.iter() {
            iw!(text, "{}, ", display_norecord(arg));
        }

        iw!(text, ");\n\n");
    }

    let mut fns = program.functions.iter().collect::<Vec<_>>();
    fns.sort_by(|(a, _), (b, _)| a.cmp(b));

    for (fn_id, f) in fns {
        iw!(text, "fn @{}(", fn_id);

        for p in f.parameters.iter() {
            iw!(text, "%{}, ", p.register);
        }

        iw!(text, ") {{\n");

        let mut blocks = std::iter::once((&f.entry_block, f.blocks.get(&f.entry_block).unwrap()))
            .chain(f.blocks.iter().filter(|(id, _)| **id != f.entry_block))
            .collect::<Vec<_>>();
        blocks.sort_by(|(a, _), (b, _)| a.cmp(b));

        for (id, block) in blocks {
            iw!(text, "  @{}.${}(", fn_id, id);
            for arg in block.parameters.iter() {
                iw!(text, "%{}, ", arg);
            }
            iw!(text, "):\n");

            for inst in block.instructions.iter() {
                iwl!(text, "    {}", inst.as_display());
            }

            iw!(text, "    ");
            match &block.end {
                ControlFlowInstruction::Jmp(inst) => inst.display(&mut text),
                ControlFlowInstruction::JmpIf(inst) => inst.display(&mut text),
                ControlFlowInstruction::Ret(inst) => inst.display(&mut text),
            }
            .unwrap();
            iwl!(text);
        }

        iw!(text, "}}\n\n");
    }

    text
}

fn display_norecord(t: &FFIValueType) -> String {
    match t {
        // ValueType::Bool(b) => format!("{}", b),
        // ValueType::ExactInteger(i) => format!("{}", i),
        // ValueType::ExactString(payload) => display_str(payload),
        // ValueType::Record(_) => unreachable!(),
        // ValueType::FnPtr(_) => "todo::FnPtr".into(),
        _ => format!("{:?}", t),
    }
}
