//! The command itself.

use std::process::Command;

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wasm-superopt"))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wasm-superopt-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn it_writes_a_smaller_valid_module_and_reports_on_it() {
    let input = scratch("in.wasm");
    let output = scratch("out.wasm");
    let wasm = wat::parse_str(
        "(module (func (export \"f\") (param i32 i32) (result i32)
            local.get 0 local.get 1 i32.xor  local.get 0 local.get 1 i32.and i32.const 2 i32.mul  i32.add))",
    )
    .unwrap();
    std::fs::write(&input, &wasm).unwrap();
    let run = command()
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let text = String::from_utf8(run.stdout).unwrap();
    assert!(text.contains("MBA simplified:     1"), "{text}");
    let written = std::fs::read(&output).unwrap();
    assert!(written.len() < wasm.len());
    wasmparser::validate(&written).unwrap();
    assert!(
        text.contains(&format!("{} -> {}", wasm.len(), written.len())),
        "{text}"
    );
}

#[test]
fn it_fails_clearly() {
    let missing = command().arg("/no/such/file.wasm").output().unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("cannot read"));

    let junk = scratch("junk.wasm");
    std::fs::write(&junk, b"this is not wasm").unwrap();
    let invalid = command().arg(&junk).output().unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("invalid module"));

    assert!(!command().output().unwrap().status.success(), "no input");
    assert!(
        !command()
            .arg("--nonsense")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !command()
            .args(["a", "b"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !command()
            .args(["a", "-o"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn it_explains_itself() {
    let help = command().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("usage:"));
    let version = command().arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&version.stdout).contains(env!("CARGO_PKG_VERSION")));
}
