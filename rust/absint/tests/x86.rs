//! Stack safety of hand-assembled x86-64.

use absint::x86::{StackVerdict, analyze};

/// `push rbp; mov rbp, rsp; sub rsp, 32`, the usual function start.
const PROLOGUE: &[u8] = &[0x55, 0x48, 0x89, 0xE5, 0x48, 0x83, 0xEC, 0x20];
/// `leave; ret`.
const EPILOGUE: &[u8] = &[0xC9, 0xC3];

fn function(body: &[u8]) -> Vec<u8> {
    [PROLOGUE, body, EPILOGUE].concat()
}

fn writes_verdicts(code: &[u8]) -> Vec<StackVerdict> {
    analyze(code, 0x1000, 0x1000)
        .accesses
        .iter()
        .filter(|a| a.write)
        .map(|a| a.verdict)
        .collect()
}

#[test]
fn a_balanced_function_with_local_writes_is_safe() {
    // mov dword [rbp-4], 1
    let code = function(&[0xC7, 0x45, 0xFC, 1, 0, 0, 0]);
    let report = analyze(&code, 0x1000, 0x1000);
    assert!(report.dangerous().is_empty(), "{report:#?}");
    assert!(report.unbalanced_returns.is_empty(), "{report:#?}");
    assert!(!report.incomplete);
    assert_eq!(
        report.safe_writes(),
        2,
        "the pushed register and the local: {report:#?}"
    );
    assert_eq!(report.instructions, 6);
}

#[test]
fn a_write_to_the_return_address_is_caught() {
    // mov qword [rbp+8], 0: the return address itself, since rbp sits 8 below it.
    let code = function(&[0x48, 0xC7, 0x45, 0x08, 0, 0, 0, 0]);
    let report = analyze(&code, 0x1000, 0x1000);
    assert_eq!(report.dangerous().len(), 1, "{report:#?}");
    // One byte below the return address is the saved frame pointer, which is not the same thing.
    let code = function(&[0xC6, 0x45, 0x07, 0]);
    assert_eq!(analyze(&code, 0x1000, 0x1000).dangerous().len(), 0);
    let code = function(&[0xC6, 0x45, 0x08, 0]);
    assert_eq!(analyze(&code, 0x1000, 0x1000).dangerous().len(), 1);
}

/// `for (i = 0; i < LIMIT; i++) buf[i] = 0`, with `buf` at `rbp-16` and `i` in eax.
fn byte_loop(limit: u8) -> Vec<u8> {
    function(&[
        0x31, 0xC0, // xor eax, eax
        // head:
        0x83, 0xF8, limit, // cmp eax, limit
        0x7D, 0x0A, // jge done
        0xC6, 0x44, 0x05, 0xF0, 0x00, // mov byte [rbp+rax-16], 0
        0x83, 0xC0, 0x01, // add eax, 1
        0xEB, 0xF1, // jmp head
              // done:
    ])
}

#[test]
fn a_loop_within_its_buffer_is_proved_safe() {
    let report = analyze(&byte_loop(16), 0x1000, 0x1000);
    assert!(report.dangerous().is_empty(), "{report:#?}");
    assert!(
        report
            .accesses
            .iter()
            .all(|a| a.verdict == StackVerdict::Safe),
        "{report:#?}"
    );
}

#[test]
fn a_loop_running_past_its_buffer_is_caught() {
    // Index 23 is at rbp+7, still inside the saved frame pointer.
    assert!(
        analyze(&byte_loop(24), 0x1000, 0x1000)
            .dangerous()
            .is_empty()
    );
    // Index 24 is at rbp+8, the first byte of the return address.
    assert_eq!(analyze(&byte_loop(25), 0x1000, 0x1000).dangerous().len(), 1);
}

#[test]
fn an_unbounded_index_is_caught() {
    // mov byte [rbp+rdi-16], 0, where rdi is an argument.
    let report = analyze(&function(&[0xC6, 0x44, 0x3D, 0xF0, 0x00]), 0x1000, 0x1000);
    let unknown = report
        .accesses
        .iter()
        .find(|a| a.text.contains("rdi"))
        .unwrap();
    // An index with no known range cannot be added to a stack pointer with any confidence.
    assert_eq!(unknown.verdict, StackVerdict::UnknownPointer, "{report:#?}");
}

#[test]
fn a_masked_index_is_proved_safe_and_a_mask_too_wide_is_not() {
    let masked = |mask: u8| {
        function(&[
            0x89, 0xF8, // mov eax, edi
            0x83, 0xE0, mask, // and eax, mask
            0xC6, 0x44, 0x05, 0xE0, 0x00, // mov byte [rbp+rax-32], 0
        ])
    };
    assert!(analyze(&masked(31), 0x1000, 0x1000).dangerous().is_empty());
    assert_eq!(analyze(&masked(127), 0x1000, 0x1000).dangerous().len(), 1);
}

#[test]
fn stack_slots_carry_values_like_unoptimized_code_does() {
    // mov dword [rbp-4], 8; mov eax, [rbp-4]; mov byte [rbp+rax-32], 0
    let safe = function(&[
        0xC7, 0x45, 0xFC, 8, 0, 0, 0, 0x8B, 0x45, 0xFC, 0xC6, 0x44, 0x05, 0xE0, 0,
    ]);
    assert!(
        analyze(&safe, 0x1000, 0x1000)
            .accesses
            .iter()
            .all(|a| a.verdict == StackVerdict::Safe)
    );
    // The same, with 40 in the slot, which reaches the return address.
    let bad = function(&[
        0xC7, 0x45, 0xFC, 40, 0, 0, 0, 0x8B, 0x45, 0xFC, 0xC6, 0x44, 0x05, 0xE0, 0,
    ]);
    assert_eq!(analyze(&bad, 0x1000, 0x1000).dangerous().len(), 1);
    // A write between replaces it: mov dword [rbp-4], 40; mov dword [rbp-4], 8.
    let replaced = function(&[
        0xC7, 0x45, 0xFC, 40, 0, 0, 0, 0xC7, 0x45, 0xFC, 8, 0, 0, 0, 0x8B, 0x45, 0xFC, 0xC6, 0x44,
        0x05, 0xE0, 0,
    ]);
    assert!(analyze(&replaced, 0x1000, 0x1000).dangerous().is_empty());
}

#[test]
fn a_bounds_check_on_a_stack_slot_counts() {
    // for (i = 0; i <= 15; i++) buf[i] = 0, with i kept at [rbp-4] and tested in place as gcc -O0 does.
    let code = function(&[
        0xC7, 0x45, 0xFC, 0, 0, 0, 0, // mov dword [rbp-4], 0
        // head:
        0x83, 0x7D, 0xFC, 0x0F, // cmp dword [rbp-4], 15
        0x7F, 0x0E, // jg done
        0x8B, 0x45, 0xFC, // mov eax, [rbp-4]
        0xC6, 0x44, 0x05, 0xE0, 0x00, // mov byte [rbp+rax-32], 0
        0x83, 0x45, 0xFC, 0x01, // add dword [rbp-4], 1
        0xEB, 0xEC, // jmp head
              // done:
    ]);
    let report = analyze(&code, 0x1000, 0x1000);
    assert!(report.dangerous().is_empty(), "{report:#?}");
    assert!(
        report
            .accesses
            .iter()
            .all(|a| a.verdict == StackVerdict::Safe),
        "{report:#?}"
    );
    assert!(!report.incomplete, "{report:#?}");
}

#[test]
fn the_size_of_an_access_counts() {
    // rbp+1 is 7 bytes below the return address, so a byte is safe and a qword runs into it.
    assert!(
        analyze(&function(&[0xC6, 0x45, 0x01, 0]), 0x1000, 0x1000)
            .dangerous()
            .is_empty()
    );
    let qword = function(&[0x48, 0xC7, 0x45, 0x01, 0, 0, 0, 0]);
    assert_eq!(analyze(&qword, 0x1000, 0x1000).dangerous().len(), 1);
}

#[test]
fn a_signed_check_says_nothing_about_a_value_that_may_be_negative() {
    // if ((int)edi < 16) buf[(unsigned)edi] = 0, where a negative edi is a huge index.
    let code = function(&[
        0x89, 0xF8, // mov eax, edi
        0x83, 0xF8, 0x10, // cmp eax, 16
        0x7D, 0x05, // jge done
        0xC6, 0x44, 0x05, 0xE0, 0x00, // mov byte [rbp+rax-32], 0
              // done:
    ]);
    assert_eq!(analyze(&code, 0x1000, 0x1000).dangerous().len(), 1);
    // An unsigned check on the same value does prove it.
    let code = function(&[
        0x89, 0xF8, 0x83, 0xF8, 0x10, 0x73, 0x05, 0xC6, 0x44, 0x05, 0xE0, 0x00,
    ]);
    assert!(analyze(&code, 0x1000, 0x1000).dangerous().is_empty());
}

#[test]
fn unbalanced_returns_are_reported() {
    // push rax; ret, which returns to whatever rax held.
    let report = analyze(&[0x50, 0xC3], 0, 0);
    assert_eq!(report.unbalanced_returns, [1]);
    // sub rsp, 8; add rsp, 8; ret is balanced.
    let report = analyze(&[0x48, 0x83, 0xEC, 8, 0x48, 0x83, 0xC4, 8, 0xC3], 0, 0);
    assert!(report.unbalanced_returns.is_empty());
    // A stack pointer that came from nowhere.
    let report = analyze(&[0x48, 0x89, 0xFC, 0xC3], 0, 0);
    assert_eq!(report.unbalanced_returns, [3]);
}

#[test]
fn aligning_the_stack_is_understood() {
    // sub rsp, 16; and rsp, -16; mov byte [rsp], 0. Aligning can only move further down.
    let code = [
        0x48, 0x83, 0xEC, 0x10, 0x48, 0x83, 0xE4, 0xF0, 0xC6, 0x04, 0x24, 0x00, 0xC3,
    ];
    let report = analyze(&code, 0, 0);
    assert!(report.dangerous().is_empty(), "{report:#?}");
    assert_eq!(
        report.unbalanced_returns.len(),
        1,
        "the alignment changed the stack pointer"
    );
    // With no room made first, an already aligned stack pointer still points at the return address.
    let code = [0x48, 0x83, 0xE4, 0xF0, 0xC6, 0x04, 0x24, 0x00, 0xC3];
    assert_eq!(analyze(&code, 0, 0).dangerous().len(), 1);
}

#[test]
fn calls_forget_what_a_callee_may_change() {
    // mov eax, 8; call +0; mov byte [rbp+rax-32], 0. After a call, eax is unknown.
    let code = function(&[
        0xB8, 8, 0, 0, 0, 0xE8, 0, 0, 0, 0, 0xC6, 0x44, 0x05, 0xE0, 0,
    ]);
    let report = analyze(&code, 0x1000, 0x1000);
    assert!(
        report
            .accesses
            .iter()
            .any(|a| a.verdict != StackVerdict::Safe),
        "{report:#?}"
    );
    // rbx is preserved across a call.
    let code = function(&[
        0xBB, 8, 0, 0, 0, 0xE8, 0, 0, 0, 0, 0xC6, 0x44, 0x1D, 0xE0, 0,
    ]);
    let report = analyze(&code, 0x1000, 0x1000);
    assert!(
        report
            .accesses
            .iter()
            .all(|a| a.verdict == StackVerdict::Safe),
        "{report:#?}"
    );
}

#[test]
fn writes_through_unknown_pointers_claim_nothing_and_forget_nothing_silently() {
    // mov [rdi], eax
    let report = analyze(&[0x89, 0x07, 0xC3], 0, 0);
    assert_eq!(
        writes_verdicts(&[0x89, 0x07, 0xC3]),
        [StackVerdict::UnknownPointer]
    );
    assert!(report.dangerous().is_empty());
}

#[test]
fn indirect_jumps_and_undecodable_code_are_reported() {
    // jmp rax
    let report = analyze(&[0xFF, 0xE0], 0, 0);
    assert_eq!(report.unresolved_jumps, [0]);
    // Running off the end of the code.
    assert!(analyze(&[0x90], 0, 0).incomplete);
    assert!(
        analyze(&[0x0F, 0x0B], 0, 0).accesses.is_empty(),
        "ud2 ends the path"
    );
    assert!(analyze(&[0x48], 0, 0).incomplete);
}

#[test]
fn instructions_it_does_not_model_forget_what_they_write() {
    // mov ecx, 8; cpuid changes ecx; mov byte [rbp+rcx-32], 0.
    let code = function(&[0xB9, 8, 0, 0, 0, 0x0F, 0xA2, 0xC6, 0x44, 0x0D, 0xE0, 0]);
    let report = analyze(&code, 0x1000, 0x1000);
    assert!(
        report
            .accesses
            .iter()
            .any(|a| a.verdict != StackVerdict::Safe),
        "{report:#?}"
    );
}

#[test]
fn random_bytes_never_panic_or_hang() {
    let mut state = 0xDEAD_BEEF_0123_4567u64;
    for _ in 0..3_000 {
        let bytes: Vec<u8> = (0..64)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        analyze(&bytes, 0, 0);
    }
}
