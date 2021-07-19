use crate::frontend::ir::{BasicBlockJump, ControlFlowInstruction, Instruction};

use super::conv_only_bb::PureBlocks;
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

pub fn display(program: &PureBlocks) -> String {
    let mut text = String::new();

    for (id, block) in program.blocks.iter() {
        iw!(text, "fn @{}(", id);

        for p in block.parameters.iter() {
            iw!(text, "%{}, ", p);
        }

        iw!(text, ") {{\n");

        for inst in block.instructions.iter() {
            match inst {
                Instruction::RecordNew(r) => {
                    iwl!(text, "    %{} = RecordNew", r.result,)
                }
                Instruction::RecordGet(_) => iwl!(text, "todo"),
                Instruction::RecordSet(_) => iwl!(text, "todo"),
                Instruction::CallVirt(t) => iwl!(text, "todo"),
                Instruction::CallExtern(t) => iwl!(text, "todo"),
                Instruction::CallStatic(t) => iwl!(text, "todo"),
                Instruction::MakeTrivial(t) => iwl!(text, "todo"),
                Instruction::MakeString(r, s) => {
                    iwl!(text, "    %{} = MakeString {}", r, s);
                }
                Instruction::MakeInteger(r, i) => {
                    iwl!(text, "    %{} = MakeNumber {}", r, i);
                }
                // Instruction::M(r, v) => iwl!(
                //     text,
                //     "    %{} = MakeBoolean {}",
                //     r,
                //     display_vt(f.register_types.get(*r), f),
                //     v
                // ),
                Instruction::ReferenceOfFunction(r, f) => {
                    iwl!(text, "    %{} = MakeFnPtr @{}", r, f)
                }
                Instruction::CompareLessThan(inst) => {
                    let r = inst.result;
                    let l = inst.lhs;
                    let rr = inst.rhs;
                    iwl!(text, "    %{} = CompareLessThan %{}, %{}", r, l, rr)
                }
                Instruction::CompareEqual(r, l, rr) => {
                    iwl!(text, "    %{} = CompareEqual %{}, %{}", r, l, rr)
                }
                Instruction::Negate(r, i) => iwl!(text, "    %{} = Negate %{}", r, i),
                Instruction::Add(re, l, r) => iwl!(text, "    %{} = Add %{}, %{}", re, l, r),
            }
        }

        match &block.end {
            ControlFlowInstruction::Jmp(BasicBlockJump(id, args)) => {
                iwl!(text, "    Jump ${}({:?})", id, args)
            }
            ControlFlowInstruction::JmpIf {
                condition,
                true_path,
                false_path,
            } => iwl!(
                text,
                "    if %{}:\n        Jump ${}({:?})\n    else\n        Jump ${}({:?})",
                condition,
                true_path.0,
                true_path.1,
                false_path.0,
                false_path.1,
            ),
            ControlFlowInstruction::Ret(r) => {
                iwl!(text, "    Ret {:?}", r)
            }
        };

        iw!(text, "}}\n\n");
    }

    text
}
