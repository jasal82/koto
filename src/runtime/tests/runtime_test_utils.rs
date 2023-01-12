#![allow(unused)]

use {
    koto_bytecode::{Chunk, Loader},
    koto_runtime::{prelude::*, Value::*},
    std::rc::Rc,
};

pub fn test_script(script: &str, expected_output: impl Into<Value>) {
    test_script_with_vm(Vm::default(), script, expected_output.into());
}

pub fn test_script_with_vm(mut vm: Vm, script: &str, expected_output: Value) {
    let mut loader = Loader::default();
    let chunk = match loader.compile_script(script, &None) {
        Ok(chunk) => chunk,
        Err(error) => {
            print_chunk(script, vm.chunk());
            panic!("Error while compiling script: {}", error);
        }
    };

    match vm.run(chunk) {
        Ok(result) => {
            match vm.run_binary_op(BinaryOp::Equal, result.clone(), expected_output.clone()) {
                Ok(Value::Bool(true)) => {}
                Ok(Value::Bool(false)) => {
                    print_chunk(script, vm.chunk());
                    panic!(
                        "Unexpected result - expected: {}, result: {}",
                        expected_output, result
                    );
                }
                Ok(other) => {
                    print_chunk(script, vm.chunk());
                    panic!("Expected bool from equality comparison, found '{}'", other);
                }
                Err(e) => {
                    print_chunk(script, vm.chunk());
                    panic!("Error while comparing output value: {}", e);
                }
            }
        }
        Err(e) => {
            print_chunk(script, vm.chunk());
            panic!("Error while running script: {}", e);
        }
    }
}

pub fn print_chunk(script: &str, chunk: Rc<Chunk>) {
    println!("{}\n", script);
    let script_lines = script.lines().collect::<Vec<_>>();

    println!("Constants\n---------\n{}\n", chunk.constants);
    println!(
        "Instructions\n------------\n{}",
        Chunk::instructions_as_string(chunk, &script_lines)
    );
}

pub fn number<T>(value: T) -> Value
where
    T: Copy,
    f64: From<T>,
{
    Number(f64::from(value).into())
}

pub fn number_list<T>(values: &[T]) -> Value
where
    T: Copy,
    i64: From<T>,
{
    let values = values
        .iter()
        .map(|n| Number(i64::from(*n).into()))
        .collect::<Vec<_>>();
    value_list(&values)
}

pub fn number_tuple<T>(values: &[T]) -> Value
where
    T: Copy,
    i64: From<T>,
{
    let values = values
        .iter()
        .map(|n| Number(i64::from(*n).into()))
        .collect::<Vec<_>>();
    value_tuple(&values)
}

pub fn value_list(values: &[Value]) -> Value {
    List(ValueList::from_slice(values))
}

pub fn value_tuple(values: &[Value]) -> Value {
    Tuple(values.into())
}

pub fn string(s: &str) -> Value {
    Str(s.into())
}
