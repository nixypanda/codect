//! A PTY smoke test for the real crossterm path.
//!
//! The rest of the frontend is covered with an injected driver and
//! `TestBackend`; this is the only test that drives an actual terminal. It is
//! skipped on platforms without a PTY, which is why it is gated to Unix.

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

#[test]
fn tui_show_starts_and_quits_in_a_pty() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");

    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open a pty");

    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_ownai"));
    command.cwd(repo.path());
    command.args(["tui", "show", "--mode", "types"]);

    let mut child = pair.slave.spawn_command(command).expect("spawn ownai");
    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let mut writer = pair.master.take_writer().expect("pty writer");
    drop(pair);

    // Drain output so a full pty buffer cannot block the child.
    let drain = std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = std::io::copy(&mut reader, &mut sink);
        sink
    });

    // Give the frontend time to enter the alternate screen and start reading;
    // a `q` written earlier is buffered by the pty and read when it is ready.
    std::thread::sleep(Duration::from_millis(800));
    writer.write_all(b"q").expect("write quit key");
    let _ = writer.flush();

    let status = child.wait().expect("wait for ownai");
    let output = drain.join().unwrap_or_default();

    assert_eq!(
        status.exit_code(),
        0,
        "`ownai tui show` should quit cleanly; output was:\n{}",
        String::from_utf8_lossy(&output)
    );
}
