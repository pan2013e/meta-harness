use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    Extension,
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

use mh_acp::{ensure_member_worktree, member_worktree_path};
use mh_core::IsolationMode;

use crate::{
    AppState,
    auth::{SessionAccess, close_expired_socket},
};

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

pub(crate) async fn session_pty_ws_handler(
    ws: WebSocketUpgrade,
    AxumPath(session_id): AxumPath<String>,
    Query(q): Query<PtyQuery>,
    State(state): State<AppState>,
    Extension(access): Extension<SessionAccess>,
) -> Response {
    let cwd = match resolve_pty_cwd(&state, &session_id, q.target.as_deref()).await {
        Ok(cwd) => cwd,
        Err(error) => return error.into_response(),
    };
    let cols = q.cols.unwrap_or(80).clamp(20, 400);
    let rows = q.rows.unwrap_or(24).clamp(6, 200);
    ws.max_message_size(1024 * 1024)
        .max_frame_size(1024 * 1024)
        .on_upgrade(move |socket| handle_pty_socket(socket, cwd, cols, rows, access))
}

async fn resolve_pty_cwd(
    state: &AppState,
    session_id: &str,
    target: Option<&str>,
) -> Result<PathBuf, (StatusCode, String)> {
    let runtime = state.switch.runtime(session_id).await;
    let _lifecycle = runtime.lifecycle_lock.lock().await;
    let sess = state
        .switch
        .store
        .get_session(session_id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let repo_root = state
        .switch
        .resolve_session_repo_root(&sess)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let session_root = crate::routes::active_session_workspace(&sess, &repo_root)?;

    if let Some(handle) = target
        .map(str::trim)
        .filter(|h| !h.is_empty() && *h != "integration")
    {
        let member = state
            .switch
            .store
            .get_member(session_id, handle)
            .map_err(|_| {
                (
                    StatusCode::NOT_FOUND,
                    format!("Unknown terminal target @{handle}"),
                )
            })?;
        if let IsolationMode::WorktreePerMember = sess.isolation_mode {
            let path = member_worktree_path(&repo_root, session_id, &member.handle);
            if path.join(".git").is_file() {
                return Ok(path);
            }
            let branch = sess.integration_branch.as_deref().ok_or_else(|| {
                (
                    StatusCode::CONFLICT,
                    "The session integration worktree is unavailable".into(),
                )
            })?;
            let member_lock = {
                let mut locks = runtime.member_locks.lock().await;
                Arc::clone(locks.entry(member.handle.clone()).or_default())
            };
            let _idle = member_lock.try_lock().map_err(|_| {
                (
                    StatusCode::CONFLICT,
                    "The member is setting up its worktree. Try again in a moment.".into(),
                )
            })?;
            return ensure_member_worktree(&repo_root, session_id, branch, &member.handle)
                .await
                .map(|worktree| worktree.path)
                .map_err(|e| (StatusCode::CONFLICT, e.to_string()));
        }
    }
    Ok(session_root)
}

fn build_utf8_shell_command(cwd: &std::path::Path) -> CommandBuilder {
    #[cfg(windows)]
    let (prog, args): (String, Vec<String>) = if let Ok(ps) =
        which::which("pwsh.exe").or_else(|_| which::which("powershell.exe"))
    {
        (
                ps.to_string_lossy().into_owned(),
                vec![
                    "-NoLogo".into(),
                    "-NoExit".into(),
                    "-Command".into(),
                    r#"[Console]::InputEncoding = [System.Text.Encoding]::UTF8;
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8;
$OutputEncoding = [System.Text.Encoding]::UTF8;
chcp 65001 | Out-Null;
$global:MetaHarnessOriginalPrompt = $function:prompt;
function global:prompt {
    $label = & $global:MetaHarnessOriginalPrompt
    if ($PWD.Provider.Name -eq 'FileSystem') {
        [Console]::Write([string][char]27 + ']9;9;' + $PWD.ProviderPath + [char]7)
        if ($label -is [string] -and $PWD.ProviderPath -match '[\\/]\.meta-harness[\\/]worktrees[\\/][^\\/]+[\\/](?<tree>_integration|_exec_[^\\/]+)(?<tail>[\\/].*)?$') {
            $short = if ($Matches.tree -eq '_integration') { 'Integration' } else { '@' + $Matches.tree.Substring(6) }
            $label = $label.Replace($PWD.Path, $short + $Matches.tail)
        }
    }
    $label
}
Clear-Host"#
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

    let mut cmd = CommandBuilder::new(&prog);
    for (key, value) in mh_core::outbound_proxy::environment() {
        cmd.env(key, value);
    }
    cmd.args(args);
    #[cfg(unix)]
    match std::path::Path::new(&prog)
        .file_name()
        .and_then(|name| name.to_str())
    {
        Some("bash") => {
            let previous = std::env::var("PROMPT_COMMAND").unwrap_or_default();
            cmd.env(
                "PROMPT_COMMAND",
                format!("printf '\\033]9;9;%s\\007' \"$PWD\"\n{previous}"),
            );
        }
        Some("fish") => {
            cmd.args([
                "--init-command",
                "function __mh_report_cwd --on-event fish_prompt; printf '\\e]9;9;%s\\a' $PWD; end",
            ]);
        }
        _ => {}
    }
    cmd.cwd(dunce::simplified(cwd));
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("LANG", "en_US.UTF-8");
    cmd.env("LC_ALL", "en_US.UTF-8");
    cmd.env("PYTHONUTF8", "1");
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd
}

async fn handle_pty_socket(
    mut socket: WebSocket,
    cwd: PathBuf,
    cols: u16,
    rows: u16,
    access: SessionAccess,
) {
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

    let initial_cwd = json!({ "type": "cwd", "path": dunce::simplified(&cwd).to_string_lossy() });
    if socket
        .send(Message::Text(initial_cwd.to_string().into()))
        .await
        .is_err()
    {
        let _ = child.kill();
        return;
    }

    let Ok(mut reader) = pair.master.try_clone_reader() else {
        let _ = child.kill();
        return;
    };
    let Ok(mut writer) = pair.master.take_writer() else {
        let _ = child.kill();
        return;
    };
    let master = Arc::new(Mutex::new(pair.master));

    let (out_tx, mut out_rx) = mpsc::channel::<String>(32);
    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(32);

    // UTF-8 streaming reader keeping incomplete trailing CJK/multi-byte bytes across read calls
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    if !pending.is_empty() {
                        let tail = String::from_utf8_lossy(&pending).into_owned();
                        let _ = out_tx.blocking_send(tail);
                    }
                    break;
                }
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    let emitted = decode_utf8_stream_chunk(&mut pending);
                    if !emitted.is_empty() && out_tx.blocking_send(emitted).is_err() {
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

    let pump = async {
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
                                        if in_tx.try_send(data.into_bytes()).is_err() {
                                            break;
                                        }
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
                            if in_tx.try_send(bin.to_vec()).is_err() {
                                break;
                            }
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
    };
    let expired = tokio::select! {
        () = access.expires() => true,
        () = pump => false,
    };

    drop(in_tx);
    drop(out_rx);
    let _ = tokio::task::spawn_blocking(move || {
        let _ = child.kill();
        let _ = child.wait();
    })
    .await;
    if expired {
        close_expired_socket(&mut socket).await;
    }
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
        let subdir = tmp.path().join("next directory");
        std::fs::create_dir(&subdir).unwrap();
        let expect_cwd_update = cfg!(windows)
            && (which::which("pwsh.exe").is_ok() || which::which("powershell.exe").is_ok());
        let expected = format!(
            "\u{1b}]9;9;{}",
            dunce::canonicalize(subdir).unwrap().display()
        );
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
                let command = if expect_cwd_update {
                    "cd 'next directory'; echo 你好世界_PTY_OK\r"
                } else {
                    "echo 你好世界_PTY_OK\r"
                };
                let _ = writer.write_all(command.as_bytes());
                let _ = writer.flush();
            }
            match rx.recv_timeout(std::time::Duration::from_millis(150)) {
                Ok(chunk) => {
                    acc.push_str(&chunk);
                    let plain = strip_ansi_sequences(&acc);
                    if plain.contains("你好世界_PTY_OK")
                        && (!expect_cwd_update || acc.contains(&expected))
                    {
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
            "expected UTF-8 CJK text and an updated directory from PowerShell; cmd={cmd_prog} early_status={early_status:?} raw={acc:?} stripped={:?}",
            strip_ansi_sequences(&acc)
        );
    }
}
