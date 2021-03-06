use std::fmt::Write;
use tinyvec::{tiny_vec, TinyVec};

use super::ISAInstruction;
use crate::id::*;

pub struct Widen<C: Tag> {
    pub result: RegisterId<C>,
    pub input: RegisterId<C>,
    pub typ: (),
}

impl<C: Tag> ISAInstruction<C> for Widen<C> {
    fn declared_register(&self) -> Option<RegisterId<C>> {
        Some(self.result)
    }

    fn used_registers(&self) -> TinyVec<[RegisterId<C>; 3]> {
        tiny_vec![self.input]
    }

    fn used_registers_mut(&mut self) -> Vec<&mut RegisterId<C>> {
        vec![&mut self.input]
    }

    fn display(&self, _w: &mut impl Write) -> std::fmt::Result {
        todo!()
    }
}
