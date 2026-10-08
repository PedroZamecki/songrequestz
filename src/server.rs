//! Songify's local API on 127.0.0.1:65530 (port configurable), so Songify overlays and tools work
//! unchanged: plain HTTP GET answers the now-playing JSON, the WebSocket at /ws/data pushes it on
//! every change, and a WebSocket on any other path takes commands
//! ({"action", "data", "password"}, answered "Command executed: ...").

use crate::{say, Config};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

/// A WebSocket command for main.rs: action, data, and where its answer goes.
pub type Command = (String, Value, oneshot::Sender<String>);

pub async fn run(
    listener: TcpListener,
    payload: watch::Receiver<Arc<str>>,
    cfg: watch::Receiver<Config>,
    cmd: mpsc::UnboundedSender<Command>,
) {
    while let Ok((sock, _)) = listener.accept().await {
        let (payload, cfg, cmd) = (payload.clone(), cfg.clone(), cmd.clone());
        tokio::spawn(async move {
            // Peek, don't read: the WebSocket handshake reads the same bytes.
            let mut head = [0u8; 2048];
            let Ok(n) = sock.peek(&mut head).await else { return };
            let head = String::from_utf8_lossy(&head[..n]).into_owned();
            let password = cfg.borrow().api_password.clone();
            let authed = password.is_empty() || same(given(&head).as_deref(), &password);
            let path = head.split_whitespace().nth(1).unwrap_or("/");
            let path = path.split('?').next().unwrap_or("/").trim_end_matches('/').to_string();
            if !head.to_ascii_lowercase().contains("upgrade: websocket") {
                http(sock, n, authed, &payload).await;
            } else if path == "/ws/data" {
                if authed {
                    data(sock, payload).await;
                } else {
                    http(sock, n, false, &payload).await;
                }
            } else {
                commands(sock, authed, password, cmd).await;
            }
        });
    }
}

/// The password a request carries: `?password=`, `X-Songify-Password` or `Authorization: Bearer`.
fn given(head: &str) -> Option<String> {
    let target = head.split_whitespace().nth(1).unwrap_or("");
    let query = target.split_once('?').map_or("", |(_, q)| q);
    if let Some(p) = query.split('&').find_map(|kv| kv.strip_prefix("password=")) {
        return Some(decode(p));
    }
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "x-songify-password" => Some(v.to_string()),
            "authorization" => v.strip_prefix("Bearer ").map(|t| t.trim().to_string()),
            _ => None,
        }
    })
}

/// Compare without stopping at the first difference.
fn same(given: Option<&str>, password: &str) -> bool {
    let Some(g) = given else { return false };
    g.len() == password.len() && g.bytes().zip(password.bytes()).fold(0, |d, (a, b)| d | (a ^ b)) == 0
}

/// `%xx` and `+` in a query value.
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: Option<&u8>| (*c? as char).to_digit(16);
        match (b[i], hex(b.get(i + 1)), hex(b.get(i + 2))) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (b'+', ..) => {
                out.push(b' ');
                i += 1;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Any plain HTTP request: the JSON (Songify answers every path with it), 503 before the first song.
async fn http(mut sock: TcpStream, len: usize, authed: bool, payload: &watch::Receiver<Arc<str>>) {
    // The request was only peeked; read it so closing the socket doesn't reset the connection.
    let mut buf = vec![0u8; len];
    let _ = sock.read_exact(&mut buf).await;
    let body = payload.borrow().clone();
    let (status, body): (&str, &str) = if !authed {
        let e = r#"{"error":"Unauthorized. Pass ?password= on the URL or send header X-Songify-Password."}"#;
        ("401 Unauthorized", e)
    } else if body.is_empty() {
        ("503 Service Unavailable", r#"{"error":"Payload not available yet."}"#)
    } else {
        ("200 OK", &body)
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\n\
         Cache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = sock.write_all(&[head.as_bytes(), body.as_bytes()].concat()).await;
}

/// /ws/data: the JSON now (once there is one) and after every change, until the client goes.
async fn data(sock: TcpStream, mut payload: watch::Receiver<Arc<str>>) {
    let Ok(ws) = tokio_tungstenite::accept_async(sock).await else {
        return;
    };
    let (mut w, mut r) = ws.split();
    payload.mark_changed();
    loop {
        tokio::select! {
            c = payload.changed() => {
                if c.is_err() {
                    return;
                }
                let p = payload.borrow_and_update().clone();
                if !p.is_empty() && w.send(Message::text(p.to_string())).await.is_err() {
                    return;
                }
            }
            m = r.next() => match m {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            },
        }
    }
}

/// A command socket: each message is a command, its answer goes back.
async fn commands(sock: TcpStream, mut authed: bool, password: String, cmd: mpsc::UnboundedSender<Command>) {
    let Ok(ws) = tokio_tungstenite::accept_async(sock).await else {
        return;
    };
    let (mut w, mut r) = ws.split();
    while let Some(Ok(m)) = r.next().await {
        let Message::Text(t) = m else { continue };
        let answer = match command(&t, &mut authed, &password) {
            Ok((action, data)) => {
                let (tx, rx) = oneshot::channel();
                if cmd.send((action, data, tx)).is_err() {
                    return;
                }
                rx.await.unwrap_or_default()
            }
            Err(e) => e,
        };
        if !answer.is_empty()
            && w.send(Message::text(format!("Command executed: {answer}")))
                .await
                .is_err()
        {
            return;
        }
    }
}

/// Parse a command and check the password (Songify's rules); Err is the answer to send back.
fn command(text: &str, authed: &mut bool, password: &str) -> Result<(String, Value), String> {
    if text.trim().is_empty() {
        return Err(String::new());
    }
    let v: Value = serde_json::from_str(text).map_err(|_| "Invalid JSON.".to_string())?;
    let action = v["action"].as_str().filter(|a| !a.trim().is_empty());
    let action = action.ok_or("Invalid command format.")?.to_ascii_lowercase();
    let given = v["password"].as_str().or(v["data"]["password"].as_str());
    if action == "auth" {
        if password.is_empty() {
            return Err("Password protection is disabled; no auth needed.".into());
        }
        *authed = same(given, password);
        return Err(if *authed {
            "Authenticated."
        } else {
            "Authentication failed."
        }
        .into());
    }
    if !*authed {
        if !same(given, password) {
            return Err(
                "Unauthorized. Authenticate with {\"action\":\"auth\",\"data\":{\"password\":\"...\"}}, \
                include \"password\" on the command, or connect with ?password=..."
                    .into(),
            );
        }
        *authed = true;
    }
    Ok((action, v["data"].clone()))
}

/// Port 65530 (or the configured one), or why not.
pub async fn bind(port: u16) -> Result<TcpListener, String> {
    let l = TcpListener::bind(("127.0.0.1", port)).await;
    let l = l.map_err(|e| format!("can't open port {port} ({e}). Is Songify or another songrequestz running?"))?;
    say(format!(
        "songify api: http://127.0.0.1:{port}/ (WebSocket /ws/data for overlays)"
    ));
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords() {
        let head = "GET /ws/data?x=1&password=a%20b HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(given(head).as_deref(), Some("a b"));
        let head = "GET / HTTP/1.1\r\nx-songify-password: pw\r\n\r\n";
        assert_eq!(given(head).as_deref(), Some("pw"));
        let head = "GET / HTTP/1.1\r\nAuthorization: Bearer pw\r\n\r\n";
        assert_eq!(given(head).as_deref(), Some("pw"));
        assert_eq!(given("GET / HTTP/1.1\r\n\r\n"), None);
        assert!(same(Some("pw"), "pw") && !same(Some("pX"), "pw") && !same(None, "pw"));

        // No password set: everything goes.
        let mut authed = true;
        assert_eq!(command(r#"{"action":"Skip"}"#, &mut authed, "").unwrap().0, "skip");
        assert_eq!(command("nope", &mut authed, ""), Err("Invalid JSON.".into()));
        assert_eq!(command("{}", &mut authed, ""), Err("Invalid command format.".into()));
        // With one: the password on the command, or auth first.
        let mut authed = false;
        assert!(command(r#"{"action":"skip"}"#, &mut authed, "pw")
            .unwrap_err()
            .starts_with("Unauthorized"));
        assert!(command(r#"{"action":"skip","password":"pw"}"#, &mut authed, "pw").is_ok() && authed);
        let mut authed = false;
        let auth = command(r#"{"action":"auth","data":{"password":"pw"}}"#, &mut authed, "pw");
        assert_eq!((auth, authed), (Err("Authenticated.".into()), true));
        assert!(command(r#"{"action":"skip"}"#, &mut authed, "pw").is_ok());
    }
}
