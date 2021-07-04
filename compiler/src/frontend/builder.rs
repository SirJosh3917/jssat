use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::frontend::ir::*;
use crate::id::{Counter, IdCompat};
use crate::name::DebugName;

type BlockId = crate::id::BlockId<crate::id::IrCtx>;
type FunctionId = crate::id::FunctionId<crate::id::IrCtx>;
type ConstantId = crate::id::ConstantId<crate::id::IrCtx>;
type RegisterId = crate::id::RegisterId<crate::id::IrCtx>;
type ExternalFunctionId = crate::id::ExternalFunctionId<crate::id::IrCtx>;

// TODO: these should be doctests probably, but those don't run in a binary crate
#[test]
#[should_panic]
pub fn panics_on_drop_with_function_start_without_function_end() {
    let builder = ProgramBuilder::new();
    #[allow(unused_variables)]
    let (my_fn, []) = builder.start_function("my_fn");
    // Oops! This accidentally got commented out!
    // builder.end_function(my_fn);
}

pub struct ProgramBuilder {
    entrypoint: Option<FunctionId>,
    constants: Vec<Constant>,
    external_functions: Vec<ExternalFunction>,
    functions: FxHashMap<FunctionId, Function>,
    gen_function_id: Counter<FunctionId>,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct ExtFnIdTyped<const PARAMETERS: usize>(pub ExternalFunctionId);

impl ProgramBuilder {
    pub fn new() -> Self {
        ProgramBuilder {
            entrypoint: None,
            constants: vec![],
            external_functions: vec![],
            functions: FxHashMap::default(),
            gen_function_id: Counter::new(),
        }
    }

    pub fn finish(self) -> IR {
        let entrypoint = self.entrypoint.expect("expected an entrypoint function! generate an entrypoint function using ProgramBuilder::start_function_main()");

        fn into_map<I: IdCompat, E>(collection: impl IntoIterator<Item = E>) -> FxHashMap<I, E> {
            collection
                .into_iter()
                .enumerate()
                .map(|(idx, element)| (I::new_with_value(idx), element))
                .collect()
        }

        IR {
            entrypoint,
            constants: into_map(self.constants),
            external_functions: into_map(self.external_functions),
            functions: self.functions,
        }
    }

    pub fn constant(&mut self, name: &str, payload: Vec<u8>) -> ConstantId {
        self.constants.push(Constant {
            name: DebugName::new(name),
            payload,
        });

        let id = self.constants.len() - 1;
        ConstantId::new_with_value(id)
    }

    pub fn constant_str_utf16(&mut self, name: &str, message: String) -> ConstantId {
        let mut payload = Vec::with_capacity(message.len() * 2);

        let utf16_payload = message
            .encode_utf16()
            .into_iter()
            .flat_map(|x| std::array::IntoIter::new(x.to_ne_bytes()));

        payload.extend(utf16_payload);

        self.constant(name, payload)
    }

    pub fn external_function<N: ToString, const PARAMETERS: usize>(
        &mut self,
        name: N,
        parameters: [FFIValueType; PARAMETERS],
        return_type: FFIReturnType,
    ) -> ExtFnIdTyped<PARAMETERS> {
        let id = self.external_function_dynargs(name, parameters.to_vec(), return_type);
        ExtFnIdTyped(id)
    }

    pub fn external_function_dynargs<N: ToString>(
        &mut self,
        name: N,
        parameters: Vec<FFIValueType>,
        return_type: FFIReturnType,
    ) -> ExternalFunctionId {
        self.external_functions.push(ExternalFunction {
            name: name.to_string(),
            parameters: parameters.to_vec(),
            return_type,
        });

        let id = self.external_functions.len() - 1;
        ExternalFunctionId::new_with_value(id)
    }

    pub fn start_function_main(&mut self) -> FunctionBuilder<'static, 0> {
        assert!(
            matches!(self.entrypoint, None),
            "can only define one entrypoint function"
        );

        let (builder, []) = self.start_function("main");
        self.entrypoint = Some(builder.id);
        builder
    }

    pub fn start_function<'name, const PARAMETERS: usize>(
        &self,
        name: &'name str,
    ) -> (FunctionBuilder<'name, PARAMETERS>, [RegisterId; PARAMETERS]) {
        let id = self.gen_function_id.next();

        // TODO: is there a better way to do this?
        // PARAMETERS = 3, parameters = [RegisterId::new_with_value_const(0), ...(1), ...(2)]
        let mut parameters = [RegisterId::new_const(); PARAMETERS];
        for (idx, parameter) in parameters.iter_mut().enumerate() {
            *parameter = RegisterId::new_with_value_const(idx);
        }

        (FunctionBuilder::new(id, name), parameters)
    }

    pub fn end_function<'n, const PARAMETERS: usize>(
        &mut self,
        mut builder: FunctionBuilder<'n, PARAMETERS>,
    ) -> FnSignature<PARAMETERS> {
        let signature = builder.signature();

        builder.is_ok_to_drop = true;
        let function = builder.finish();

        self.functions.insert(signature.id, function);

        signature
    }
}

pub struct FunctionBuilder<'name, const PARAMETERS: usize> {
    pub id: FunctionId,
    name: &'name str,
    gen_block_id: Counter<BlockId>,
    gen_register_id: Arc<Counter<RegisterId>>,
    entrypoint: Option<BlockId>,
    blocks: FxHashMap<BlockId, FunctionBlock>,
    /// To prevent mistakes, if the user does not call `end_function` after
    /// having declared a builder with `start_function`, then this field will
    /// remain `false`. If, when being dropped, this field is `false`, a panic
    /// will be issued to tell the user to make the appropriate call to
    /// `end_function`. Ideally, this would be done at compile time with ownership
    /// principles, but any design (that I can come up with) that uses mutability
    /// would prevent the builder from being used in a multithreaded environment.
    is_ok_to_drop: bool,
}

impl<'n, const P: usize> FunctionBuilder<'n, P> {
    fn new(id: FunctionId, name: &'n str) -> Self {
        Self {
            id,
            name,
            gen_block_id: Counter::new(),
            gen_register_id: Arc::new(Counter::new_with_value(P)),
            entrypoint: None,
            blocks: FxHashMap::default(),
            is_ok_to_drop: false,
        }
    }

    fn finish(self) -> Function {
        Function {
            name: DebugName::new(self.name),
            parameters: (0..P)
                .into_iter()
                .map(|p| Parameter {
                    name: DebugName::none(),
                    register: RegisterId::new_with_value_const(p),
                })
                .collect(),
            entry_block: self.entrypoint.expect("expected entry block"),
            // TODO: find some safer way to move out data
            blocks: self.blocks.clone(),
        }
    }

    pub const fn parameter_const<const PARAMETER: usize>(&self) -> RegisterId {
        // TODO: make this a constant assertion
        // this is currently not possible because use of generic parameters from
        // outer functions (in this case, the `T` in `impl<T>`) is illegal
        // (so we can't use `P` in a constant context)
        assert!(
            PARAMETER < P,
            "constant parameter must be less than total amount of parameters"
        );

        RegisterId::new_with_value_const(PARAMETER)
    }

    pub fn parameter(&self, parameter: usize) -> RegisterId {
        assert!(
            parameter < P,
            "parameter argument must be less than total amount of parameters"
        );

        RegisterId::new_with_value(parameter)
    }

    pub fn signature(&self) -> FnSignature<P> {
        FnSignature { id: self.id }
    }

    pub fn start_block_main(&mut self) -> BlockBuilder<0> {
        assert!(
            matches!(self.entrypoint, None),
            "can only define one entrypointt block"
        );

        let (builder, []) = self.start_block();
        self.entrypoint = Some(builder.id);

        builder
    }

    pub fn start_block<const PARAMETERS: usize>(
        &mut self,
    ) -> (BlockBuilder<PARAMETERS>, [RegisterId; PARAMETERS]) {
        let id = self.gen_block_id.next();

        // TODO: is there a better way to do this?
        // PARAMETERS = 3, parameters = [RegisterId::new_with_value_const(0), ...(1), ...(2)]
        let mut parameters = [RegisterId::new_const(); PARAMETERS];
        for parameter in parameters.iter_mut() {
            *parameter = self.gen_register_id.next();
        }

        let builder = BlockBuilder::new(id, self.gen_register_id.clone(), parameters.clone());

        (builder, parameters)
    }

    pub fn end_block<const PARAMETERS: usize>(
        &mut self,
        mut builder: FinalizedBlockBuilder<PARAMETERS>,
    ) -> BlkSignature<PARAMETERS> {
        let signature = builder.builder.signature();

        builder.is_ok_to_drop = true;
        let block = builder.finish();

        self.blocks.insert(signature.id, block);

        signature
    }
}

impl<const P: usize> Drop for FunctionBuilder<'_, P> {
    fn drop(&mut self) {
        if !self.is_ok_to_drop {
            panic!("A `FunctionBuilder` (created with `start_function`) was dropped without `end_function` being called.");
        }
    }
}

pub struct FnSignature<const PARAMETERS: usize> {
    pub id: FunctionId,
}

pub struct BlkSignature<const PARAMETERS: usize> {
    pub id: BlockId,
}

#[derive(Clone)]
pub struct BlockBuilder<const PARAMETERS: usize> {
    pub id: BlockId,
    gen_register_id: Arc<Counter<RegisterId>>,
    parameters: [RegisterId; PARAMETERS],
    instructions: Vec<Instruction>,
    is_ok_to_drop: bool,
}

impl<const P: usize> BlockBuilder<P> {
    fn new(
        id: BlockId,
        gen_register_id: Arc<Counter<RegisterId>>,
        parameters: [RegisterId; P],
    ) -> Self {
        Self {
            id,
            gen_register_id,
            parameters,
            instructions: vec![],
            is_ok_to_drop: false,
        }
    }

    pub fn get_runtime(&mut self) -> RegisterId {
        let register = self.gen_register_id.next();

        self.instructions.push(Instruction::GetRuntime(register));

        register
    }

    pub fn make_string(&mut self, constant_id: ConstantId) -> RegisterId {
        let result = self.gen_register_id.next();
        self.instructions
            .push(Instruction::MakeString(result, constant_id));
        result
    }

    pub fn make_number_decimal(&mut self, value: f64) -> RegisterId {
        let result = self.gen_register_id.next();
        self.instructions
            .push(Instruction::MakeNumber(result, value));
        result
    }

    pub fn add(&mut self, lhs: RegisterId, rhs: RegisterId) -> RegisterId {
        let result = self.gen_register_id.next();
        self.instructions.push(Instruction::Add(result, lhs, rhs));
        result
    }

    pub fn compare_less_than(&mut self, lhs: RegisterId, rhs: RegisterId) -> RegisterId {
        let result = self.gen_register_id.next();
        self.instructions
            .push(Instruction::CompareLessThan(result, lhs, rhs));
        result
    }

    pub fn call<const PARAMETERS: usize>(
        &mut self,
        function_signature: FnSignature<PARAMETERS>,
        values: [RegisterId; PARAMETERS],
    ) {
        self.call_dynargs(function_signature.id, values.to_vec())
    }

    pub fn call_dynargs(&mut self, function_id: FunctionId, values: Vec<RegisterId>) {
        self.instructions.push(Instruction::Call(
            None,
            Callable::Static(function_id),
            values,
        ));
    }

    pub fn call_with_result<const PARAMETERS: usize>(
        &mut self,
        function_signature: FnSignature<PARAMETERS>,
        values: [RegisterId; PARAMETERS],
    ) -> RegisterId {
        self.call_dynargs_with_result(function_signature.id, values.to_vec())
    }

    pub fn call_dynargs_with_result(
        &mut self,
        function_id: FunctionId,
        values: Vec<RegisterId>,
    ) -> RegisterId {
        let result = self.gen_register_id.next();

        self.instructions.push(Instruction::Call(
            Some(result),
            Callable::Static(function_id),
            values,
        ));

        result
    }

    pub fn call_external_function<const PARAMETERS: usize>(
        &mut self,
        external_function: ExtFnIdTyped<PARAMETERS>,
        values: [RegisterId; PARAMETERS],
    ) {
        self.call_external_function_dynargs(external_function.0, values.to_vec())
    }

    pub fn call_external_function_dynargs(
        &mut self,
        external_function: ExternalFunctionId,
        values: Vec<RegisterId>,
    ) {
        self.instructions.push(Instruction::Call(
            None,
            Callable::External(external_function),
            values,
        ));
    }

    pub fn call_external_function_with_result<const PARAMETERS: usize>(
        &mut self,
        external_function: ExtFnIdTyped<PARAMETERS>,
        values: [RegisterId; PARAMETERS],
    ) -> RegisterId {
        self.call_external_function_dynargs_with_result(external_function.0, values.to_vec())
    }

    pub fn call_external_function_dynargs_with_result(
        &mut self,
        external_function: ExternalFunctionId,
        values: Vec<RegisterId>,
    ) -> RegisterId {
        let result = self.gen_register_id.next();

        self.instructions.push(Instruction::Call(
            Some(result),
            Callable::External(external_function),
            values,
        ));

        result
    }

    pub fn jmp<const PARAMETERS: usize>(
        self,
        block: BlkSignature<PARAMETERS>,
        values: [RegisterId; PARAMETERS],
    ) -> FinalizedBlockBuilder<P> {
        self.jmp_dynargs(block.id, values.to_vec())
    }

    pub fn jmp_dynargs(
        mut self,
        block: BlockId,
        values: Vec<RegisterId>,
    ) -> FinalizedBlockBuilder<P> {
        self.is_ok_to_drop = true;
        FinalizedBlockBuilder {
            builder: self,
            is_ok_to_drop: false,
            end_control_flow: ControlFlowInstruction::Jmp(BasicBlockJump(block, values)),
        }
    }

    pub fn ret(mut self, value: Option<RegisterId>) -> FinalizedBlockBuilder<P> {
        self.is_ok_to_drop = true;
        FinalizedBlockBuilder {
            builder: self,
            is_ok_to_drop: false,
            end_control_flow: ControlFlowInstruction::Ret(value),
        }
    }

    pub fn jmpif<const PARAMS_TRUE: usize, const PARAMS_FALSE: usize>(
        self,
        condition: RegisterId,
        block_true: BlkSignature<PARAMS_TRUE>,
        values_true: [RegisterId; PARAMS_TRUE],
        block_false: BlkSignature<PARAMS_FALSE>,
        values_false: [RegisterId; PARAMS_FALSE],
    ) -> FinalizedBlockBuilder<P> {
        self.jmpif_dynargs(
            condition,
            block_true.id,
            values_true.to_vec(),
            block_false.id,
            values_false.to_vec(),
        )
    }

    pub fn jmpif_dynargs(
        mut self,
        condition: RegisterId,
        block_true: BlockId,
        values_true: Vec<RegisterId>,
        block_false: BlockId,
        values_false: Vec<RegisterId>,
    ) -> FinalizedBlockBuilder<P> {
        self.is_ok_to_drop = true;
        FinalizedBlockBuilder {
            builder: self,
            is_ok_to_drop: false,
            end_control_flow: ControlFlowInstruction::JmpIf {
                condition,
                true_path: BasicBlockJump(block_true, values_true),
                false_path: BasicBlockJump(block_false, values_false),
            },
        }
    }

    pub fn signature(&self) -> BlkSignature<P> {
        BlkSignature { id: self.id }
    }
}

impl<const P: usize> Drop for BlockBuilder<P> {
    fn drop(&mut self) {
        if !self.is_ok_to_drop {
            panic!("A `BlockBuilder` (created with `start_block`) was dropped without a finalizing method (e.g. `ret`) being called.");
        }
    }
}

pub struct FinalizedBlockBuilder<const PARAMETERS: usize> {
    builder: BlockBuilder<PARAMETERS>,
    end_control_flow: ControlFlowInstruction,
    is_ok_to_drop: bool,
}

impl<const P: usize> FinalizedBlockBuilder<P> {
    fn finish(mut self) -> FunctionBlock {
        self.is_ok_to_drop = true;

        // TODO: find a faster way to do this (but needs to still be safe)
        let parameters = self.builder.parameters.clone();
        let instructions = self.builder.instructions.clone();
        let end = self.end_control_flow.clone();
        FunctionBlock {
            parameters: parameters.to_vec(),
            instructions,
            end,
        }
    }
}

impl<const P: usize> Drop for FinalizedBlockBuilder<P> {
    fn drop(&mut self) {
        if !self.is_ok_to_drop {
            panic!("A `FinalizedBlockBuilder` (created with a finalizing method) was dropped without `end_block` being called.");
        }
    }
}
