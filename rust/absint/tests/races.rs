//! Locksets over the text format.

use absint::races::analyze;
use absint::text::parse;

fn races(source: &str) -> (usize, usize) {
    let report = analyze(&parse(source).unwrap());
    (report.races.len(), report.misuse.len())
}

const HEADER: &str = "global counter\nglobal other\nlock m\nlock n\n";

#[test]
fn a_shared_lock_proves_there_is_no_race() {
    let source = format!(
        "{HEADER}threads worker() {{\n lock m\n v = read counter\n v = v + 1\n write counter = v\n unlock m\n}}\nthread other() {{\n lock m\n write counter = 1\n unlock m\n}}"
    );
    assert_eq!(races(&source), (0, 0));
}

#[test]
fn a_missing_lock_is_a_race() {
    let source = format!(
        "{HEADER}thread a() {{\n write counter = 1\n}}\nthread b() {{\n lock m\n write counter = 2\n unlock m\n}}"
    );
    assert_eq!(races(&source).0, 1);
}

#[test]
fn different_locks_do_not_protect_anything() {
    let source = format!(
        "{HEADER}thread a() {{\n lock m\n write counter = 1\n unlock m\n}}\nthread b() {{\n lock n\n write counter = 2\n unlock n\n}}"
    );
    assert_eq!(races(&source).0, 1);
}

#[test]
fn reads_alone_do_not_race() {
    let source = format!(
        "{HEADER}thread a() {{\n x = read counter\n}}\nthread b() {{\n y = read counter\n}}"
    );
    assert_eq!(races(&source).0, 0);
    let source = format!(
        "{HEADER}thread a() {{\n x = read counter\n}}\nthread b() {{\n write counter = 1\n}}"
    );
    assert_eq!(races(&source).0, 1);
}

#[test]
fn different_globals_do_not_race() {
    let source = format!(
        "{HEADER}thread a() {{\n write counter = 1\n}}\nthread b() {{\n write other = 2\n}}"
    );
    assert_eq!(races(&source).0, 0);
}

#[test]
fn a_thread_only_races_with_itself_if_several_can_run() {
    assert_eq!(
        races(&format!("{HEADER}thread a() {{\n write counter = 1\n}}")).0,
        0
    );
    assert_eq!(
        races(&format!("{HEADER}threads a() {{\n write counter = 1\n}}")).0,
        1
    );
    // A function that is never started as a thread is not concurrent with anything.
    assert_eq!(
        races(&format!(
            "{HEADER}fn a() {{\n write counter = 1\n}}\nthread b() {{\n write counter = 2\n}}"
        ))
        .0,
        0
    );
}

#[test]
fn a_lock_held_on_only_one_path_does_not_count() {
    let source = format!(
        "{HEADER}thread a(c) {{\n if c < 0 goto yes else join\nyes:\n lock m\njoin:\n write counter = 1\n if c < 0 goto release else end\nrelease:\n unlock m\nend:\n return\n}}\nthread b() {{\n lock m\n write counter = 2\n unlock m\n}}"
    );
    let (found, _) = races(&source);
    assert_eq!(found, 1);
}

#[test]
fn access_after_release_is_not_protected() {
    let source = format!(
        "{HEADER}thread a() {{\n lock m\n unlock m\n write counter = 1\n}}\nthread b() {{\n lock m\n write counter = 2\n unlock m\n}}"
    );
    assert_eq!(races(&source).0, 1);
}

#[test]
fn a_lock_taken_in_a_loop_is_held_inside_it() {
    let source = format!(
        "{HEADER}threads a() {{\n i = 0\nhead:\n if i < 3 goto body else end\nbody:\n lock m\n write counter = i\n unlock m\n i = i + 1\n goto head\nend:\n return\n}}"
    );
    assert_eq!(races(&source), (0, 0));
}

#[test]
fn lock_misuse_is_reported() {
    assert_eq!(races(&format!("{HEADER}thread a() {{\n unlock m\n}}")).1, 1);
    assert_eq!(races(&format!("{HEADER}thread a() {{\n lock m\n}}")).1, 1);
}
