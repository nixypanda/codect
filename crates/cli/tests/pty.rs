// PTY smoke tests for the real crossterm path.
//
// The rest of the frontend is covered with an injected driver and
// `TestBackend`; these are the only tests that drive an actual terminal. They
// are skipped on platforms without a PTY, which is why they are gated to Unix.

#![cfg(unix)]

mod support;

use std::io::Write;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use support::TestRepo;

const RUST_BASE: &str = "\
pub struct User {
    pub id: u32,
}
";

const RUST_TYPE_VARIANT: &str = "\
pub struct User {
    pub id: u64,
}
";

fn run_in_pty(repo: &TestRepo, args: &[&str]) -> u32 {
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open a pty");

    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_codect"));
    command.cwd(repo.path());
    command.args(args);

    let mut child = pair.slave.spawn_command(command).expect("spawn codect");
    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let mut writer = pair.master.take_writer().expect("pty writer");
    drop(pair);

    // Drain output so a full pty buffer cannot block the child.
    let drain = std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = std::io::copy(&mut reader, &mut sink);
        sink
    });

    // Give the frontend time to enter the alternate screen and start reading.
    // The startup color query reads standard input before the event loop begins,
    // so a key written during that window would be consumed by the query; wait
    // for startup to finish first. A key written earlier would otherwise be
    // buffered by the pty and read when it is ready.
    std::thread::sleep(Duration::from_millis(2500));
    writer.write_all(b"q").expect("write quit key");
    let _ = writer.flush();

    let status = child.wait().expect("wait for codect");
    let output = drain.join().unwrap_or_default();
    assert!(
        !output.is_empty(),
        "the frontend should have drawn something before quitting"
    );
    status.exit_code()
}

#[test]
fn tui_show_starts_and_quits_in_a_pty() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");

    assert_eq!(run_in_pty(&repo, &["tui", "show", "--mode", "types"]), 0);
}

#[test]
fn tui_diff_starts_and_quits_in_a_pty() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/lib.rs", RUST_TYPE_VARIANT);
    repo.commit("type change");

    assert_eq!(
        run_in_pty(
            &repo,
            &["tui", "diff", "range", "--mode", "types", "HEAD~1", "HEAD"]
        ),
        0
    );
    assert_eq!(
        run_in_pty(
            &repo,
            &[
                "tui", "diff", "commits", "--mode", "types", "HEAD~1", "HEAD"
            ]
        ),
        0
    );
}
