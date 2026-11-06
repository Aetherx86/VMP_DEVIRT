//! Static race detection by locksets.
//!
//! A race needs two accesses to one global from threads that can run at once, with at least one a
//! write, and with no lock held across both. The set of locks certainly held at each access comes from
//! a forward analysis in which merging paths intersects. If no pair qualifies, every access to every
//! global is protected by a common lock, which rules out data races on the globals.
//!
//! It ignores the start and join order of threads, so ordering that happens to prevent a race is still
//! reported as one: it can raise false alarms, but it does not miss a race among what it models.

use crate::ir::{Function, Instr, Program, ThreadKind};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    pub function: usize,
    pub block: usize,
    pub instr: usize,
    pub line: u32,
    pub global: u32,
    pub write: bool,
    pub locks: BTreeSet<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Race {
    pub global: u32,
    pub first: Access,
    pub second: Access,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockMisuse {
    pub function: usize,
    pub line: u32,
    pub lock: u32,
    /// Releasing a lock that may not be held, or holding one at return.
    pub message: &'static str,
}

#[derive(Debug, Default)]
pub struct RaceReport {
    pub accesses: usize,
    pub races: Vec<Race>,
    pub misuse: Vec<LockMisuse>,
}

fn locksets(function: &Function) -> Vec<Option<BTreeSet<u32>>> {
    let n = function.blocks.len();
    let mut entry: Vec<Option<BTreeSet<u32>>> = vec![None; n];
    entry[0] = Some(BTreeSet::new());
    let mut pending = vec![0];
    while let Some(block) = pending.pop() {
        let Some(mut held) = entry[block].clone() else {
            continue;
        };
        for (instr, _) in &function.blocks[block].instrs {
            match instr {
                Instr::Lock(l) => {
                    held.insert(*l);
                }
                Instr::Unlock(l) => {
                    held.remove(l);
                }
                _ => {}
            }
        }
        for target in function.successors(block) {
            let updated = match &entry[target] {
                None => Some(held.clone()),
                Some(old) => {
                    let meet: BTreeSet<u32> = old.intersection(&held).copied().collect();
                    (meet != *old).then_some(meet)
                }
            };
            if let Some(set) = updated {
                entry[target] = Some(set);
                pending.push(target);
            }
        }
    }
    entry
}

pub fn analyze(program: &Program) -> RaceReport {
    let mut report = RaceReport::default();
    let mut accesses: Vec<Access> = Vec::new();
    for (index, function) in program.functions.iter().enumerate() {
        if function.thread == ThreadKind::None {
            continue;
        }
        let entry = locksets(function);
        for (b, block) in function.blocks.iter().enumerate() {
            let Some(mut held) = entry[b].clone() else {
                continue;
            };
            for (i, (instr, line)) in block.instrs.iter().enumerate() {
                match instr {
                    Instr::Lock(l) => {
                        held.insert(*l);
                    }
                    Instr::Unlock(l) => {
                        if !held.remove(l) {
                            report.misuse.push(LockMisuse {
                                function: index,
                                line: *line,
                                lock: *l,
                                message: "releases a lock that may not be held",
                            });
                        }
                    }
                    Instr::ReadGlobal { global, .. } | Instr::WriteGlobal { global, .. } => {
                        accesses.push(Access {
                            function: index,
                            block: b,
                            instr: i,
                            line: *line,
                            global: *global,
                            write: matches!(instr, Instr::WriteGlobal { .. }),
                            locks: held.clone(),
                        })
                    }
                    _ => {}
                }
            }
            if function.successors(b).is_empty() {
                for l in &held {
                    report.misuse.push(LockMisuse {
                        function: index,
                        line: 0,
                        lock: *l,
                        message: "returns while still holding a lock",
                    });
                }
            }
        }
    }
    report.accesses = accesses.len();
    for (i, a) in accesses.iter().enumerate() {
        for b in &accesses[i..] {
            let same_thread_code = a.function == b.function;
            let can_overlap =
                !same_thread_code || program.functions[a.function].thread == ThreadKind::Many;
            if a.global == b.global
                && (a.write || b.write)
                && can_overlap
                && a.locks.is_disjoint(&b.locks)
            {
                report.races.push(Race {
                    global: a.global,
                    first: a.clone(),
                    second: b.clone(),
                });
            }
        }
    }
    report
}
