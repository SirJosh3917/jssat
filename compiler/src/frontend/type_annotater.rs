use ref_cast::RefCast;
use std::collections::VecDeque;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::frontend::ir::*;
use crate::name::DebugName;
use crate::{id::*, UnwrapNone};

use super::conv_only_bb::{Block, PureBlocks};

/// Type annotation mechanism in JSSAT.
///
/// This works by symbolically executing the JSSAT IR, and emitting equivalent functions.
// can't use references because we need them to live 'static for tokio::spawn to work
pub fn annotate(ir: &IR, pure_blocks: PureBlocks) -> SymbolicEngine {
    let mut entrypoints = FxHashMap::default();
    for (id, func) in ir.functions.iter() {
        entrypoints.insert(*id, func.entry_block).expect_free();
    }

    let ir_entry_block_id = ir.functions.get(&ir.entrypoint).unwrap().entry_block;
    let entry_block_id = pure_blocks.get_block_id_by_host(ir.entrypoint, ir_entry_block_id);

    let symb_exec_eng =
        SymbolicEngineToken::new(pure_blocks, entrypoints, ir.external_functions.clone());

    let explore_req = symb_exec_eng.clone().explore_fn(BlockExecutionKey {
        id: entry_block_id,
        parameters: vec![],
    });

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(explore_req);
    drop(rt);

    let mutex = Arc::try_unwrap(symb_exec_eng.0).expect("nothing should be using the mutex");
    Mutex::into_inner(mutex)
}

#[derive(Clone)]
struct SymbolicEngineToken(Arc<Mutex<SymbolicEngine>>);

#[derive(Debug)]
pub struct SymbolicEngine {
    pub blocks: Arc<RwLock<PureBlocks>>,
    pub entrypoints: FxHashMap<FunctionId<IrCtx>, BlockId<IrCtx>>,
    pub executions: Executions,
    pub ext_fns: FxHashMap<ExternalFunctionId<IrCtx>, ExternalFunction>,
    pub typed_blocks: FxHashMap<BlockKey, TypedFunction>,
    pub new_fn_ids: Counter<FunctionId<AnnotatedCtx>>,
}

#[derive(Debug)]
pub struct Executions {
    // mapping of ORIGINAL fn id + block id to NEW fn id + block id
    executions: FxHashMap<BlockId<PureBbCtx>, Vec<(Vec<ValueType>, BlockExecution)>>,
}

impl Executions {
    pub fn new() -> Self {
        Self {
            executions: Default::default(),
        }
    }

    pub fn get(&self, key: &BlockExecutionKey) -> Option<&BlockExecution> {
        self.executions.get(&key.id).and_then(|blocks| {
            blocks
                .iter()
                .filter(|(p, _)| p == &key.parameters)
                .map(|(_, block)| block)
                .next()
        })
    }

    pub fn insert(&mut self, key: BlockExecutionKey, execution: BlockExecution) {
        let executions = (self.executions)
            .entry(key.id)
            .or_insert_with(|| Vec::with_capacity(1));

        for (params, exec) in executions.iter_mut() {
            if params == &key.parameters {
                *exec = execution;
                return;
            }
        }

        executions.push((key.parameters, execution));
    }

    pub fn all_fn_invocations(
        &self,
    ) -> impl Iterator<Item = (BlockId<PureBbCtx>, &Vec<ValueType>, &BlockExecution)> {
        self.executions
            .iter()
            .flat_map(|(k, v)| v.iter().map(move |e| (k, e)))
            .map(|(blk, (args, cf))| (*blk, args, cf))
    }

    pub fn into_all_fn_invocations(
        self,
    ) -> impl Iterator<Item = (BlockId<PureBbCtx>, Vec<ValueType>, BlockExecution)> {
        self.executions
            .into_iter()
            .flat_map(|(k, v)| v.into_iter().map(move |e| (k, e)))
            .map(|(blk, (args, cf))| (blk, args, cf))
    }
}

impl Default for Executions {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct BlockKey {
    pub function: FunctionId<AnnotatedCtx>,
    pub block: BlockId<AnnotatedCtx>,
}

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub struct BlockExecutionKey {
    pub id: BlockId<PureBbCtx>,
    pub parameters: Vec<ValueType>,
}

#[derive(Debug)]
pub enum BlockExecution {
    InProgress(BlockKey),
    Finished(BlockKey),
}

impl BlockExecution {
    pub fn key(&self) -> BlockKey {
        match self {
            BlockExecution::InProgress(a) | BlockExecution::Finished(a) => *a,
        }
    }
}

#[derive(Debug)]
pub struct TypedFunction {
    pub return_type: ReturnType,
    pub eval_blocks: Vec<(
        BlockId<PureBbCtx>,
        Vec<ValueType>,
        ExplorationBranch,
        FxHashMap<RegisterId<PureBbCtx>, ValueType>,
    )>,
}

impl TypedFunction {
    pub fn find(
        &self,
        block: &BlockId<PureBbCtx>,
        args: &[ValueType],
    ) -> (
        &ExplorationBranch,
        &FxHashMap<RegisterId<PureBbCtx>, ValueType>,
    ) {
        (self.eval_blocks.iter())
            .filter(|(blk, blk_args, _, _)| blk == block && blk_args == args)
            .map(|(_, _, branch, map)| (branch, map))
            .next()
            .unwrap()
    }
}

impl SymbolicEngineToken {
    fn new(
        blocks: PureBlocks,
        entrypoints: FxHashMap<FunctionId<IrCtx>, BlockId<IrCtx>>,
        ext_fns: FxHashMap<ExternalFunctionId<IrCtx>, ExternalFunction>,
    ) -> Self {
        Self(Arc::new(Mutex::new(SymbolicEngine {
            blocks: Arc::new(RwLock::new(blocks)),
            entrypoints,
            ext_fns,
            executions: Executions::new(),
            typed_blocks: FxHashMap::default(),
            new_fn_ids: Counter::new(),
        })))
    }

    fn explore_fn_later(&self, block: BlockExecutionKey) -> JoinHandle<ReturnType> {
        let me = self.clone();
        tokio::task::spawn(me.explore_fn(block))
    }

    async fn explore_fn(self, block: BlockExecutionKey) -> ReturnType {
        let mut me = (self.0.try_lock()).expect("Lock should be contentionless");

        // first, check if we've already executed this block with the values present
        let key = match me.executions.get(&block) {
            // we've invoked this function and computed it fully before
            Some(BlockExecution::Finished(re)) => {
                return me.typed_blocks.get(re).unwrap().return_type.clone();
            }
            // if we're in progress of executing this exact function, that means
            // we've taken on such a path that calls the same exact function.
            // by returning `Never`, we display this recursiveness - this path
            // is one that would never end. If there are other paths in the
            // function, this return type will get more accurate as the other
            // paths are combined.
            Some(BlockExecution::InProgress(_)) => return ReturnType::Never,
            None => {
                let block_key = BlockKey {
                    function: me.new_fn_ids.next(),
                    block: BlockId::new(),
                };

                me.executions
                    .insert(block.clone(), BlockExecution::InProgress(block_key));

                block_key
            }
        };

        drop(me);

        // we choose never as `Never union T = T`, making it the most unifyable default
        let mut return_type = ReturnType::Never;
        let mut eval_blocks = Vec::new();

        let mut block_stack = VecDeque::new();
        block_stack.push_back(block.clone());

        while let Some(exec_key) = block_stack.pop_front() {
            let has_evaled_block = {
                eval_blocks.iter().any(|(block, keys, _, _)| {
                    *block == exec_key.id && keys == &exec_key.parameters
                })
            };

            if has_evaled_block {
                continue;
            }

            let block = exec_key.id;
            let params = exec_key.parameters.clone();

            let Exploration {
                control_flow,
                types,
            } = self.explore_block(exec_key).await;

            match &control_flow {
                ExplorationBranch::Branch(next) => {
                    block_stack.extend(next.clone());
                }
                ExplorationBranch::Complete(ret_type) => return_type.unify(ret_type.clone()),
            };

            eval_blocks.push((block, params, control_flow, types));
        }

        let typed = TypedFunction {
            return_type,
            eval_blocks,
        };

        let mut me = (self.0.try_lock()).expect("Lock should be contentionless");

        me.executions
            .insert(block.clone(), BlockExecution::Finished(key));
        me.typed_blocks.insert(key, typed);
        me.typed_blocks.get(&key).unwrap().return_type.clone()
    }

    async fn explore_block(&self, key: BlockExecutionKey) -> Exploration {
        let mut me = (self.0.try_lock()).expect("Lock should be contentionless");

        let blocks = me.blocks.clone();
        let blocks = blocks.try_read().expect("Blocks should be contentionless");
        let block = blocks.get_block(key.id);

        let mut types = FxHashMap::default();

        debug_assert_eq!(block.parameters.len(), key.parameters.len(), "at {:?}", key);
        for (parameter, r#type) in block.parameters.iter().zip(key.parameters.iter()) {
            types.insert(*parameter, r#type.clone());
        }

        for inst in block.instructions.iter() {
            let map = |r: &RegisterId<PureBbCtx>| types.get(r).unwrap();

            match inst {
                Instruction::GetRuntime(reg) => {
                    types.insert(*reg, ValueType::Runtime);
                }
                Instruction::MakeString(reg, str) => {
                    // TODO: solve MakeString issue
                    types.insert(*reg, ValueType::ExactString(str.map_context::<IrCtx>()));
                }
                Instruction::MakeInteger(reg, value) => {
                    types.insert(*reg, ValueType::ExactInteger(*value));
                }
                Instruction::CompareLessThan(reg, lhs, rhs) => {
                    let comparison = match (map(lhs).is_comparable(), map(rhs).is_comparable()) {
                        (None, _) => unimplemented!("LHS is uncomparable"),
                        (_, None) => unimplemented!("RHS is uncomparable"),
                        (Some(lhs), Some(rhs)) => lhs.perform_less_than(&rhs),
                    };

                    types.insert(*reg, comparison.into_value_type());
                }
                Instruction::Add(reg, lhs, rhs) => {
                    let addition = match (map(lhs).is_addable(), map(rhs).is_addable()) {
                        (_, None) => unimplemented!("LHS is uncomparable"),
                        (None, _) => unimplemented!("RHS is uncomparable"),
                        (Some(a), Some(b)) => a.perform_add(&b).expect("should be able to add"),
                    };

                    types.insert(*reg, addition.into_value_type());
                }
                Instruction::Call(result, func, arg_regs) => {
                    let args = arg_regs.iter().map(map).cloned().collect::<Vec<_>>();

                    match func {
                        Callable::External(id) if result.is_some() => {
                            let ext_fn = me.ext_fns.get(id).unwrap();

                            let ret_type = match &ext_fn.return_type {
                                FFIReturnType::Void => todo!(
                                    "figure out what to do with void return type being assigned"
                                ),
                                FFIReturnType::Value(v) => ffi_value_type_to_value_type(v),
                            };

                            types.insert(result.unwrap(), ret_type);

                            debug_assert_eq!(args.len(), ext_fn.parameters.len());
                            for (arg_typ, ffi_typ) in args.iter().zip(ext_fn.parameters.iter()) {
                                assert!(FFICoerce::can_coerce(arg_typ, ffi_typ));
                            }
                        }
                        Callable::External(id) => {
                            // we don't care about the return type, so /shrug
                            // TODO: dedup code?
                            let ext_fn = me.ext_fns.get(id).unwrap();

                            debug_assert_eq!(args.len(), ext_fn.parameters.len());
                            for (arg_typ, ffi_typ) in args.iter().zip(ext_fn.parameters.iter()) {
                                assert!(
                                    FFICoerce::can_coerce(arg_typ, ffi_typ),
                                    "cannot coerce {:?} to {:?} on {:?}",
                                    arg_typ,
                                    ffi_typ,
                                    inst
                                );
                            }
                        }
                        Callable::Static(id) => {
                            let entrypoint = *me.entrypoints.get(id).unwrap();
                            let pure_bb_id = blocks.get_block_id_by_host(*id, entrypoint);

                            drop(me);

                            let key = BlockExecutionKey {
                                id: pure_bb_id,
                                parameters: args,
                            };

                            let ret_type = self
                                .explore_fn_later(key)
                                .await
                                .expect("couldnt explore function call");

                            match ret_type {
                                ReturnType::Void if result.is_none() => {}
                                ReturnType::Void => todo!("figure out what to do"),
                                ReturnType::Value(value) if matches!(result, Some(_)) => {
                                    types.insert(result.unwrap(), value);
                                }
                                ReturnType::Value(_) => {
                                    // we computed the function return type but it's never used
                                    // /shrug
                                }
                                ReturnType::Never => todo!("return never"),
                            };

                            me = (self.0.try_lock()).expect("Lock should be contentionless");
                        }
                    };
                }
            }
        }

        let map_bsc_blk_jmp = |BasicBlockJump(jmp_block, args)| BlockExecutionKey {
            id: jmp_block,
            parameters: args
                .into_iter()
                .map(|r| {
                    types
                        .get(&r)
                        .unwrap_or_else(|| panic!("in {:?} -> {:?}({:?})", &key, &jmp_block, r))
                        .clone()
                })
                .collect(),
        };

        let control_flow = match &block.end {
            ControlFlowInstruction::Jmp(target) => {
                ExplorationBranch::Branch(vec![map_bsc_blk_jmp(target.clone())])
            }
            ControlFlowInstruction::JmpIf {
                condition,
                true_path,
                false_path,
            } => {
                let cond = types.get(condition).unwrap();

                ExplorationBranch::Branch(match cond {
                    ValueType::Bool(true) => vec![map_bsc_blk_jmp(true_path.clone())],
                    ValueType::Bool(false) => vec![map_bsc_blk_jmp(false_path.clone())],
                    ValueType::Boolean => vec![
                        map_bsc_blk_jmp(true_path.clone()),
                        map_bsc_blk_jmp(false_path.clone()),
                    ],
                    _ => unimplemented!("cannot operate on condition {:?}", cond),
                })
            }
            ControlFlowInstruction::Ret(Some(reg)) => {
                ExplorationBranch::Complete(ReturnType::Value(types.get(reg).unwrap().clone()))
            }
            ControlFlowInstruction::Ret(None) => ExplorationBranch::Complete(ReturnType::Void),
        };

        Exploration {
            control_flow,
            types,
        }
    }
}

struct Exploration {
    control_flow: ExplorationBranch,
    types: FxHashMap<RegisterId<PureBbCtx>, ValueType>,
}

#[derive(Debug)]
pub enum ExplorationBranch {
    Branch(Vec<BlockExecutionKey>),
    Complete(ReturnType),
}

#[derive(Debug)]
pub struct Parameter {
    pub name: DebugName,
    pub register: RegisterId<NoContext>,
    pub r#type: ValueType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReturnType {
    Void,
    Value(ValueType),
    /// # [`ValueType::Never`]
    ///
    /// The type assigned to a function when it recurses to infinity, with no
    /// end in sight.
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValueType {
    /// # `Any`
    ///
    /// The `Any` type in JSSAT is used a a polymorphic "catch-all" for when
    /// the type system cannot figure something out.
    ///
    /// Narrowing an `Any` into a more specific type when it's not possible to
    /// do so results in runtime errors. This feature of the `Any` type allows
    /// us to compile all user provided code into an output, even if the code
    /// given should be considered a compiler error.
    ///
    /// The `Any` type is the most generic type possible for all values. Any
    /// JSSAT RT value can be cast into an `Any`, besides exotic primitives,
    // TODO: is `Reference`/`Pointer` the finalized name?
    /// such as a `Runtime` or `Reference`/`Pointer`.
    ///
    /// A hierarchy of JSSAT RT types is shown below:
    ///
    /// - [`ValueType::Any`]
    ///   - [`ValueType::String`]
    ///     - [`ValueType::ExactString`]
    Any,
    Runtime,
    String,
    // TODO: an "ExactString" should just be a String with some kind of
    // ExactnessGuarantee to be exactly a type of a constant
    ExactString(ConstantId<IrCtx>),
    Number,
    ExactInteger(i64),
    Boolean,
    Bool(bool),
    /// Pointer to data of the specified size. Pointer(16) -> `i16*`.
    Pointer(u16),
    Word,
}

pub fn ffi_value_type_to_value_type(ffi_value_type: &FFIValueType) -> ValueType {
    match ffi_value_type {
        FFIValueType::Any => ValueType::Any,
        FFIValueType::Runtime => ValueType::Runtime,
        FFIValueType::Pointer(size) => ValueType::Pointer(*size),
        FFIValueType::Word => ValueType::Word,
        FFIValueType::String => ValueType::String,
    }
}

impl ValueType {
    pub fn is_comparable(&self) -> Option<ValueComparable> {
        match self {
            // TODO: *is* an `Any` comparable? or should we force the user to unwrap it
            ValueType::Any => todo!(),
            ValueType::Number => Some(ValueComparable::Number),
            ValueType::ExactInteger(value) => Some(ValueComparable::Num(*value)),
            _ => None,
        }
    }

    pub fn is_addable(&self) -> Option<ValueAddable> {
        match self {
            ValueType::ExactString(_) => todo!(),
            ValueType::String => Some(ValueAddable::String),
            ValueType::Number => Some(ValueAddable::Number),
            ValueType::ExactInteger(n) => Some(ValueAddable::Num(*n)),
            _ => None,
        }
    }
}

pub enum ValueAddable {
    Number,
    Num(i64),
    String,
    // Str(Vec<u8>),
}

#[derive(Debug, Error)]
pub enum AdditionError {
    #[error("the types are incompatible")]
    IncompatibleTypes,
}

impl ValueAddable {
    pub fn perform_add(&self, other: &ValueAddable) -> Result<ValueAddable, AdditionError> {
        Ok(match (self, other) {
            (ValueAddable::Number, ValueAddable::Number) => ValueAddable::Number,
            (ValueAddable::Number, ValueAddable::Num(_)) => ValueAddable::Number,
            (ValueAddable::Num(_), ValueAddable::Number) => ValueAddable::Number,
            (ValueAddable::Num(a), ValueAddable::Num(b)) => ValueAddable::Num(*a + *b),
            (ValueAddable::String, ValueAddable::String) => ValueAddable::String,
            (ValueAddable::Number, ValueAddable::String)
            | (ValueAddable::Num(_), ValueAddable::String)
            | (ValueAddable::String, ValueAddable::Number)
            | (ValueAddable::String, ValueAddable::Num(_)) => {
                return Err(AdditionError::IncompatibleTypes)
            }
        })
    }
}

pub enum ValueComparable {
    Number,
    Num(i64),
}

impl ValueComparable {
    pub fn perform_less_than(&self, other: &ValueComparable) -> ValueConditional {
        match (self, other) {
            // TODO: use guarantees (e.g. "x < y") to make more informed decisions
            (ValueComparable::Number, _) | (_, ValueComparable::Number) => {
                ValueConditional::Boolean
            }
            (ValueComparable::Num(lhs), ValueComparable::Num(rhs)) => {
                ValueConditional::Bool(lhs < rhs)
            }
        }
    }
}

pub trait IntoValueType {
    fn into_value_type(self) -> ValueType;
}

impl IntoValueType for ValueConditional {
    fn into_value_type(self) -> ValueType {
        match self {
            ValueConditional::Boolean => ValueType::Boolean,
            ValueConditional::Bool(b) => ValueType::Bool(b),
        }
    }
}

impl IntoValueType for ValueAddable {
    fn into_value_type(self) -> ValueType {
        match self {
            ValueAddable::Number => ValueType::Number,
            ValueAddable::Num(n) => ValueType::ExactInteger(n),
            ValueAddable::String => ValueType::String,
        }
    }
}

pub enum ValueConditional {
    Boolean,
    Bool(bool),
}

struct FFICoerce;

impl FFICoerce {
    // TODO: figure out how to merge "can_coerce" and "do_coerce" into one, so
    // that in order for something to be coercible there must be a conversion
    // routine that can handle it - all enforced at compile time
    pub fn can_coerce(value_type: &ValueType, ffi: &FFIValueType) -> bool {
        match (ffi, value_type) {
            (FFIValueType::Any, ValueType::Any)
            | (FFIValueType::Any, ValueType::String)
            | (FFIValueType::Any, ValueType::ExactString(_))
            | (FFIValueType::Any, ValueType::Number)
            | (FFIValueType::Any, ValueType::ExactInteger(_))
            // | (FFIValueType::Any, ValueType::Word)
            | (FFIValueType::Any, ValueType::Boolean)
            | (FFIValueType::Any, ValueType::Bool(_))
            | (FFIValueType::Runtime, ValueType::Runtime)
            // | (FFIValueType::Word, ValueType::Number)
            // | (FFIValueType::Word, ValueType::ExactNumber(_))
            | (FFIValueType::Word, ValueType::Word)
            | (FFIValueType::String, ValueType::String)
            | (FFIValueType::String, ValueType::ExactString(_))
            => true,
            (FFIValueType::Pointer(p1), ValueType::Pointer(p2)) if p1 == p2 => true,
            (_, _) => false
        }
    }
}

impl ReturnType {
    fn unify(&mut self, other: ReturnType) {
        match (&self, other) {
            (ReturnType::Value(_), ReturnType::Value(_)) => todo!("unify 2 values"),
            (a, b) if *a == &b => {
                // do nothing, as both types are the same
            }
            (ReturnType::Never, other) => {
                *self = other;
            }
            (_, ReturnType::Never) => {}
            (ReturnType::Void, ReturnType::Void) => {}
            (ReturnType::Void, ReturnType::Value(_)) => todo!(),
            (ReturnType::Value(_), ReturnType::Void) => todo!(),
        }
    }
}

// ===
// im so good at writing clean code
// ===

#[derive(RefCast, Hash, PartialEq, Eq)]
#[repr(transparent)]
pub struct BlockInvocationArgs(pub Vec<ValueType>);

pub struct AnnotatedBlockInformation {
    pub args: Vec<ValueType>,
    pub registers: FxHashMap<RegisterId<PureBbCtx>, ValueType>,
}

pub struct AnnotatedBlock<'block> {
    pub block: &'block crate::frontend::conv_only_bb::Block,
    pub args: &'block Vec<ValueType>,
    pub registers: &'block FxHashMap<RegisterId<PureBbCtx>, ValueType>,
}

struct AnnotatedBlockTag(BlockId<PureBbCtx>, usize);

pub struct PureAnnotatedBlocks {
    pub blocks: PureBlocks,
    invocations: Vec<AnnotatedBlockInformation>,
    id_map: FxHashMap<BlockId<AnnotatedCtx>, AnnotatedBlockTag>,
    invocation_map:
        FxHashMap<BlockId<PureBbCtx>, FxHashMap<BlockInvocationArgs, BlockId<AnnotatedCtx>>>,
    return_types: FxHashMap<BlockId<AnnotatedCtx>, ReturnType>,
}

impl SymbolicEngine {
    pub fn extract(mut self) -> PureAnnotatedBlocks {
        let blocks = Arc::try_unwrap(self.blocks).expect("nothing should be using the arc");
        let blocks = RwLock::into_inner(blocks);

        let executions = self.executions;

        let annotated_block_id_gen = Counter::new();
        let mut invocations = Vec::new();
        let mut id_map = FxHashMap::default();
        let mut invocation_map = FxHashMap::default();
        let mut return_types = FxHashMap::default();

        for (pure_block_id, invocation_args, evaluation) in executions.into_all_fn_invocations() {
            let typed_fn = self.typed_blocks.get(&evaluation.key()).unwrap();

            let mut blocks_and_args = vec![(pure_block_id, invocation_args)];
            for (a, b, c, d) in typed_fn.eval_blocks.iter() {
                blocks_and_args.push((*a, b.clone()));
            }

            for (pure_block_id, invocation_args) in blocks_and_args {
                let annotated_blk_id = annotated_block_id_gen.next();

                return_types.insert(annotated_blk_id, typed_fn.return_type.clone());

                let mut registers = None;
                for (a, b, _, d) in typed_fn.eval_blocks.iter() {
                    if *a == pure_block_id && b == &invocation_args {
                        registers = Some(d);
                        break;
                    }
                }
                let registers = registers.unwrap().clone();

                invocation_map
                    .entry(pure_block_id)
                    .or_insert_with(FxHashMap::default)
                    .entry(BlockInvocationArgs(invocation_args.clone()))
                    .insert(annotated_blk_id);

                let invocation_idx = invocations.len();
                invocations.push(AnnotatedBlockInformation {
                    args: invocation_args,
                    registers,
                });

                id_map.insert(
                    annotated_blk_id,
                    AnnotatedBlockTag(pure_block_id, invocation_idx),
                );
            }
        }

        PureAnnotatedBlocks {
            blocks,
            invocations,
            id_map,
            invocation_map,
            return_types,
        }
    }
}

impl PureAnnotatedBlocks {
    pub fn get_block_id(
        &self,
        block_id: BlockId<PureBbCtx>,
        invocation_args: &Vec<ValueType>,
    ) -> BlockId<AnnotatedCtx> {
        let map = self.invocation_map.get(&block_id).unwrap();

        *map.get(BlockInvocationArgs::ref_cast(invocation_args))
            .unwrap()
    }

    pub fn get_block(&self, block_id: BlockId<AnnotatedCtx>) -> AnnotatedBlock {
        let &AnnotatedBlockTag(block_id, invocation_idx) = self.id_map.get(&block_id).unwrap();
        let block = self.blocks.get_block(block_id);
        let annotated_info = &self.invocations[invocation_idx];

        AnnotatedBlock {
            block,
            args: &annotated_info.args,
            registers: &annotated_info.registers,
        }
    }

    pub fn get_return_type(&self, block_id: BlockId<AnnotatedCtx>) -> &ReturnType {
        self.return_types.get(&block_id).unwrap()
    }
}
