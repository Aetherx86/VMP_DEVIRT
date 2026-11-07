//! The `absint` command.

use absint::analysis::{Verdict, analyze};
use absint::ir::{Function, Program};
use std::process::ExitCode;

const USAGE: &str = "usage: absint FILE [--x86-64 [--base ADDRESS] [--entry ADDRESS]] [--all]

FILE is a WebAssembly module, a program in the text format, or with --x86-64 raw machine code.
Reports everything it could not prove safe, or with --all, everything it checked.
The exit code is 0 if everything was proved, 1 if something was not, and 2 on an error.";

fn number(text: &str) -> Option<u64> {
    match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

fn check_functions(functions: &[Function], all: bool) -> (usize, usize) {
    let (mut proved, mut unproved) = (0, 0);
    for function in functions {
        let result = analyze(function);
        for check in &result.checks {
            if check.verdict == Verdict::Proved {
                proved += 1;
            } else {
                unproved += 1;
            }
            if all || check.verdict != Verdict::Proved {
                let block = &function.blocks[check.block].name;
                let verdict = match check.verdict {
                    Verdict::Proved if check.dead => "proved (unreachable)",
                    Verdict::Proved => "proved",
                    Verdict::Possible => "POSSIBLE",
                    Verdict::Definite => "DEFINITE",
                };
                let place = if check.line > 0 {
                    format!("line {}", check.line)
                } else {
                    format!("in {block}")
                };
                println!(
                    "{}: {place}: {verdict}: {}",
                    function.name,
                    check.fault.describe()
                );
            }
        }
    }
    (proved, unproved)
}

fn check_races(program: &Program) -> usize {
    let report = absint::races::analyze(program);
    for race in &report.races {
        println!(
            "POSSIBLE: data race on `{}`, between `{}` line {} and `{}` line {}",
            program.globals[race.global as usize],
            program.functions[race.first.function].name,
            race.first.line,
            program.functions[race.second.function].name,
            race.second.line,
        );
    }
    for misuse in &report.misuse {
        println!(
            "POSSIBLE: `{}` {} (`{}`)",
            program.functions[misuse.function].name,
            misuse.message,
            program.locks[misuse.lock as usize]
        );
    }
    if report.accesses > 0 && report.races.is_empty() {
        println!(
            "no data races among {} accesses to globals",
            report.accesses
        );
    }
    report.races.len() + report.misuse.len()
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut path, mut x86, mut base, mut entry, mut all) = (None, false, 0u64, None, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--x86-64" => x86 = true,
            "--all" => all = true,
            "--base" | "--entry" => {
                let Some(value) = args.next().as_deref().and_then(number) else {
                    eprintln!("{arg} needs an address\n\n{USAGE}");
                    return ExitCode::from(2);
                };
                if arg == "--base" {
                    base = value;
                } else {
                    entry = Some(value);
                }
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("absint {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => {
                eprintln!("unknown option {other}\n\n{USAGE}");
                return ExitCode::from(2);
            }
            _ => path = Some(arg),
        }
    }
    let Some(path) = path else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    if x86 {
        let report = absint::x86::analyze(&bytes, base, entry.unwrap_or(base));
        for access in &report.accesses {
            let verdict = match access.verdict {
                absint::x86::StackVerdict::Safe => "safe",
                absint::x86::StackVerdict::MayOverwriteReturnAddress => {
                    "POSSIBLE overwrite of the return address"
                }
                absint::x86::StackVerdict::MayLeaveFrame => "may leave the frame",
                absint::x86::StackVerdict::UnknownPointer => {
                    "unknown pointer, so nothing is claimed"
                }
            };
            if all || access.verdict != absint::x86::StackVerdict::Safe {
                println!("{:#x}: {}: {verdict}", access.address, access.text);
            }
        }
        for at in &report.unbalanced_returns {
            println!("{at:#x}: POSSIBLE return with the stack pointer not restored");
        }
        for at in &report.unresolved_jumps {
            println!("{at:#x}: an indirect jump, and nothing past it was analyzed");
        }
        if report.incomplete {
            println!("incomplete: control reached something outside the code or undecodable");
        }
        println!(
            "{} instructions, {} safe writes to the stack",
            report.instructions,
            report.safe_writes()
        );
        let clean = report.dangerous().is_empty()
            && report.unbalanced_returns.is_empty()
            && !report.incomplete
            && report.unresolved_jumps.is_empty();
        return if clean {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    let (program, skipped) = if bytes.starts_with(b"\0asm") {
        match absint::wasm::lower(&bytes) {
            Ok(l) => (l.program, l.skipped),
            Err(e) => {
                eprintln!("{path}: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        let parsed = std::str::from_utf8(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|t| absint::text::parse(t).map_err(|e| e.to_string()));
        match parsed {
            Ok(p) => (p, Vec::new()),
            Err(e) => {
                eprintln!("{path}: {e}");
                return ExitCode::from(2);
            }
        }
    };
    let (proved, unproved) = check_functions(&program.functions, all);
    let races = check_races(&program);
    for (name, reason) in &skipped {
        println!("{name}: not analyzed, because {reason}");
    }
    println!(
        "{proved} checks proved, {unproved} not proved, {} functions not analyzed",
        skipped.len()
    );
    if unproved == 0 && races == 0 && skipped.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
