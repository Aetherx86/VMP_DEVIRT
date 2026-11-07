//! The `wasm-superopt` command.

use std::process::ExitCode;

const USAGE: &str = "usage: wasm-superopt INPUT.wasm [-o OUTPUT.wasm]

Simplifies a WebAssembly module: folds constants, removes Mixed Boolean-Arithmetic and opaque
predicates, and removes dead code and shuffling. Prints a report, and writes the result if given
somewhere to write it. The behavior of the module does not change, and it never gets larger.";

fn main() -> ExitCode {
    let mut input = None;
    let mut output = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--output" => match args.next() {
                Some(path) => output = Some(path),
                None => return fail(&format!("{arg} needs a path\n\n{USAGE}")),
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("wasm-superopt {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => {
                return fail(&format!("unknown option {other}\n\n{USAGE}"));
            }
            _ if input.is_some() => return fail(&format!("more than one input\n\n{USAGE}")),
            _ => input = Some(arg),
        }
    }
    let Some(input) = input else {
        return fail(USAGE);
    };
    let bytes = match std::fs::read(&input) {
        Ok(bytes) => bytes,
        Err(e) => return fail(&format!("cannot read {input}: {e}")),
    };
    let result = match wasm_superopt::optimize(&bytes) {
        Ok(result) => result,
        Err(e) => return fail(&format!("{input}: {e}")),
    };
    if let Some(path) = output
        && let Err(e) = std::fs::write(&path, &result.wasm)
    {
        return fail(&format!("cannot write {path}: {e}"));
    }
    let r = &result.report;
    println!(
        "bytes:              {} -> {}",
        r.bytes_before, r.bytes_after
    );
    println!(
        "instructions:       {} -> {}",
        r.instructions_before, r.instructions_after
    );
    println!(
        "functions improved: {} of {}, in {} passes",
        r.functions_improved, r.functions, r.passes
    );
    println!("constants folded:   {}", r.constants_folded);
    println!("identities applied: {}", r.identities_applied);
    println!("MBA simplified:     {}", r.mba_simplified);
    println!("opaque predicates:  {}", r.opaque_predicates);
    println!("branches removed:   {}", r.branches_removed);
    ExitCode::SUCCESS
}

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::FAILURE
}
