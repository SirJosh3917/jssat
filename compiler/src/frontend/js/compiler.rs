use swc_ecmascript::ast::Script;

use crate::{
    frontend::js::{
        builder::{FnArg, FnRef, ProgramBuilder},
        ir::{Type, IR},
    },
    name::{DebugName, Name},
};

pub fn traverse(_script: Script) -> IR {
    let mut program = ProgramBuilder::new();
    // let surrounding_agent = program.global(Name::new("surrounding_agent"));
    let print = program.external_function(
        Name::new("jssatrt_print"),
        Type::Void,
        vec![Type::Runtime, Type::Any, Type::Any],
    );

    let mut print_stub = program.function(DebugName::new("print_stub"), false);
    {
        let print_value = print_stub.parameter(DebugName::new("value"));

        let mut entry = print_stub.block(DebugName::new("entry"));
        entry.call(
            FnRef::ExtFn(print),
            &[
                FnArg::Runtime,
                FnArg::Reg(print_value),
                FnArg::Reg(print_value),
            ],
            false,
        );
        entry.ret(&mut print_stub, None);
    }

    let mut main = program.function(DebugName::new("main"), true);
    {
        let mut entry = main.block(DebugName::new("entry"));

        let hello_world = program.constant(DebugName::new("hello_world"), "Hello, World!".into());
        entry.call(FnRef::Fn(&print_stub), &[FnArg::Cnst(hello_world)], false);

        entry.ret(&mut main, None);
    }

    print_stub.finish(&mut program);
    main.finish(&mut program);

    program.build()
}
