use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{
        Path as AxumPath, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use mh_acp::ensure_member_worktree;
use mh_core::IsolationMode;

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct PtyQuery {
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub target: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientPtyMsg {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

pub async fn session_pty_ws_handler(
    ws: WebSocketUpgrade,
    AxumPath(session_id): AxumPath<String>,
    Query(q): Query<PtyQuery>,
    State(state): State<AppState>,
) -> Response {
    let Ok(cwd) = resolve_pty_cwd(&state, &session_id, q.target.as_deref()).await else {
        return (StatusCode::NOT_FOUND, "session or worktree not found").into_response();
    };
    let cols = q.cols.unwrap_or(80).clamp(20, 400);
    let rows = q.rows.unwrap_or(24).clamp(6, 200);
    ws.on_upgrade(move |socket| handle_pty_socket(socket, cwd, cols, rows))
}

async fn resolve_pty_cwd(
    state: &AppState,
    session_id: &str,
    target: Option<&str>,
) -> Result<PathBuf, ()> {
    let sess = state.switch.store.get_session(session_id).map_err(|_| ())?;
    let repo_root = state
        .switch
        .resolve_session_repo_root(&sess)
        .map_err(|_| ())?;

    if let Some(handle) = target
        .map(str::trim)
        .filter(|h| !h.is_empty() && *h != "integration")
        && let Ok(member) = state.switch.store.get_member(session_id, handle)
    {
        if let Some(ref p) = member.worktree_path {
            let pb = PathBuf::from(p);
            if pb.is_dir() {
                return Ok(pb);
            }
        }
        if sess.isolation_mode == IsolationMode::WorktreePerMember
            && let Some(ref int_br) = sess.integration_branch
            && let Ok(m_wt) =
                ensure_member_worktree(&repo_root, session_id, int_br, &member.handle).await
        {
            return Ok(m_wt.path);
        }
    }

    if let Some(ref int_wt) = sess.integration_wt_path {
        let pb = PathBuf::from(int_wt);
        if pb.is_dir() {
            return Ok(pb);
        }
    }
    if repo_root.is_dir() {
        return Ok(repo_root);
    }
    Err(())
}

fn build_utf8_shell_command(cwd: &std::path::Path) -> CommandBuilder {
    #[cfg(windows)]
    let (prog, args): (String, Vec<String>) = if let Ok(pwsh) = which::which("pwsh.exe") {
        (
            pwsh.to_string_lossy().into_owned(),
            vec![
                "-NoLogo".into(),
                "-NoExit".into(),
                "-Command".into(),
                "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; \
                 [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
                 $OutputEncoding = [System.Text.Encoding]::UTF8; \
                 chcp 65001 | Out-Null; \
                 Clear-Host"
                    .into(),
            ],
        )
    } else if let Ok(ps) = which::which("powershell.exe") {
        (
            ps.to_string_lossy().into_owned(),
            vec![
                "-NoLogo".into(),
                "-NoExit".into(),
                "-Command".into(),
                "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; \
                 [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
                 $OutputEncoding = [System.Text.Encoding]::UTF8; \
                 chcp 65001 | Out-Null; \
                 Clear-Host"
                    .into(),
            ],
        )
    } else {
        (
            "cmd.exe".into(),
            vec!["/K".into(), "chcp 65001 >nul".into()],
        )
    };

    #[cfg(not(windows))]
    let (prog, args): (String, Vec<String>) = (
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        Vec::new(),
    );

    let mut cmd = CommandBuilder::new(prog);
    cmd.args(args);
    cmd.cwd(dunce::simplified(cwd));
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("LANG", "en_US.UTF-8");
    cmd.env("LC_ALL", "en_US.UTF-8");
    cmd.env("PYTHONUTF8", "1");
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd
}

async fn handle_pty_socket(mut socket: WebSocket, cwd: PathBuf, cols: u16, rows: u16) {
    let pty_system = native_pty_system();
    let Ok(pair) = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }) else {
        let _ = socket
            .send(Message::Text(
                json!({ "type": "output", "data": "\r\n[Failed to allocate PTY]\r\n" })
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    };

    let cmd = build_utf8_shell_command(&cwd);
    let Ok(mut child) = pair.slave.spawn_command(cmd) else {
        let _ = socket
            .send(Message::Text(
                json!({ "type": "output", "data": "\r\n[Failed to spawn shell process]\r\n" })
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    };
    drop(pair.slave);

    let Ok(mut reader) = pair.master.try_clone_reader() else {
        let _ = child.kill();
        return;
    };
    let Ok(mut writer) = pair.master.take_writer() else {
        let _ = child.kill();
        return;
    };
    let master = Arc::new(Mutex::new(pair.master));

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // UTF-8 streaming reader keeping incomplete trailing CJK/multi-byte bytes across read calls
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    if !pending.is_empty() {
                        let tail = String::from_utf8_lossy(&pending).into_owned();
                        let _ = out_tx.send(tail);
                    }
                    break;
                }
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    let emitted = decode_utf8_stream_chunk(&mut pending);
                    if !emitted.is_empty() && out_tx.send(emitted).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    std::thread::spawn(move || {
        while let Some(bytes) = in_rx.blocking_recv() {
            if writer.write_all(&bytes).is_err() || writer.flush().is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            maybe_out = out_rx.recv() => {
                match maybe_out {
                    Some(text) => {
                        let frame = json!({ "type": "output", "data": text }).to_string();
                        if socket.send(Message::Text(frame.into())).await.is_err() {
                            break;
                        }
                    }
                    None => {
                        let _ = socket
                            .send(Message::Text(
                                json!({ "type": "exit" }).to_string().into(),
                            ))
                            .await;
                        break;
                    }
                }
            }
            ws_msg = socket.recv() => {
                match ws_msg {
                    Some(Ok(Message::Text(txt))) => {
                        if let Ok(parsed) = serde_json::from_str::<ClientPtyMsg>(&txt) {
                            match parsed {
                                ClientPtyMsg::Input { data } => {
                                    let _ = in_tx.send(data.into_bytes());
                                }
                                ClientPtyMsg::Resize { cols, rows } => {
                                    if let Ok(m) = master.lock() {
                                        let _ = m.resize(PtySize {
                                            rows: rows.clamp(6, 200),
                                            cols: cols.clamp(20, 400),
                                            pixel_width: 0,
                                            pixel_height: 0,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Binary(bin))) => {
                        let _ = in_tx.send(bin.to_vec());
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = socket.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    _ => {}
                }
            }
        }
    }

    drop(in_tx);
    let _ = child.kill();
    let _ = child.wait();
}

pub fn decode_utf8_stream_chunk(pending: &mut Vec<u8>) -> String {
    let mut consumed = 0usize;
    let mut emitted = String::new();
    loop {
        match std::str::from_utf8(&pending[consumed..]) {
            Ok(valid) => {
                emitted.push_str(valid);
                consumed = pending.len();
                break;
            }
            Err(err) => {
                let valid_len = err.valid_up_to();
                if valid_len > 0
                    && let Ok(valid) = std::str::from_utf8(&pending[consumed..consumed + valid_len])
                {
                    emitted.push_str(valid);
                }
                consumed += valid_len;
                if let Some(bad_len) = err.error_len() {
                    emitted.push('\u{FFFD}');
                    consumed += bad_len;
                } else {
                    // Incomplete trailing UTF-8 sequence; preserve for next read
                    break;
                }
            }
        }
    }
    if consumed > 0 {
        pending.drain(..consumed);
    }
    emitted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi_sequences(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                match chars.peek().copied() {
                    Some('[') => {
                        chars.next();
                        for next in chars.by_ref() {
                            if ('@'..='~').contains(&next) {
                                break;
                            }
                        }
                    }
                    Some(']') => {
                        chars.next();
                        while let Some(next) = chars.next() {
                            if next == '\x07' {
                                break;
                            }
                            if next == '\x1b' && chars.peek() == Some(&'\\') {
                                chars.next();
                                break;
                            }
                        }
                    }
                    Some(_) => {
                        chars.next();
                    }
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn utf8_stream_decoder_preserves_split_cjk_sequences() {
        let sample = "你好世界 — Meta-Harness 终端";
        let bytes = sample.as_bytes();
        let mut pending = Vec::new();
        let mut decoded = String::new();

        for chunk in bytes.chunks(2) {
            pending.extend_from_slice(chunk);
            decoded.push_str(&decode_utf8_stream_chunk(&mut pending));
        }

        assert!(pending.is_empty());
        assert_eq!(decoded, sample);
        assert!(!decoded.contains('\u{FFFD}'));
    }

    #[test]
    fn pty_round_trips_utf8_cjk_text() {
        let tmp = tempfile::tempdir().unwrap();
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let cmd = build_utf8_shell_command(tmp.path());
        let cmd_prog = format!("{cmd:?}");
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();

        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut pending = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        if !pending.is_empty() {
                            let _ = tx.send(String::from_utf8_lossy(&pending).into_owned());
                        }
                        break;
                    }
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        let text = decode_utf8_stream_chunk(&mut pending);
                        if !text.is_empty() && tx.send(text).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(format!("[READ_ERR:{e}]"));
                        break;
                    }
                }
            }
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
        let mut acc = String::new();
        let mut matched = false;
        let mut cmd_sent_at: Option<std::time::Instant> = None;
        let mut dsr_replies = 0usize;
        while std::time::Instant::now() < deadline {
            if acc.matches("\x1b[6n").count() > dsr_replies {
                dsr_replies = acc.matches("\x1b[6n").count();
                let _ = writer.write_all(b"\x1b[1;1R");
                let _ = writer.flush();
            }
            let plain_now = strip_ansi_sequences(&acc);
            if cmd_sent_at.is_none()
                && (plain_now.contains('>')
                    || plain_now.contains('$')
                    || plain_now.contains('#')
                    || std::time::Instant::now() + std::time::Duration::from_secs(9) < deadline)
            {
                cmd_sent_at = Some(std::time::Instant::now());
                let _ = writer
                    .write_all(b"echo \xE4\xBD\xA0\xE5\xA5\xBD\xE4\xB8\x96\xE7\x95\x8C_PTY_OK\r");
                let _ = writer.flush();
            }
            match rx.recv_timeout(std::time::Duration::from_millis(150)) {
                Ok(chunk) => {
                    acc.push_str(&chunk);
                    let plain = strip_ansi_sequences(&acc);
                    if plain.contains("你好世界_PTY_OK") {
                        matched = true;
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        let early_status = child.try_wait();
        let _ = writer.write_all(b"exit\r\n");
        let _ = writer.flush();
        let _ = child.kill();
        let _ = child.wait();

        assert!(
            matched,
            "expected UTF-8 CJK string in PTY output; cmd={cmd_prog} early_status={early_status:?} raw={acc:?} stripped={:?}",
            strip_ansi_sequences(&acc)
        );
    }
}
