use std::array::IntoIter;

use rustc_hash::FxHashMap;

use crate::{
    frontend::builder::{DynBlockBuilder, FnSignature, ProgramBuilder, RegisterId},
    isa::InternalSlot,
    UnwrapNone,
};

use super::parse_nodes::{self as js, Visitor};

// TODO(refactor): maybe separate this out somewhere else? it's simpler being
//   in one place though
fn emit_virt_overrides(
    kind: js::ParseNodeKind,
    idx: usize,
    block: &mut DynBlockBuilder,
    slot: InternalSlot,
) -> bool {
    match (kind, idx) {
        _ => return false,
    };

    true
}

pub struct NodeEmitter<'block> {
    block: &'block mut DynBlockBuilder,
    stack: Vec<ParseNode>,
    pub last_completed: Option<ParseNode>,
    simple_fns: FxHashMap<InternalSlot, FnSignature<1>>,
}

impl<'b> NodeEmitter<'b> {
    pub fn new(block: &'b mut DynBlockBuilder, program: &mut ProgramBuilder) -> Self {
        let simple_fns = Self::generate_simple_fns(program);

        Self {
            block,
            stack: Vec::new(),
            last_completed: None,
            simple_fns,
        }
    }

    /// Generates the "simple" ECMAScript functions - the functions that are
    /// defined by the following catch-most clause:
    ///
    /// > For example, assume that some algorithm has a step of the form:
    /// > “Return the result of evaluating Block” and that there is a
    /// > production:
    /// >
    /// > ```text
    /// > Block :
    /// >     { StatementList }
    /// > ```
    /// >
    /// > but the *Evaluation* operation does not associate an algorithm with
    /// > that production. In that case, the *Evaluation* operation
    /// > implicitly includes > an association of the form:
    /// >
    /// > > Runtime Semantics:
    /// > >
    /// > > ```text
    /// > > EvaluationBlock :
    /// > >     { StatementList }
    /// > > ```
    /// > > 1.  Return the result of evaluating StatementList.
    fn generate_simple_fns(
        program: &mut ProgramBuilder,
    ) -> FxHashMap<InternalSlot, FnSignature<1>> {
        let mut map = FxHashMap::default();

        for slot in IntoIter::new(RUNTIME_SEMANTICS) {
            let (mut f, [x]) = program.start_function();

            let mut e = f.start_block_main();
            let fn_ptr = e.record_get_slot(x, slot);
            let result = e.call_virt_with_result(fn_ptr, [x]);
            f.end_block(e.ret(Some(result)));

            let signature = program.end_function(f);

            map.insert(slot, signature).expect_free();
        }

        map
    }
}

const PARSE_NODE_SLOTS: [InternalSlot; 6] = [
    InternalSlot::JSSATParseNodeSlot1,
    InternalSlot::JSSATParseNodeSlot2,
    InternalSlot::JSSATParseNodeSlot3,
    InternalSlot::JSSATParseNodeSlot4,
    InternalSlot::JSSATParseNodeSlot5,
    InternalSlot::JSSATParseNodeSlot6,
];

const RUNTIME_SEMANTICS: [InternalSlot; 2] = [
    InternalSlot::JSSATLexicallyDeclaredNames,
    InternalSlot::JSSATVarDeclaredNames,
];

pub struct ParseNode {
    pub parse_node: RegisterId,
    kind: js::ParseNodeKind,
    variant_idx: usize,
    parse_node_slot: usize,
}

impl ParseNode {
    fn new(block: &mut DynBlockBuilder, kind: js::ParseNodeKind, variant_idx: usize) -> Self {
        let parse_node = block.record_new();

        Self {
            parse_node,
            kind,
            variant_idx,
            parse_node_slot: 0,
        }
    }

    fn on_child_created(&mut self, block: &mut DynBlockBuilder, child: &ParseNode) {
        let slot = self.parse_node_slot;
        self.parse_node_slot += 1;

        let slot = *PARSE_NODE_SLOTS
            .get(slot)
            .expect("expected slot (increase -> 6+)");

        block.record_set_slot(self.parse_node, slot, child.parse_node);
    }

    fn finish(
        self,
        block: &mut DynBlockBuilder,
        simple_fns: &FxHashMap<InternalSlot, FnSignature<1>>,
    ) -> Self {
        debug_assert!({
            IntoIter::new(RUNTIME_SEMANTICS).all(|slot| simple_fns.contains_key(&slot))
        });

        for (slot, fn_id) in simple_fns {
            let did_emit_overrides = emit_virt_overrides(self.kind, self.variant_idx, block, *slot);

            // if we don't have a custom implementation of a runtime semantic,
            // emit the default function
            if !did_emit_overrides {
                let virt_fn = block.make_fnptr(fn_id.id);
                block.record_set_slot(self.parse_node, *slot, virt_fn);
            }
        }

        self
    }
}

impl<'b> Visitor for NodeEmitter<'b> {
    fn pre_visit(&mut self, kind: js::ParseNodeKind, variant_idx: usize) {
        println!("-> {:?}", kind);

        let node = ParseNode::new(self.block, kind, variant_idx);
        self.stack.push(node);
    }

    fn post_visit(&mut self) {
        let node = (self.stack.pop())
            .expect("post_visit should be called exactly same amount as pre_visit");

        println!("<- {:?}", node.kind);

        let node = node.finish(self.block, &self.simple_fns);

        if let Some(parent) = self.stack.last_mut() {
            parent.on_child_created(self.block, &node);
        }

        self.last_completed = Some(node);
    }
}