//! A minimal in-process OpenOCD Tcl RPC server stand-in.
//!
//! Speaks the same byte-0x1a-terminated framing as real OpenOCD. Returns
//! pre-registered canned responses by command string. Useful for testing
//! the OpenOcdJtagTransport, DebugModule, and RiverCpuDriver flows without
//! requiring a real OpenOCD process.
//!
//! Usage:
//! ```ignore
//! let server = MockOpenOcdServer::new()
//!     .respond("halt", "")
//!     .respond("reg a0", "a0 (/64): 0x000000000000002a")
//!     .start()
//!     .await;
//! let endpoint = server.addr();
//! // ... drive a transport at `endpoint` ...
//! server.shutdown().await;
//! ```

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
            // Accept exactly one connection. If a test needs more, extend.
            if let Ok((mut sock, _)) = listener.accept().await {
                handle_connection(&mut sock, &responses, &received_for_task).await;
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
            // Drop the delimiter.
            pending.drain(..1);
            let cmd = String::from_utf8_lossy(&cmd_bytes).into_owned();
            {
                let mut log = received.lock().await;
                log.push(cmd.clone());
            }
            let resp = responses.get(&cmd).cloned().unwrap_or_default();
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
