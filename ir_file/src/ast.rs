use lexpr::datum::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AST {
    pub source: String,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub header: Header,
    pub body: Vec<Statement>,
    pub span: Span,
    pub body_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub document_index: String,
    pub method_name: FnName,
    pub parameters: Vec<Variable>,
    pub span: Span,
}

type Variable = String;
type FnName = String;
type Slot = String;
type Atom = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assign {
    pub variable: Variable,
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub span: Span,
    pub data: StatementData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatementData {
    Assign(Assign),
    /// An if statement as an expression is different than an if statement as a
    /// statement becuase an if statement as an expression MUST have a carry
    /// value, whereas an if statement as a statement does not.
    If {
        condition: Expression,
        then: Vec<Statement>,
        r#else: Option<Vec<Statement>>,
    },
    RecordSetProp {
        record: Expression,
        prop: Expression,
        value: Option<Expression>,
    },
    RecordSetSlot {
        record: Expression,
        slot: Slot,
        value: Option<Expression>,
    },
    ListSet {
        list: Expression,
        prop: Expression,
        value: Option<Expression>,
    },
    Return {
        expr: Option<Expression>,
    },
    // TODO(irfile): should we have a comment instruction?
    // Comment {
    //     message: String,
    //     location: Span,
    // },
    /// A call as an expression is different from a call as a statement, because
    /// an expression call expects a value whereas statement call does not.
    CallStatic {
        function_name: FnName,
        args: Vec<Expression>,
    },
    // TODO(isa): implement calling external functions
    // CallExternal {},
    CallVirt {
        fn_ptr: Expression,
        args: Vec<Expression>,
    },
    Assert {
        expr: Expression,
        message: String,
    },
    Loop {
        init: Vec<Assign>,
        cond: Expression,
        next: Vec<Assign>,
        body: Vec<Statement>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expression {
    pub span: Option<Span>,
    pub data: ExpressionData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotOrExpr {
    Slot(Slot),
    Expr(Box<Expression>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionData {
    /// If enabled, there is a piece of "threaded state" which will
    /// automatically thread itself through every single function declaration
    /// and call. This is an instruction to get that piece of threaded state.
    GetGlobal,
    /// An if statement as an expression is different than an if statement as a
    /// statement becuase an if statement as an expression MUST have a carry
    /// value, whereas an if statement as a statement does not.
    If {
        condition: Box<Expression>,
        then: (Vec<Statement>, Box<Expression>),
        r#else: (Vec<Statement>, Box<Expression>),
    },
    VarReference {
        variable: Variable,
    },
    LetIn {
        variable: Variable,
        be_bound_to: Box<Expression>,
        r#in: (Vec<Statement>, Box<Expression>),
    },
    Unreachable,
    RecordNew,
    RecordGetProp {
        record: Box<Expression>,
        property: Box<Expression>,
    },
    RecordGetSlot {
        record: Box<Expression>,
        slot: Slot,
    },
    RecordHasProp {
        record: Box<Expression>,
        property: Box<Expression>,
    },
    RecordHasSlot {
        record: Box<Expression>,
        slot: SlotOrExpr,
    },
    ListNew,
    ListGet {
        list: Box<Expression>,
        property: Box<Expression>,
    },
    ListHas {
        list: Box<Expression>,
        property: Box<Expression>,
    },
    ListLen {
        list: Box<Expression>,
    },
    GetFnPtr {
        function_name: FnName,
    },
    /// A call as an expression is different from a call as a statement, because
    /// an expression call expects a value whereas statement call does not.
    CallStatic {
        function_name: FnName,
        args: Vec<Expression>,
    },
    // TODO(isa): implement calling external functions
    // CallExternal {},
    CallVirt {
        fn_ptr: Box<Expression>,
        args: Vec<Expression>,
    },
    MakeAtom {
        atom: Atom,
    },
    MakeBytes {
        bytes: Vec<u8>,
    },
    MakeInteger {
        value: i64,
    },
    MakeBoolean {
        value: bool,
    },
    BinOp {
        kind: BinOpKind,
        lhs: Box<Expression>,
        rhs: Box<Expression>,
    },
    Negate {
        expr: Box<Expression>,
    },
    IsTypeOf {
        expr: Box<Expression>,
        kind: String,
    },
    IsTypeAs {
        lhs: Box<Expression>,
        rhs: Box<Expression>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOpKind {
    Add,
    And,
    Or,
    Eq,
    Lt,
}

pub trait Visitor {
    fn pre_visit_section(&mut self) {}
    fn post_visit_section(&mut self) {}

    fn pre_visit_stmt(&mut self) {}
    fn post_visit_stmt(&mut self) {}

    fn pre_visit_expr(&mut self) {}
    fn post_visit_expr(&mut self) {}

    fn visit_ast(&mut self, ast: &mut AST) {
        self.visit_ast_impl(ast);
    }

    fn visit_ast_impl(&mut self, ast: &mut AST) {
        for section in &mut ast.sections {
            self.visit_section(section);
        }
    }

    fn visit_section(&mut self, section: &mut Section) {
        self.visit_section_impl(section);
    }

    fn visit_section_impl(&mut self, section: &mut Section) {
        self.visit_stmts(&mut section.body);
    }

    fn visit_maybe_stmts(&mut self, stmts: Option<&mut [Statement]>) {
        if let Some(stmts) = stmts {
            self.visit_stmts(stmts);
        }
    }

    fn visit_stmts(&mut self, stmts: &mut [Statement]) {
        self.visit_stmts_impl(stmts);
    }

    fn visit_stmts_impl(&mut self, stmts: &mut [Statement]) {
        for stmt in stmts {
            self.visit_stmt(stmt);
        }
    }

    fn visit_stmt(&mut self, stmts: &mut Statement) {
        self.visit_stmt_impl(stmts);
    }

    fn visit_stmt_impl(&mut self, stmt: &mut Statement) {
        match &mut stmt.data {
            StatementData::Assign(assign) => self.visit_assign(assign),
            StatementData::If {
                condition,
                then,
                r#else,
            } => {
                self.visit_expr(condition);
                self.visit_stmts(then);
                self.visit_maybe_stmts(r#else.as_deref_mut());
            }
            StatementData::RecordSetProp {
                record,
                prop,
                value,
            } => {
                self.visit_expr(record);
                self.visit_expr(prop);
                self.visit_maybe_expr(value.as_mut());
            }
            StatementData::RecordSetSlot {
                record,
                slot,
                value,
            } => {
                self.visit_expr(record);
                self.visit_slot(slot);
                self.visit_maybe_expr(value.as_mut());
            }
            StatementData::ListSet { list, prop, value } => {
                self.visit_expr(list);
                self.visit_expr(prop);
                self.visit_maybe_expr(value.as_mut());
            }
            StatementData::Return { expr } => {
                self.visit_maybe_expr(expr.as_mut());
            }
            StatementData::CallStatic {
                function_name: _,
                args,
            } => {
                self.visit_exprs(args);
            }
            StatementData::CallVirt { fn_ptr, args } => {
                self.visit_expr(fn_ptr);
                self.visit_exprs(args);
            }
            StatementData::Assert { expr, message: _ } => {
                self.visit_expr(expr);
            }
            StatementData::Loop {
                init,
                cond,
                next,
                body,
            } => {
                self.visit_assigns(init);
                self.visit_expr(cond);
                self.visit_assigns(next);
                self.visit_stmts(body);
            }
        }
    }

    fn visit_assigns(&mut self, assigns: &mut [Assign]) {
        for assign in assigns {
            self.visit_assign(assign);
        }
    }

    fn visit_assign(&mut self, assign: &mut Assign) {
        self.visit_assign_impl(assign);
    }

    fn visit_assign_impl(&mut self, assign: &mut Assign) {
        self.visit_expr(&mut assign.value);
    }

    fn visit_maybe_expr(&mut self, expr: Option<&mut Expression>) {
        if let Some(expr) = expr {
            self.visit_expr(expr);
        }
    }

    fn visit_slot_or_expr(&mut self, slot_or_expr: &mut SlotOrExpr) {
        match slot_or_expr {
            SlotOrExpr::Slot(slot) => self.visit_slot(slot),
            SlotOrExpr::Expr(expr) => self.visit_expr(expr),
        }
    }

    fn visit_exprs(&mut self, exprs: &mut [Expression]) {
        for expr in exprs {
            self.visit_expr(expr);
        }
    }

    fn visit_expr(&mut self, expr: &mut Expression) {
        self.visit_expr_impl(expr);
    }

    fn visit_expr_impl(&mut self, expr: &mut Expression) {
        match &mut expr.data {
            ExpressionData::If {
                condition,
                then: (then_stmts, then_expr),
                r#else: (else_stmts, else_expr),
            } => {
                self.visit_expr(condition);
                self.visit_stmts(then_stmts);
                self.visit_expr(then_expr);
                self.visit_stmts(else_stmts);
                self.visit_expr(else_expr);
            }
            ExpressionData::LetIn {
                variable: _,
                be_bound_to,
                r#in: (stmts, expr),
            } => {
                self.visit_expr(be_bound_to);
                self.visit_stmts(stmts);
                self.visit_expr(expr);
            }
            ExpressionData::RecordGetProp { record, property } => {
                self.visit_expr(record);
                self.visit_expr(property);
            }
            ExpressionData::RecordGetSlot { record, slot } => {
                self.visit_expr(record);
                self.visit_slot(slot);
            }
            ExpressionData::RecordHasProp { record, property } => {
                self.visit_expr(record);
                self.visit_expr(property);
            }
            ExpressionData::RecordHasSlot { record, slot } => {
                self.visit_expr(record);
                self.visit_slot_or_expr(slot);
            }
            ExpressionData::ListGet { list, property } => {
                self.visit_expr(list);
                self.visit_expr(property);
            }
            ExpressionData::ListHas { list, property } => {
                self.visit_expr(list);
                self.visit_expr(property);
            }
            ExpressionData::ListLen { list } => {
                self.visit_expr(list);
            }
            ExpressionData::CallStatic {
                function_name: _,
                args,
            } => {
                self.visit_exprs(args);
            }
            ExpressionData::CallVirt { fn_ptr, args } => {
                self.visit_expr(fn_ptr);
                self.visit_exprs(args);
            }
            ExpressionData::BinOp { kind: _, lhs, rhs } => {
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            ExpressionData::Negate { expr } => {
                self.visit_expr(expr);
            }
            ExpressionData::IsTypeOf { expr, kind: _ } => {
                self.visit_expr(expr);
            }
            ExpressionData::IsTypeAs { lhs, rhs } => {
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            ExpressionData::MakeAtom { atom } => {
                self.visit_slot(atom);
            }
            ExpressionData::GetFnPtr { function_name: _ }
            | ExpressionData::MakeBytes { bytes: _ }
            | ExpressionData::MakeInteger { value: _ }
            | ExpressionData::MakeBoolean { value: _ }
            | ExpressionData::VarReference { variable: _ }
            | ExpressionData::GetGlobal
            | ExpressionData::Unreachable
            | ExpressionData::RecordNew
            | ExpressionData::ListNew => {}
        }
    }

    fn visit_slot(&mut self, _slot: &mut String) {}
}
