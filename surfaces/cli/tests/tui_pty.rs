#![cfg(unix)]

use std::process::Command;
use std::time::Duration;

use expectrl::{Eof, Expect};

/// 冷启动必须走完真实 PTY：没有预启动 daemon，也不复用用户 Runtime。
/// 这覆盖内嵌 daemon 的 Session 生命周期，而不只是视图状态机。
#[test]
fn cold_start_shows_an_empty_now_screen_and_quits_cleanly() {
    let runtime = tempfile::tempdir().expect("temporary runtime directory");
    let mut command = Command::new(env!("CARGO_BIN_EXE_else-bot"));
    command
        .arg("--dir")
        .arg(runtime.path())
        .env("TERM", "xterm-256color");
    let mut terminal = expectrl::session::OsSession::spawn(command).expect("spawn TUI in a PTY");
    terminal.set_expect_timeout(Some(Duration::from_secs(20)));

    // 增量重绘会把一行拆成几段写入，所以这里只认单词；状态条本身的排版
    // 由 views 里的帧测试负责。
    terminal.expect("else").expect("status line");
    // 空状态不摆空表格，而是给下一个动作。
    terminal
        .expect("Nothing is running yet.")
        .expect("loaded empty state proves the embedded runtime stayed alive");
    terminal
        .expect("Press n and describe an outcome")
        .expect("empty state teaches the next action");
    terminal.send("q").expect("quit key");
    terminal.expect(Eof).expect("clean process exit");

    // 如果这里 panic，tempdir 仍在作用域内；不能留下后台 Runtime 或测试数据。
    drop(terminal);
}

#[test]
fn thread_can_be_created_through_the_real_terminal() {
    let runtime = tempfile::tempdir().expect("temporary runtime directory");
    let mut command = Command::new(env!("CARGO_BIN_EXE_else-bot"));
    command
        .arg("--dir")
        .arg(runtime.path())
        .env("TERM", "xterm-256color");
    let mut terminal = expectrl::session::OsSession::spawn(command).expect("spawn TUI in a PTY");
    terminal.set_expect_timeout(Some(Duration::from_secs(20)));

    terminal
        .expect("Nothing is running yet.")
        .expect("Now screen loaded");
    terminal.send("n").expect("open composer");
    terminal
        .expect("What should else take care of?")
        .expect("composer visible");
    terminal
        .send("Verify the terminal product flow")
        .expect("enter outcome");
    terminal.send("\r").expect("submit thread");
    // 提交之后直接落到那条 Thread 上，而不是被丢回列表再自己找。
    terminal
        .expect("works in")
        .expect("the new thread opens with its facts");
    terminal
        .expect("nothing produced yet")
        .expect("a fresh thread says so plainly");
    terminal.send("q").expect("quit key");
    terminal.expect(Eof).expect("clean process exit");

    // 增量 ANSI 重绘可能把一个文本 Span 拆成多段，持久状态比终端字节匹配更可靠。
    let list = Command::new(env!("CARGO_BIN_EXE_else-bot"))
        .arg("--dir")
        .arg(runtime.path())
        .args(["list", "--json"])
        .output()
        .expect("list persisted threads");
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("Verify the terminal product flow"));
}
