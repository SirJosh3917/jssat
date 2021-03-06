use crate::parser::rules::apply_rule_recursively_inner;

use self::rules::apply_rule_recursively;
use super::*;

mod node;
mod rules;
use node::*;

pub fn parse(code: &str) -> AST {
    let source = code.to_owned();
    let nodes = parse_with_rule_application(parse_to_nodes(code));

    let sections = nodes
        .into_iter()
        .map(|node| {
            let span = node.span();
            let mut children = node.expect_parent();
            let body = children.pop().unwrap();
            let body_span = body.span();
            let header = parse_header(children.pop().unwrap());

            let body = parse_body(body.expect_parent());
            Section {
                header,
                body,
                span,
                body_span,
            }
        })
        .collect();

    AST { source, sections }
}

fn parse_with_rule_application(nodes: Vec<Node>) -> Vec<Node> {
    let mut new_nodes = Vec::new();
    let mut custom_rules = Vec::new();

    // read all rules first so you can just define rules in your file and have them
    // work anywhere else
    for node in nodes.iter() {
        if let Node::Parent(children, _) = &node {
            if let Some(Node::Word(header_word, _)) = children.get(0) {
                if header_word == "def" {
                    let (rule, generate) = match (children.get(1), children.get(2)) {
                        (Some(n), Some(m)) => (n.clone(), m.clone()),
                        _ => panic!("malformed rule {}", node.to_lisp()),
                    };

                    custom_rules.push((rule, generate));
                    continue;
                }
            }
        };
    }

    // then apply the rules to themselves thoroughly
    // this will mean that once we match a rule, it will already be fully expanded
    // which is useful for both performance and for ease of use (as we now mostly
    // don't have to care about the order things are defined in)
    custom_rules = custom_rules
        .iter()
        // for every rewrite rule
        .map(|(rule, generate)| {
            let mut new_generate = generate.clone();

            // keep rewriting the rule using the existing rules until it is fully rewritten
            // (i.e. expanded)
            #[allow(
                unused_parens,
                // emphasize that `while {} {}` is `while expr block`
            )]
            while ({
                let mut did_change = false;

                new_generate =
                    custom_rules
                        .iter()
                        .fold(new_generate, |new_generate, (rule, generate)| {
                            let (changed, new_generate) =
                                apply_rule_recursively_inner(rule, generate, new_generate);
                            did_change = did_change || changed;
                            new_generate
                        });

                did_change
            }) {}

            // use the fully rewritten rule as the new rule
            (rule.clone(), new_generate)
        })
        .collect();

    for node in nodes {
        if let Node::Parent(children, _) = &node {
            if let Some(Node::Word(header_word, _)) = children.get(0) {
                if header_word == "def" {
                    continue;
                }
            }
        };

        let node = custom_rules.iter().fold(node, |node, (rule, generate)| {
            apply_rule_recursively(rule, generate, node)
        });

        new_nodes.push(node);
    }

    new_nodes
}

#[cfg(test)]
mod parse_with_rule_application_tests {
    use super::*;

    macro_rules! parse {
        ($code: expr, $yields: expr) => {
            assert_eq!(
                parse_with_rule_application(parse_to_nodes($code)),
                parse_to_nodes($yields)
            )
        };
    }

    #[test]
    fn identity() {
        let id = |x| parse!(x, x);

        id("f x");
        id("(whatever-idk)");
        id("(a (s) (d) (fff))");
    }

    #[test]
    fn rules_work_in_all_locations() {
        parse!("(def x y) x", "y");
        parse!("x (def x y)", "y");
        parse!("x (def x y) x", "y y");
    }

    #[test]
    fn applies_rules_to_node() {
        parse!(
            r#"
(def x y)

(x
    x
    (x)
    (x))
"#,
            "(y y (y) (y))"
        );
    }

    #[test]
    fn rules_apply_themselves_to_expand_fully() {
        parse!(
            r#"
(def a b)
(def c d)
(def b c)
(a b c d)
"#,
            "(d d d d)"
        );

        parse!(
            r#"
(def (and :a :b) (:a and :b))
(def (and3 :a :b :c) (and (and :a :b) :c))

(and3 x y z)
        "#,
            "((x and y) and z)"
        )
    }

    #[test]
    fn rules_are_order_independent() {
        let emit = |x| match x {
            1 => "(def (wrap1 :x) (unwrapped1 :x))",
            2 => "(def (wrap2 :x) (unwrapped2 (wrap1 :x)))",
            3 => "(def (wrap3 :x) (unwrapped3 (wrap2 :x)))",
            _ => unreachable!(),
        };

        let test = |a, b, c| {
            parse!(
                &format!("{}\n{}\n{}\n(wrap3 x)", emit(a), emit(b), emit(c)),
                "(unwrapped3 (unwrapped2 (unwrapped1 x)))"
            )
        };

        test(1, 2, 3);
        test(1, 3, 2);
        test(2, 1, 3);
        test(3, 1, 2);
        test(2, 3, 1);
        test(3, 2, 1);
    }

    #[test]
    fn applies_rule_recursively() {
        parse!(
            r#"
(def (f :1 :2) ((:1) (:2)))
(f (f 1 2) (f 3 4))
"#,
            "((((1) (2))) (((3) (4))))"
        );
    }
}

fn parse_header(header: Node) -> Header {
    let span = header.span();
    let mut header = header.expect_parent();
    let parameters = header.pop().unwrap().expect_parent();
    let method_name = header.pop().unwrap().expect_word();
    let document_index = header.pop().unwrap().expect_atom();
    assert!(header.is_empty());

    let parameters = parameters
        .into_iter()
        .map(|p| p.expect_word().trim().trim_matches(',').to_string())
        .collect();

    Header {
        document_index,
        method_name,
        parameters,
        span,
    }
}

fn parse_body(body: Vec<Node>) -> Vec<Statement> {
    body.into_iter()
        .map(|node| {
            let node_span = node.span();
            let children = match node {
                Node::Parent(children, _) => children,
                other => panic!("expected parent node, got {:?}", other),
            };

            let get = |x| children.get(x).map(Node::as_ref);
            let getp = |x| children.get(x);

            if let (
                Some(Node::Word("loop", _)),
                Some(assigns),
                Some(cond),
                Some(next),
                Some(body),
            ) = (get(0), getp(1), get(2), getp(3), getp(4))
            {
                let assigns = parse_body(assigns.clone().expect_parent())
                    .into_iter()
                    .map(|s| match s.data {
                        StatementData::Assign(a) => a,
                        _ => panic!("expected list of assignments"),
                    })
                    .collect();

                let nexts = parse_body(next.clone().expect_parent())
                    .into_iter()
                    .map(|s| match s.data {
                        StatementData::Assign(a) => a,
                        _ => panic!("expected list of assignments"),
                    })
                    .collect();

                let expr = parse_expression(cond);

                let body = parse_body(body.clone().expect_parent());

                return Statement {
                    span: node_span,
                    data: StatementData::Loop {
                        init: assigns,
                        next: nexts,
                        cond: expr,
                        body,
                    },
                };
            }

            match (get(0), get(1), get(2), get(3)) {
                (Some(Node::Word("assert", _)), Some(expr), Some(Node::String(msg, _)), None) => {
                    Statement {
                        span: node_span,
                        data: StatementData::Assert {
                            expr: parse_expression(expr),
                            message: msg.to_owned(),
                        },
                    }
                }
                // (Some(Node::Word("comment", _)), Some(Node::String(msg, _)), None, None) => {
                //     Statement::Comment {
                //         message: msg.to_string(),
                //         location: node_span,
                //     }
                // }
                (Some(Node::Word(identifier, _)), Some(Node::Word("=", _)), Some(expr), None) => {
                    Statement {
                        span: node_span,
                        data: StatementData::Assign(Assign {
                            variable: identifier.to_string(),
                            value: parse_expression(expr),
                        }),
                    }
                }
                (
                    Some(Node::Word("record-set-slot", _)),
                    Some(record),
                    Some(Node::Word(slot, _)),
                    Some(value),
                ) => Statement {
                    span: node_span,
                    data: StatementData::RecordSetSlot {
                        record: parse_expression(record),
                        slot: slot.to_owned(),
                        value: Some(parse_expression(value)),
                    },
                },
                (
                    Some(Node::Word("record-del-slot", _)),
                    Some(record),
                    Some(Node::Word(slot, _)),
                    None,
                ) => Statement {
                    span: node_span,
                    data: StatementData::RecordSetSlot {
                        record: parse_expression(record),
                        slot: slot.to_owned(),
                        value: None,
                    },
                },
                (Some(Node::Word("record-set-prop", _)), Some(record), Some(prop), Some(value)) => {
                    Statement {
                        span: node_span,
                        data: StatementData::RecordSetProp {
                            record: parse_expression(record),
                            prop: parse_expression(prop),
                            value: Some(parse_expression(value)),
                        },
                    }
                }
                (Some(Node::Word("record-del-prop", _)), Some(record), Some(prop), None) => {
                    Statement {
                        span: node_span,
                        data: StatementData::RecordSetProp {
                            record: parse_expression(record),
                            prop: parse_expression(prop),
                            value: None,
                        },
                    }
                }
                (Some(Node::Word("list-set", _)), Some(list), Some(prop), Some(value)) => {
                    Statement {
                        span: node_span,
                        data: StatementData::ListSet {
                            list: parse_expression(list),
                            prop: parse_expression(prop),
                            value: Some(parse_expression(value)),
                        },
                    }
                }
                (Some(Node::Word("list-del", _)), Some(list), Some(prop), None) => Statement {
                    span: node_span,
                    data: StatementData::ListSet {
                        list: parse_expression(list),
                        prop: parse_expression(prop),
                        value: None,
                    },
                },
                (Some(Node::Word("call", _)), Some(Node::Word(fn_name, _)), _, _) => Statement {
                    span: node_span,
                    data: StatementData::CallStatic {
                        function_name: fn_name.to_owned(),
                        args: children
                            .iter()
                            .skip(2)
                            .map(|node| parse_expression(node.as_ref()))
                            .collect(),
                    },
                },
                (Some(Node::Word("call-virt", _)), Some(expr), _, _) => Statement {
                    span: node_span,
                    data: StatementData::CallVirt {
                        fn_ptr: parse_expression(expr),
                        args: children
                            .iter()
                            .skip(2)
                            .map(|node| parse_expression(node.as_ref()))
                            .collect(),
                    },
                },
                (Some(Node::Word("if", _)), Some(condition), Some(Node::Parent(then, _)), None) => {
                    Statement {
                        span: node_span,
                        data: StatementData::If {
                            condition: parse_expression(condition),
                            then: parse_body(then),
                            r#else: None,
                        },
                    }
                }
                (
                    Some(Node::Word("if", _)),
                    Some(condition),
                    Some(Node::Parent(then, _)),
                    Some(Node::Parent(r#else, _)),
                ) => Statement {
                    span: node_span,
                    data: StatementData::If {
                        condition: parse_expression(condition),
                        then: parse_body(then),
                        r#else: Some(parse_body(r#else)),
                    },
                },
                (Some(Node::Word("return", _)), Some(expr), None, None) => Statement {
                    span: node_span,
                    data: StatementData::Return {
                        expr: Some(parse_expression(expr)),
                    },
                },
                (Some(Node::Word("return", _)), None, None, None) => Statement {
                    span: node_span,
                    data: StatementData::Return { expr: None },
                },
                _ => panic!(
                    "unrecognized statement {}",
                    // TODO(maybe-rustc-bug): why can't rustc infer the type here?
                    Node::<String>::Parent(children, node_span).to_lisp()
                ),
            }
        })
        .collect()
}

fn parse_expression(node: Node<&str>) -> Expression {
    let node_span = Some(node.span());
    match node {
        Node::Word("get-global", _) => Expression {
            span: node_span,
            data: ExpressionData::GetGlobal,
        },
        Node::Word("record-new", _) => Expression {
            span: node_span,
            data: ExpressionData::RecordNew,
        },
        Node::Word("list-new", _) => Expression {
            span: node_span,
            data: ExpressionData::ListNew,
        },
        Node::Word("true", _) => Expression {
            span: node_span,
            data: ExpressionData::MakeBoolean { value: true },
        },
        Node::Word("false", _) => Expression {
            span: node_span,
            data: ExpressionData::MakeBoolean { value: false },
        },
        Node::Word("unreachable", _) => Expression {
            span: node_span,
            data: ExpressionData::Unreachable,
        },
        Node::Atom(identifier, _) => Expression {
            span: node_span,
            data: ExpressionData::VarReference {
                variable: identifier.to_string(),
            },
        },
        Node::String(str, _) => Expression {
            span: node_span,
            data: ExpressionData::MakeBytes {
                bytes: str.as_bytes().to_owned(),
            },
        },
        Node::Number(num, _) => Expression {
            span: node_span,
            data: ExpressionData::MakeInteger {
                value: if let Some(v) = num.as_i64() {
                    v
                } else {
                    panic!("cannot do fp at this time")
                },
            },
        },
        Node::Parent(children, parent_span) => {
            let get = |x| children.get(x).map(Node::as_ref);

            match (get(0), get(1), get(2), get(3), get(4), get(5)) {
                (
                    Some(Node::Word("let", _)),
                    Some(Node::Word(identifier, _)),
                    Some(Node::Word("=", _)),
                    Some(expr),
                    Some(Node::Word("in", _)),
                    Some(expr2),
                ) => {
                    let r#in = match expr2 {
                        Node::Parent(mut children, _) => {
                            let result = children.pop().unwrap();
                            (
                                parse_body(children),
                                Box::new(parse_expression(result.as_ref())),
                            )
                        }
                        expr => (Vec::new(), Box::new(parse_expression(expr))),
                    };

                    return Expression {
                        span: node_span,
                        data: ExpressionData::LetIn {
                            variable: identifier.into(),
                            be_bound_to: Box::new(parse_expression(expr)),
                            r#in,
                        },
                    };
                }
                (
                    Some(Node::Word("if", _)),
                    Some(condition),
                    Some(Node::Parent(mut then, _)),
                    Some(Node::Parent(mut r#else, _)),
                    None,
                    None,
                ) => {
                    let condition = parse_expression(condition);

                    let then_expr = parse_expression(then.pop().unwrap().as_ref());
                    let then_stmts = parse_body(then);

                    let else_expr = parse_expression(r#else.pop().unwrap().as_ref());
                    let else_stmts = parse_body(r#else);

                    return Expression {
                        span: node_span,
                        data: ExpressionData::If {
                            condition: Box::new(condition),
                            then: (then_stmts, Box::new(then_expr)),
                            r#else: (else_stmts, Box::new(else_expr)),
                        },
                    };
                }
                _ => {}
            };

            match (get(0), get(1), get(2)) {
                (Some(Node::Word("record-get-prop", _)), Some(record), Some(expr)) => Expression {
                    span: node_span,
                    data: ExpressionData::RecordGetProp {
                        record: Box::new(parse_expression(record)),
                        property: Box::new(parse_expression(expr)),
                    },
                },
                (
                    Some(Node::Word("record-get-slot", _)),
                    Some(record),
                    Some(Node::Word(slot, _)),
                ) => Expression {
                    span: node_span,
                    data: ExpressionData::RecordGetSlot {
                        record: Box::new(parse_expression(record)),
                        slot: slot.to_owned(),
                    },
                },
                (Some(Node::Word("record-has-prop", _)), Some(record), Some(expr)) => Expression {
                    span: node_span,
                    data: ExpressionData::RecordHasProp {
                        record: Box::new(parse_expression(record)),
                        property: Box::new(parse_expression(expr)),
                    },
                },
                (
                    Some(Node::Word("record-has-slot", _)),
                    Some(record),
                    Some(Node::Word(slot, _)),
                ) => Expression {
                    span: node_span,
                    data: ExpressionData::RecordHasSlot {
                        record: Box::new(parse_expression(record)),
                        slot: SlotOrExpr::Slot(slot.to_owned()),
                    },
                },
                (Some(Node::Word("record-has-slot", _)), Some(record), Some(expr)) => Expression {
                    span: node_span,
                    data: ExpressionData::RecordHasSlot {
                        record: Box::new(parse_expression(record)),
                        slot: SlotOrExpr::Expr(Box::new(parse_expression(expr))),
                    },
                },
                (Some(Node::Word("list-get", _)), Some(list), Some(expr)) => Expression {
                    span: node_span,
                    data: ExpressionData::ListGet {
                        list: Box::new(parse_expression(list)),
                        property: Box::new(parse_expression(expr)),
                    },
                },
                (Some(Node::Word("list-has", _)), Some(list), Some(expr)) => Expression {
                    span: node_span,
                    data: ExpressionData::ListHas {
                        list: Box::new(parse_expression(list)),
                        property: Box::new(parse_expression(expr)),
                    },
                },
                (Some(Node::Word("list-len", _)), Some(list), None) => Expression {
                    span: node_span,
                    data: ExpressionData::ListLen {
                        list: Box::new(parse_expression(list)),
                    },
                },
                (Some(Node::Word("get-fn-ptr", _)), Some(Node::Word(fn_name, _)), None) => {
                    Expression {
                        span: node_span,
                        data: ExpressionData::GetFnPtr {
                            function_name: fn_name.to_owned(),
                        },
                    }
                }
                (Some(Node::Word("call", _)), Some(Node::Word(fn_name, _)), _) => Expression {
                    span: node_span,
                    data: ExpressionData::CallStatic {
                        function_name: fn_name.to_owned(),
                        args: children
                            .iter()
                            .skip(2)
                            .map(|node| parse_expression(node.as_ref()))
                            .collect(),
                    },
                },
                (Some(Node::Word("call-virt", _)), Some(expr), _) => Expression {
                    span: node_span,
                    data: ExpressionData::CallVirt {
                        fn_ptr: Box::new(parse_expression(expr)),
                        args: children
                            .iter()
                            .skip(2)
                            .map(|node| parse_expression(node.as_ref()))
                            .collect(),
                    },
                },
                (Some(Node::Word("atom", _)), Some(Node::Word(atom, _)), None) => Expression {
                    span: node_span,
                    data: ExpressionData::MakeAtom {
                        atom: atom.to_string(),
                    },
                },
                (
                    Some(lhs),
                    Some(Node::Word(kind @ ("+" | "and" | "==" | "<" | "or"), _)),
                    Some(rhs),
                ) => Expression {
                    span: node_span,
                    data: ExpressionData::BinOp {
                        kind: match kind {
                            // DEAR FUTURE EDITORS: make sure to edit the pattern up above too
                            // you're welcome
                            "+" => BinOpKind::Add,
                            "and" => BinOpKind::And,
                            "==" => BinOpKind::Eq,
                            "<" => BinOpKind::Lt,
                            "or" => BinOpKind::Or,
                            _ => unreachable!("what"),
                        },
                        lhs: Box::new(parse_expression(lhs)),
                        rhs: Box::new(parse_expression(rhs)),
                    },
                },
                (Some(Node::Word("not", _)), Some(expr), None) => Expression {
                    span: node_span,
                    data: ExpressionData::Negate {
                        expr: Box::new(parse_expression(expr)),
                    },
                },
                (Some(Node::Word("is-type-of", _)), Some(Node::Word(kind, _)), Some(expr)) => {
                    Expression {
                        span: node_span,
                        data: ExpressionData::IsTypeOf {
                            expr: Box::new(parse_expression(expr)),
                            kind: kind.to_owned(),
                        },
                    }
                }
                (Some(Node::Word("is-type-as", _)), Some(lhs), Some(rhs)) => Expression {
                    span: node_span,
                    data: ExpressionData::IsTypeAs {
                        lhs: Box::new(parse_expression(lhs)),
                        rhs: Box::new(parse_expression(rhs)),
                    },
                },
                (Some(parenthetical), None, None) => parse_expression(parenthetical),
                _ => panic!(
                    "unrecognized expression {:?} at {:?}",
                    Node::<String>::Parent(children, parent_span).to_lisp(),
                    parent_span
                ),
            }
        }
        other => panic!(
            "unrecognized expression {:?} at {:?}",
            other.to_lisp(),
            other.span()
        ),
    }
}

#[test]
fn parses_header() {
    // TODO: there's some crate that allows us to make tests an then save the
    //   data if it looks right, we should use that
    // assert_eq!(
    //     parse_header(
    //         parse_to_nodes("(:6.9.4.2 TheFunctionName ( a, b, c ))")
    //             .pop()
    //             .unwrap()
    //     ),
    //     Header {
    //         document_index: "6.9.4.2".into(),
    //         method_name: "TheFunctionName".into(),
    //         parameters: vec!["a".into(), "b".into(), "c".into()],
    //         span: Span::,
    //     }
    // );
}

// #[test]
#[cfg(disabled_for_now)]
fn parses_statement() {
    let x = || Expression::VarReference {
        variable: "x".into(),
    };

    let y = || Expression::VarReference {
        variable: "y".into(),
    };

    let z = || Expression::VarReference {
        variable: "z".into(),
    };

    let check = |code, stmt| assert_eq!(parse_body(parse_to_nodes(code)), vec![stmt]);

    check(
        "(y = :x)",
        Statement::Assign(Assign {
            variable: "y".into(),
            value: x(),
        }),
    );

    check(
        "(record-set-prop :x :y :z)",
        Statement::RecordSetProp {
            record: x(),
            prop: y(),
            value: Some(z()),
        },
    );

    check(
        "(record-del-prop :x :y)",
        Statement::RecordSetProp {
            record: x(),
            prop: y(),
            value: None,
        },
    );

    check(
        "(record-set-slot :x InternalSlot :z)",
        Statement::RecordSetSlot {
            record: x(),
            slot: "InternalSlot".into(),
            value: Some(z()),
        },
    );

    check(
        "(record-del-slot :x InternalSlot)",
        Statement::RecordSetSlot {
            record: x(),
            slot: "InternalSlot".into(),
            value: None,
        },
    );

    check("(return)", Statement::Return { expr: None });
    check("(return :x)", Statement::Return { expr: Some(x()) });

    check(
        "(call f)",
        Statement::CallStatic {
            function_name: "f".into(),
            args: Vec::new(),
        },
    );

    check(
        "(call f :x :y :z)",
        Statement::CallStatic {
            function_name: "f".into(),
            args: vec![x(), y(), z()],
        },
    );

    check(
        "(call-virt :x)",
        Statement::CallVirt {
            fn_ptr: x(),
            args: Vec::new(),
        },
    );

    check(
        "(call-virt :x :y :z)",
        Statement::CallVirt {
            fn_ptr: x(),
            args: vec![y(), z()],
        },
    );

    check(
        r#"(assert :x "x must be true")"#,
        Statement::Assert {
            expr: x(),
            message: "x must be true".into(),
        },
    );
}

#[cfg(test)]
macro_rules! expr {
    ($x: expr) => {
        parse_expression(parse_to_nodes($x)[0].as_ref())
    };
}

#[cfg(disabled_for_now)]
fn parses_expression() {
    let x = || Expression::VarReference {
        variable: "x".into(),
    };

    let y = || Expression::VarReference {
        variable: "y".into(),
    };

    let z = || Expression::VarReference {
        variable: "z".into(),
    };

    assert_eq!(expr!(":x"), x());
    assert_eq!(expr!(":y"), y());
    assert_eq!(expr!(":z"), z());

    let x = || Box::new(x());
    let y = || Box::new(y());
    let z = || Box::new(z());

    assert_eq!(
        expr!("(if :x (:y) (:z))"),
        Expression::If {
            condition: x(),
            then: (Vec::new(), y()),
            r#else: (Vec::new(), z()),
        }
    );

    assert_eq!(
        expr!("(if :x ((z = :x) :y) ((z = :y) :z))"),
        Expression::If {
            condition: x(),
            then: (
                vec![Statement::Assign(Assign {
                    variable: "z".into(),
                    value: *x(),
                })],
                y()
            ),
            r#else: (
                vec![Statement::Assign(Assign {
                    variable: "z".into(),
                    value: *y(),
                })],
                z()
            ),
        }
    );

    // test parenthetical nesting
    assert_eq!(expr!("record-new"), Expression::RecordNew);
    assert_eq!(expr!("(record-new)"), Expression::RecordNew);
    assert_eq!(expr!("((record-new))"), Expression::RecordNew);

    assert_eq!(
        expr!("(record-get-prop :x :y)"),
        Expression::RecordGetProp {
            record: x(),
            property: y(),
        }
    );

    assert_eq!(
        expr!("(record-get-slot :x InternalSlot)"),
        Expression::RecordGetSlot {
            record: x(),
            slot: "InternalSlot".into()
        }
    );

    assert_eq!(
        expr!("(record-has-prop :x :y)"),
        Expression::RecordHasProp {
            record: x(),
            property: y(),
        }
    );

    assert_eq!(
        expr!("(record-has-slot :x InternalSlot)"),
        Expression::RecordHasSlot {
            record: x(),
            slot: "InternalSlot".into(),
        }
    );

    assert_eq!(
        expr!("(get-fn-ptr FnName)"),
        Expression::GetFnPtr {
            function_name: "FnName".into()
        }
    );

    assert_eq!(
        expr!("(call FnName)"),
        Expression::CallStatic {
            function_name: "FnName".into(),
            args: Vec::new()
        }
    );

    assert_eq!(
        expr!("(call FnName :x :y :z)"),
        Expression::CallStatic {
            function_name: "FnName".into(),
            args: vec![*x(), *y(), *z()]
        }
    );

    assert_eq!(
        expr!("(call-virt :x)"),
        Expression::CallVirt {
            fn_ptr: x(),
            args: vec![],
        }
    );

    assert_eq!(
        expr!("(call-virt :x :y :z)"),
        Expression::CallVirt {
            fn_ptr: x(),
            args: vec![*y(), *z()],
        }
    );

    assert_eq!(
        expr!("(atom undefined)"),
        Expression::MakeAtom {
            atom: "undefined".into()
        }
    );

    assert_eq!(
        expr!(r#""xD""#),
        Expression::MakeBytes {
            bytes: "xD".as_bytes().to_owned()
        }
    );

    assert_eq!(
        expr!(r#""xD""#),
        Expression::MakeBytes {
            bytes: "xD".as_bytes().to_owned()
        }
    );

    assert_eq!(expr!(r#"69"#), Expression::MakeInteger { value: 69 });
    assert_eq!(expr!(r#"(69)"#), Expression::MakeInteger { value: 69 });
    assert_eq!(expr!(r#"((69))"#), Expression::MakeInteger { value: 69 });

    assert_eq!(expr!("true"), Expression::MakeBoolean { value: true });
    assert_eq!(expr!("false"), Expression::MakeBoolean { value: false });

    let binop = |kind| Expression::BinOp {
        kind,
        lhs: x(),
        rhs: y(),
    };

    assert_eq!(expr!("(:x + :y)"), binop(BinOpKind::Add));
    assert_eq!(expr!("(:x and :y)"), binop(BinOpKind::And));
    assert_eq!(expr!("(:x or :y)"), binop(BinOpKind::Or));
    assert_eq!(expr!("(:x == :y)"), binop(BinOpKind::Eq));
    assert_eq!(expr!("(:x < :y)"), binop(BinOpKind::Lt));

    assert_eq!(expr!("(not :x)"), Expression::Negate { expr: x() });

    assert_eq!(
        expr!("(is-type-of String :x)"),
        Expression::IsTypeOf {
            expr: x(),
            kind: "String".into()
        }
    );

    assert_eq!(
        expr!("(is-type-as :x :y)"),
        Expression::IsTypeAs { lhs: x(), rhs: y() }
    );

    assert_eq!(expr!("unreachable"), Expression::Unreachable);

    assert_eq!(
        expr!("(let x = :y in :x)"),
        Expression::LetIn {
            variable: "x".into(),
            be_bound_to: y(),
            r#in: (vec![], x())
        }
    );

    assert_eq!(
        expr!("(let x = :y in (:x))"),
        Expression::LetIn {
            variable: "x".into(),
            be_bound_to: y(),
            r#in: (vec![], x())
        }
    );

    assert_eq!(
        expr!("(let x = :y in ((tmp = record-new) :x))"),
        Expression::LetIn {
            variable: "x".into(),
            be_bound_to: y(),
            r#in: (
                vec![Statement::Assign(Assign {
                    variable: "tmp".into(),
                    value: Expression::RecordNew
                })],
                x()
            )
        }
    );

    assert_eq!(
        expr!("(let x = :y in ((tmp = record-new) (:x)))"),
        Expression::LetIn {
            variable: "x".into(),
            be_bound_to: y(),
            r#in: (
                vec![Statement::Assign(Assign {
                    variable: "tmp".into(),
                    value: Expression::RecordNew
                })],
                x()
            )
        }
    );
}
