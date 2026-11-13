//! Localhost web UI for VirtualKvm: a display layout editor.
//!
//! `lan-mouse web` connects to the running daemon through the regular frontend
//! IPC (like the CLI and the GTK frontend) and serves a single page on
//! `127.0.0.1`. Every API request needs the random token printed at startup and a
//! loopback `Host` header (DNS rebinding protection). See `docs/design/layout-gui.md`.

use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use futures::StreamExt;
use lan_mouse_ipc::{
    ClientConfig, ClientHandle, ClientState, ConnectionError, DisplayInfo, DisplayLink,
    FrontendEvent, FrontendRequest, IpcError, Position, connect_async,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{net::TcpListener, sync::mpsc};

const INDEX_HTML: &str = include_str!("../assets/index.html");
const TOKEN_HEADER: &str = "x-vkvm-token";

#[derive(clap::Args, Clone, Debug, PartialEq, Eq)]
pub struct WebArgs {
    /// port of the web UI (always bound to 127.0.0.1)
    #[arg(long, default_value_t = 4243)]
    port: u16,
    /// do not open the browser automatically
    #[arg(long)]
    no_open: bool,
}

#[derive(Debug, Error)]
pub enum WebError {
    #[error("could not connect: `{0}` - is the lan-mouse daemon running?")]
    ServiceNotRunning(#[from] ConnectionError),
    #[error("error communicating with the daemon: {0}")]
    Ipc(#[from] IpcError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug, Serialize)]
struct ClientView {
    handle: ClientHandle,
    config: ClientConfig,
    state: ClientState,
}

#[derive(Clone, Debug, Default, Serialize)]
struct Snapshot {
    /// displays of this machine
    local: Vec<DisplayInfo>,
    clients: Vec<ClientView>,
    fingerprint: Option<String>,
    connected: bool,
}

impl Snapshot {
    fn upsert(&mut self, handle: ClientHandle, config: ClientConfig, state: ClientState) {
        match self.clients.iter_mut().find(|c| c.handle == handle) {
            Some(c) => {
                c.config = config;
                c.state = state;
            }
            None => self.clients.push(ClientView {
                handle,
                config,
                state,
            }),
        }
        self.clients.sort_by_key(|c| c.handle);
    }

    fn apply(&mut self, event: FrontendEvent) {
        match event {
            FrontendEvent::Enumerate(clients) => {
                self.clients = clients
                    .into_iter()
                    .map(|(handle, config, state)| ClientView {
                        handle,
                        config,
                        state,
                    })
                    .collect();
                self.clients.sort_by_key(|c| c.handle);
            }
            FrontendEvent::Created(h, c, s) | FrontendEvent::State(h, c, s) => self.upsert(h, c, s),
            FrontendEvent::Deleted(h) => self.clients.retain(|c| c.handle != h),
            FrontendEvent::Displays(displays) => self.local = displays,
            FrontendEvent::PublicKeyFingerprint(fp) => self.fingerprint = Some(fp),
            _ => {}
        }
    }
}

#[derive(Clone)]
struct AppState {
    snapshot: Arc<Mutex<Snapshot>>,
    requests: mpsc::UnboundedSender<FrontendRequest>,
    token: Arc<String>,
    port: u16,
}

impl AppState {
    fn request(&self, request: FrontendRequest) {
        let _ = self.requests.send(request);
    }

    /// loopback `Host` + matching token
    fn check(&self, headers: &HeaderMap) -> Result<(), StatusCode> {
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let allowed = [
            format!("127.0.0.1:{}", self.port),
            format!("localhost:{}", self.port),
        ];
        if !allowed.iter().any(|a| a == host) {
            return Err(StatusCode::FORBIDDEN);
        }
        let token = headers
            .get(TOKEN_HEADER)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if token != self.token.as_str() {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(())
    }
}

pub async fn run(args: WebArgs) -> Result<(), WebError> {
    let (mut events, mut writer) = connect_async(Some(Duration::from_secs(5))).await?;

    let (requests, mut request_rx) = mpsc::unbounded_channel::<FrontendRequest>();
    let snapshot = Arc::new(Mutex::new(Snapshot {
        connected: true,
        ..Default::default()
    }));

    // forward requests to the daemon
    tokio::task::spawn_local(async move {
        while let Some(request) = request_rx.recv().await {
            if let Err(e) = writer.request(request).await {
                log::warn!("ipc write failed: {e}");
                break;
            }
        }
    });

    // keep the snapshot up to date
    {
        let snapshot = snapshot.clone();
        tokio::task::spawn_local(async move {
            while let Some(event) = events.next().await {
                match event {
                    Ok(event) => snapshot.lock().expect("lock").apply(event),
                    Err(e) => {
                        log::warn!("ipc read failed: {e}");
                        break;
                    }
                }
            }
            snapshot.lock().expect("lock").connected = false;
        });
    }

    for r in [
        FrontendRequest::Sync,
        FrontendRequest::Enumerate(),
        FrontendRequest::GetDisplays,
    ] {
        let _ = requests.send(r);
    }

    let token: String = {
        let mut rng = rand::thread_rng();
        (0..32)
            .map(|_| format!("{:02x}", rng.gen::<u8>()))
            .collect()
    };
    let state = AppState {
        snapshot,
        requests,
        token: Arc::new(token.clone()),
        port: args.port,
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/api/state", get(get_state))
        .route("/api/link", post(set_link))
        .with_state(state);

    let listener = TcpListener::bind(("127.0.0.1", args.port)).await?;
    let url = format!("http://127.0.0.1:{}/#token={token}", args.port);
    println!("VirtualKvm UI: {url}");
    if !args.no_open {
        open_browser(&url);
    }
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Html(INDEX_HTML),
    )
        .into_response()
}

async fn get_state(State(app): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(code) = app.check(&headers) {
        return code.into_response();
    }
    // ask for fresh data; the next poll will see it
    app.request(FrontendRequest::Enumerate());
    app.request(FrontendRequest::GetDisplays);
    let snapshot = app.snapshot.lock().expect("lock").clone();
    Json(snapshot).into_response()
}

#[derive(Debug, Deserialize)]
struct LinkRequest {
    handle: ClientHandle,
    position: Position,
    link: Option<DisplayLink>,
}

async fn set_link(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LinkRequest>,
) -> Response {
    if let Err(code) = app.check(&headers) {
        return code.into_response();
    }
    let current = app
        .snapshot
        .lock()
        .expect("lock")
        .clients
        .iter()
        .find(|c| c.handle == req.handle)
        .map(|c| c.config.pos);
    match current {
        None => return StatusCode::NOT_FOUND.into_response(),
        Some(pos) if pos != req.position => {
            app.request(FrontendRequest::UpdatePosition(req.handle, req.position))
        }
        Some(_) => {}
    }
    app.request(FrontendRequest::UpdateLink(req.handle, req.link));
    app.request(FrontendRequest::Enumerate());
    StatusCode::NO_CONTENT.into_response()
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(windows)]
    let r = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = r {
        log::warn!("could not open the browser: {e}");
    }
}
