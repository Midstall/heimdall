//! Minimal in-process OpenOCD Tcl RPC server stand-in.
//! Identical to the one in heimdall-driver/tests/common/. Duplicated here for
//! self-contained daemon tests.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const DELIM: u8 = 0x1a;

#[derive(Default)]
pub struct MockOpenOcdServer {
    responses: HashMap<String, String>,
}

pub struct RunningServer {
    addr: SocketAddr,
    handle: JoinHandle<()>,
    received: Arc<Mutex<Vec<String>>>,
}

impl MockOpenOcdServer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn respond(mut self, cmd: impl Into<String>, response: impl Into<String>) -> Self {
        self.responses.insert(cmd.into(), response.into());
        self
    }

    pub async fn start(self) -> RunningServer {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local_addr");
        let responses = Arc::new(self.responses);
        let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let received_for_task = received.clone();
        let handle = tokio::spawn(async move {
            // Accept multiple connections so the daemon can reconnect; loop
            // until the listener drops.
            while let Ok((mut sock, _)) = listener.accept().await {
                let resp = responses.clone();
                let recv = received_for_task.clone();
                tokio::spawn(async move {
                    handle_connection(&mut sock, &resp, &recv).await;
                });
            }
        });
        RunningServer {
            addr,
            handle,
            received,
        }
    }
}

impl RunningServer {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    pub async fn received(&self) -> Vec<String> {
        self.received.lock().await.clone()
    }
    pub async fn shutdown(self) {
        self.handle.abort();
        let _ = self.handle.await;
    }
}

async fn handle_connection(
    sock: &mut TcpStream,
    responses: &HashMap<String, String>,
    received: &Arc<Mutex<Vec<String>>>,
) {
    let mut buf = [0u8; 1024];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let n = match sock.read(&mut buf).await {
            Ok(0) => return,
            Ok(n) => n,
            Err(_) => return,
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(pos) = pending.iter().position(|&b| b == DELIM) {
            let cmd_bytes: Vec<u8> = pending.drain(..pos).collect();
            pending.drain(..1);
            let cmd = String::from_utf8_lossy(&cmd_bytes).into_owned();
            {
                let mut log = received.lock().await;
                log.push(cmd.clone());
            }
            // Exact match first; fall back to prefix-match for commands like
            // `load_image <dynamic_path> 0x... bin`.
            let resp = responses
                .get(&cmd)
                .cloned()
                .or_else(|| {
                    responses
                        .iter()
                        .find(|(k, _)| cmd.starts_with(k.as_str()))
                        .map(|(_, v)| v.clone())
                })
                .unwrap_or_default();
            if sock.write_all(resp.as_bytes()).await.is_err() {
                return;
            }
            if sock.write_all(&[DELIM]).await.is_err() {
                return;
            }
            if sock.flush().await.is_err() {
                return;
            }
        }
    }
}
