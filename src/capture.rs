use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
    time::{Duration, Instant},
};

use futures::StreamExt;
use input_capture::{
    CaptureError, CaptureEvent, CaptureHandle, InputCapture, InputCaptureError, Position, Segment,
};
use input_event::{Event, KeyboardEvent, scancode};
use lan_mouse_proto::ProtoEvent;
use local_channel::mpsc::{Receiver, Sender, channel};
use tokio::task::{JoinHandle, spawn_local};
use tokio_util::sync::CancellationToken;

use crate::client::ClientManager;
use crate::connect::LanMouseConnection;
use crate::displays::Layout;

pub(crate) struct Capture {
    cancellation_token: CancellationToken,
    request_tx: Sender<CaptureRequest>,
    task: JoinHandle<()>,
    event_rx: Receiver<ICaptureEvent>,
}

pub(crate) enum ICaptureEvent {
    /// a client was entered
    CaptureBegin(CaptureHandle),
    /// capture disabled
    CaptureDisabled,
    /// capture disabled
    CaptureEnabled,
    /// A (new) client was entered.
    /// In contrast to [`ICaptureEvent::CaptureBegin`] this
    /// event is only triggered when the capture was
    /// explicitly released in the meantime by
    /// either the remote client leaving its device region,
    /// a new device entering the screen or the release bind.
    ClientEntered(u64),
    /// The previously active client was left, i.e. capture
    /// was released for the given handle. Mirrors
    /// [`ICaptureEvent::ClientEntered`] for the leave side
    /// and fires on every release path (release-bind chord,
    /// remote `Leave`, explicit `Release` request, send
    /// failure, or destroy of the active capture).
    ClientLeft(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureType {
    /// a normal input capture
    Default,
    /// A capture only interested in [`CaptureEvent::Begin`] events.
    /// The capture is released immediately, if there is no
    /// Default capture at the same position.
    EnterOnly,
}

#[derive(Clone, Debug)]
enum CaptureRequest {
    /// capture must release the mouse
    Release,
    /// add a capture client
    Create(CaptureHandle, Position, CaptureType),
    /// destory a capture client
    Destroy(CaptureHandle),
    /// reenable input capture
    Reenable,
    /// set release bind
    SetReleaseBind(Vec<scancode::Linux>),
    /// (VirtualKvm) the display layout of a client changed
    UpdateLayout(CaptureHandle),
    /// (VirtualKvm) send our layout for this client to the peer (mirrored there)
    SyncLayout(CaptureHandle),
    /// (VirtualKvm) an incoming (enter-only) capture takes its segments from this client's layout
    LayoutSource(CaptureHandle, lan_mouse_ipc::ClientHandle),
}

impl Capture {
    pub(crate) fn new(
        backend: Option<input_capture::Backend>,
        conn: LanMouseConnection,
        release_bind: Vec<scancode::Linux>,
        client_manager: ClientManager,
    ) -> Self {
        let (request_tx, request_rx) = channel();
        let (event_tx, event_rx) = channel();
        let cancellation_token = CancellationToken::new();
        let capture_task = CaptureTask {
            active_client: None,
            backend,
            cancellation_token: cancellation_token.clone(),
            captures: Default::default(),
            client_manager,
            pending_enter: None,
            crossing: None,
            pending_layout_sync: Default::default(),
            layout_source: Default::default(),
            conn,
            event_tx,
            request_rx,
            release_bind: Rc::new(RefCell::new(release_bind)),
            state: Default::default(),
        };
        let task = spawn_local(capture_task.run());
        Self {
            cancellation_token,
            request_tx,
            task,
            event_rx,
        }
    }

    pub(crate) fn reenable(&self) {
        self.request_tx
            .send(CaptureRequest::Reenable)
            .expect("channel closed");
    }

    pub(crate) async fn terminate(&mut self) {
        self.cancellation_token.cancel();
        log::debug!("terminating capture");
        if let Err(e) = (&mut self.task).await {
            log::warn!("{e}");
        }
    }

    pub(crate) fn create(
        &self,
        handle: CaptureHandle,
        pos: lan_mouse_ipc::Position,
        capture_type: CaptureType,
    ) {
        let pos = to_capture_pos(pos);
        self.request_tx
            .send(CaptureRequest::Create(handle, pos, capture_type))
            .expect("channel closed");
    }

    pub(crate) fn destroy(&self, handle: CaptureHandle) {
        self.request_tx
            .send(CaptureRequest::Destroy(handle))
            .expect("channel closed");
    }

    pub(crate) fn release(&self) {
        self.request_tx
            .send(CaptureRequest::Release)
            .expect("channel closed");
    }

    pub(crate) async fn event(&mut self) -> ICaptureEvent {
        self.event_rx.recv().await.expect("channel closed")
    }

    /// (VirtualKvm) re-apply the display layout of a client
    pub(crate) fn update_layout(&self, handle: CaptureHandle) {
        let _ = self.request_tx.send(CaptureRequest::UpdateLayout(handle));
    }

    /// (VirtualKvm) the incoming capture `handle` belongs to the peer `client`:
    /// its return barrier follows that client's layout
    pub(crate) fn set_layout_source(
        &self,
        handle: CaptureHandle,
        client: lan_mouse_ipc::ClientHandle,
    ) {
        let _ = self
            .request_tx
            .send(CaptureRequest::LayoutSource(handle, client));
    }

    /// (VirtualKvm) tell the peer about our layout so it stores the mirrored one
    pub(crate) fn sync_layout(&self, handle: CaptureHandle) {
        let _ = self.request_tx.send(CaptureRequest::SyncLayout(handle));
    }

    pub(crate) fn set_release_bind(&mut self, bind: Vec<scancode::Linux>) {
        let _ = self.request_tx.send(CaptureRequest::SetReleaseBind(bind));
    }
}

/// debounce a statement `$st`, i.e. the statement is executed only if the
/// time since the previous execution is at least `$dur`.
/// `$prev` is used to keep track of this timestamp
macro_rules! debounce {
    ($prev:ident, $dur:expr, $st:stmt) => {
        let exec = match $prev.get() {
            None => true,
            Some(instant) if instant.elapsed() > $dur => true,
            _ => false,
        };
        if exec {
            $prev.replace(Some(Instant::now()));
            $st
        }
    };
}

struct CaptureTask {
    active_client: Option<CaptureHandle>,
    backend: Option<input_capture::Backend>,
    cancellation_token: CancellationToken,
    captures: Vec<(CaptureHandle, Position, CaptureType)>,
    /// (VirtualKvm) access to display links and peer displays
    client_manager: ClientManager,
    /// (VirtualKvm) `Enter`/`EnterAt` to (re)send until the peer acknowledges
    pending_enter: Option<ProtoEvent>,
    /// (VirtualKvm) incoming capture -> client whose layout it follows
    layout_source: std::collections::HashMap<CaptureHandle, lan_mouse_ipc::ClientHandle>,
    /// (VirtualKvm) layouts not delivered to the peer yet (retried)
    pending_layout_sync: HashSet<CaptureHandle>,
    /// (VirtualKvm) local point where the barrier was crossed
    crossing: Option<(f64, f64, Position)>,
    conn: LanMouseConnection,
    event_tx: Sender<ICaptureEvent>,
    release_bind: Rc<RefCell<Vec<scancode::Linux>>>,
    request_rx: Receiver<CaptureRequest>,
    state: State,
}

impl CaptureTask {
    fn add_capture(&mut self, handle: CaptureHandle, pos: Position, capture_type: CaptureType) {
        self.captures.push((handle, pos, capture_type));
    }

    fn remove_capture(&mut self, handle: CaptureHandle) {
        self.captures.retain(|&(h, ..)| handle != h);
        self.layout_source.remove(&handle);
    }

    fn is_default_capture_at(&self, pos: Position) -> bool {
        self.captures
            .iter()
            .any(|&(_, p, t)| p == pos && t == CaptureType::Default)
    }

    fn get_pos(&self, handle: CaptureHandle) -> Position {
        self.captures
            .iter()
            .find(|(h, ..)| *h == handle)
            .expect("no such capture")
            .1
    }

    /// (VirtualKvm) resolved display layout of a client, if it has one
    fn layout(&self, handle: CaptureHandle) -> Option<Layout> {
        let layout = self.client_manager.get_layout(handle)?;
        Some(Layout::new(
            layout,
            &vkvm_displays::list(),
            &self.client_manager.remote_displays(handle),
        ))
    }

    /// (VirtualKvm) barrier segments for a client (None = whole desktop edge).
    /// Without the peer's displays we can't know where they touch ours yet.
    fn segments(&self, handle: CaptureHandle) -> Option<Vec<Segment>> {
        let capture_type = self
            .captures
            .iter()
            .find(|(h, ..)| *h == handle)
            .map(|c| c.2)?;
        let client = match capture_type {
            CaptureType::Default => handle,
            // the return barrier of an incoming connection
            CaptureType::EnterOnly => *self.layout_source.get(&handle)?,
        };
        let layout = self.layout(client)?;
        if layout.remote.is_empty() {
            return None;
        }
        Some(
            layout
                .contacts()
                .into_iter()
                .map(|c| Segment {
                    side: to_capture_pos(c.side),
                    x: c.display.x,
                    y: c.display.y,
                    width: c.display.w,
                    height: c.display.h,
                    from: c.from,
                    to: c.to,
                })
                .collect(),
        )
    }

    async fn apply_segments(
        &mut self,
        capture: &mut InputCapture,
        handle: CaptureHandle,
    ) -> Result<(), CaptureError> {
        let segments = self.segments(handle);
        log::info!("client {handle}: barrier segments {segments:?}");
        capture.set_segments(handle, segments).await?;
        // incoming captures that follow this client's layout
        let incoming: Vec<_> = self
            .layout_source
            .iter()
            .filter(|&(_, &c)| c == handle)
            .map(|(&h, _)| h)
            .collect();
        for h in incoming {
            let segments = self.segments(h);
            log::info!("incoming {h}: barrier segments {segments:?}");
            capture.set_segments(h, segments).await?;
        }
        Ok(())
    }

    /// (VirtualKvm) send our layout to the peer (it stores the mirrored one).
    /// If the connection is not up yet, it is retried later.
    async fn send_layout(&mut self, handle: CaptureHandle) {
        let event = match self.client_manager.get_layout(handle) {
            Some(l) => ProtoEvent::LayoutSync {
                present: true,
                dx: l.dx,
                dy: l.dy,
            },
            None => ProtoEvent::LayoutSync {
                present: false,
                dx: 0,
                dy: 0,
            },
        };
        log::info!("client {handle}: sending {event}");
        match self.conn.send(event, handle).await {
            Ok(()) => {
                self.pending_layout_sync.remove(&handle);
            }
            Err(e) => {
                log::info!("client {handle}: layout not sent yet ({e}), will retry");
                self.pending_layout_sync.insert(handle);
            }
        }
    }

    async fn retry_layout_sync(&mut self) {
        for handle in self.pending_layout_sync.clone() {
            self.send_layout(handle).await;
        }
    }

    /// (VirtualKvm) connect to active clients whose displays we don't know yet,
    /// so the layout works (and the UI shows them) before the first crossing
    async fn probe_peers(&mut self) {
        self.retry_layout_sync().await;
        for (handle, _, state) in self.client_manager.get_client_states() {
            if state.active && state.remote_displays.is_empty() {
                let _ = self.conn.send(ProtoEvent::Ping, handle).await;
            }
        }
    }

    fn get_type(&self, handle: CaptureHandle) -> CaptureType {
        self.captures
            .iter()
            .find(|(h, ..)| *h == handle)
            .expect("no such capture")
            .2
    }

    async fn run(mut self) {
        let mut probe = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            if let Err(e) = self.do_capture().await {
                log::warn!("input capture exited: {e}");
            }
            loop {
                tokio::select! {
                    r = self.request_rx.recv() => match r.expect("channel closed") {
                        CaptureRequest::Reenable => break,
                        CaptureRequest::Create(h, p, t) => self.add_capture(h, p, t),
                        CaptureRequest::Destroy(h) => self.remove_capture(h),
                        CaptureRequest::Release => { /* nothing to do */ }
                        CaptureRequest::SetReleaseBind(bind) => {
                            self.release_bind.borrow_mut().clone_from(&bind);
                        }
                        CaptureRequest::UpdateLayout(_) => { /* applied when capture starts */ }
                        CaptureRequest::SyncLayout(h) => self.send_layout(h).await,
                        CaptureRequest::LayoutSource(h, c) => {
                            self.layout_source.insert(h, c);
                        }
                    },
                    _ = probe.tick() => self.probe_peers().await,
                    _ = self.cancellation_token.cancelled() => return,
                }
            }
        }
    }

    async fn do_capture(&mut self) -> Result<(), InputCaptureError> {
        /* allow cancelling capture request */
        let mut capture = tokio::select! {
            r = InputCapture::new(self.backend) => r?,
            _ = self.cancellation_token.cancelled() => return Ok(()),
        };

        let _capture_guard = DropGuard::new(
            self.event_tx.clone(),
            ICaptureEvent::CaptureEnabled,
            ICaptureEvent::CaptureDisabled,
        );

        /* create barriers for active clients */
        let r = self.create_captures(&mut capture).await;
        if let Err(e) = r {
            capture.terminate().await?;
            return Err(e.into());
        }

        let r = self.do_capture_session(&mut capture).await;

        // FIXME replace with async drop when stabilized
        capture.terminate().await?;

        r
    }

    async fn create_captures(&mut self, capture: &mut InputCapture) -> Result<(), CaptureError> {
        let captures = self.captures.clone();
        for (handle, pos, _type) in captures {
            tokio::select! {
                r = capture.create(handle, pos) => r?,
                _ = self.cancellation_token.cancelled() => return Ok(()),
            }
            self.apply_segments(capture, handle).await?;
        }
        Ok(())
    }

    async fn do_capture_session(
        &mut self,
        capture: &mut InputCapture,
    ) -> Result<(), InputCaptureError> {
        let mut probe = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = probe.tick() => self.probe_peers().await,
                event = capture.next() => match event {
                    Some(event) => self.handle_capture_event(capture, event?).await?,
                    None => return Ok(()),
                },
                (handle, event) = self.conn.recv() => {
                    // (VirtualKvm) the peer finished announcing its displays:
                    // the barrier segments may change
                    if let ProtoEvent::DisplayInfo { index, count, .. } = event {
                        if index + 1 == count {
                            self.apply_segments(capture, handle).await?;
                            // the connection is up: deliver a pending layout
                            if self.pending_layout_sync.contains(&handle) {
                                self.send_layout(handle).await;
                            }
                        }
                        continue;
                    }
                    if let Some(active) = self.active_client {
                        if handle != active {
                            // we only care about events coming from the client we are currently connected to
                            // only `Ack` and `Leave` are relevant
                            continue
                        }
                    }

                    match event {
                        // connection acknowlegded => set state to Sending
                        ProtoEvent::Ack(_) => {
                            log::info!("client {handle} acknowledged the connection!");
                            self.state = State::Sending;
                        }
                        // client disconnected
                        ProtoEvent::Leave(_) => {
                            log::info!("releasing capture: left remote client device region");
                            self.release_capture(capture).await?;
                        },
                        // (VirtualKvm) place our cursor where it left the peer
                        ProtoEvent::LeaveAt { x, y } => {
                            let point = self.layout(handle).and_then(|g| g.to_local(x, y));
                            log::info!("releasing capture: left remote client device at ({x:.0}, {y:.0}) -> {point:?}");
                            self.release_capture_at(capture, point).await?;
                        },
                        _ => {}
                    }
                },
                e = self.request_rx.recv() => match e.expect("channel closed") {
                    CaptureRequest::Reenable => { /* already active */ },
                    CaptureRequest::Release => self.release_capture(capture).await?,
                    CaptureRequest::Create(h, p, t) => {
                        self.add_capture(h, p, t);
                        capture.create(h, p).await?;
                        self.apply_segments(capture, h).await?;
                    }
                    CaptureRequest::UpdateLayout(h) => self.apply_segments(capture, h).await?,
                    CaptureRequest::SyncLayout(h) => self.send_layout(h).await,
                    CaptureRequest::LayoutSource(h, c) => {
                        self.layout_source.insert(h, c);
                        let segments = self.segments(h);
                        log::info!("incoming {h}: barrier segments {segments:?}");
                        capture.set_segments(h, segments).await?;
                    }
                    CaptureRequest::Destroy(h) => {
                        // If the capture we're tearing down is the
                        // currently-active one, treat this as a
                        // release for hook purposes. The release_capture
                        // path also clears active_client and flushes
                        // pressed-key state to the peer; without this,
                        // `cli deactivate` (or a hostname change
                        // re-creating the client) would skip leave_hook.
                        if self.active_client == Some(h) {
                            self.release_capture(capture).await?;
                        }
                        self.remove_capture(h);
                        capture.destroy(h).await?;
                    }
                    CaptureRequest::SetReleaseBind(bind) => {
                        self.release_bind.borrow_mut().clone_from(&bind);
                    }
                },
                _ = self.cancellation_token.cancelled() => break,
            }
        }
        Ok(())
    }

    async fn handle_capture_event(
        &mut self,
        capture: &mut InputCapture,
        event: (CaptureHandle, CaptureEvent),
    ) -> Result<(), CaptureError> {
        let (handle, event) = event;
        log::trace!("({handle}): {event:?}");

        if capture.keys_pressed(&self.release_bind.borrow()) {
            log::info!("releasing capture: release-bind pressed");
            return self.release_capture(capture).await;
        }

        let begin = matches!(event, CaptureEvent::Begin | CaptureEvent::BeginAt { .. });
        if begin {
            self.event_tx
                .send(ICaptureEvent::CaptureBegin(handle))
                .expect("channel closed");
        }

        // enter only capture (for incoming connections)
        if self.get_type(handle) == CaptureType::EnterOnly {
            // if there is no active outgoing connection at the current capture,
            // we release the capture
            if !self.is_default_capture_at(self.get_pos(handle)) {
                log::info!("releasing capture: no active client at this position");
                capture.release().await?;
            }
            // we dont care about events from incoming handles except for releasing the capture
            return Ok(());
        }

        // activated a new client
        if begin && Some(handle) != self.active_client {
            self.state = State::WaitingForAck;
            self.active_client.replace(handle);
            self.event_tx
                .send(ICaptureEvent::ClientEntered(handle))
                .expect("channel closed");
        }

        let opposite_pos = to_proto_pos(self.get_pos(handle).opposite());

        // (VirtualKvm) with a display link, tell the peer exactly where to enter.
        // The peer's displays may only become known once the connection is up,
        // so keep retrying while the `Enter` is not acknowledged yet.
        if begin {
            self.crossing = match event {
                CaptureEvent::BeginAt { x, y, side } => Some((x, y, side)),
                _ => None,
            };
            self.pending_enter = None;
        }
        if self.state == State::WaitingForAck
            && !matches!(self.pending_enter, Some(ProtoEvent::EnterAt { .. }))
        {
            let entry = self
                .crossing
                .and_then(|(x, y, side)| self.layout(handle)?.to_remote(to_ipc_pos(side), x, y))
                .map(|(x, y)| ProtoEvent::EnterAt {
                    pos: opposite_pos,
                    x,
                    y,
                });
            if let Some(e) = &entry {
                log::info!("client {handle}: {e}");
            }
            self.pending_enter = Some(entry.unwrap_or(ProtoEvent::Enter(opposite_pos)));
        }
        let enter = self
            .pending_enter
            .unwrap_or(ProtoEvent::Enter(opposite_pos));

        let event = match event {
            CaptureEvent::Begin | CaptureEvent::BeginAt { .. } => enter,
            CaptureEvent::Input(e) => match self.state {
                // connection not acknowledged, repeat `Enter` event
                State::WaitingForAck => enter,
                State::Sending => ProtoEvent::Input(e),
            },
        };

        if let Err(e) = self.conn.send(event, handle).await {
            const DUR: Duration = Duration::from_millis(500);
            debounce!(PREV_LOG, DUR, log::warn!("releasing capture: {e}"));
            // Funnel through release_capture so the leave_hook
            // fires and active_client is cleared (without this the
            // active_client field would stay stale until the next
            // Begin from a different handle).
            self.release_capture(capture).await?;
        }
        Ok(())
    }

    async fn release_capture(&mut self, capture: &mut InputCapture) -> Result<(), CaptureError> {
        self.release_capture_at(capture, None).await
    }

    /// release the capture; with `point`, the local cursor reappears there (VirtualKvm)
    async fn release_capture_at(
        &mut self,
        capture: &mut InputCapture,
        point: Option<(f64, f64)>,
    ) -> Result<(), CaptureError> {
        self.pending_enter = None;
        self.crossing = None;
        // If we have an active client, notify them we're leaving
        if let Some(handle) = self.active_client.take() {
            // Surface the leave to the service layer so it can fire
            // the per-client leave_hook. Sent before the network
            // teardown below so we never race against the peer
            // disappearing.
            self.event_tx
                .send(ICaptureEvent::ClientLeft(handle))
                .expect("channel closed");
            // Synthesize key-up events for every key still held in the
            // capture's pressed_keys set BEFORE sending Leave. Without
            // this, pressing the release-bind chord (typically all four
            // modifiers) leaves the peer with phantom held modifiers:
            // the down events were forwarded while capture was active,
            // but the matching up events arrive after the local tap
            // flips to passthrough and never reach the peer. The peer
            // then runs every subsequent keystroke through those held
            // mods until its watchdog times out (1+ s) or our Leave
            // arrives — and Leave can be lost over UDP/DTLS.
            for key in capture.take_pressed_keys() {
                let key_up = ProtoEvent::Input(Event::Keyboard(KeyboardEvent::Key {
                    time: 0,
                    key: key as u32,
                    state: 0,
                }));
                if let Err(e) = self.conn.send(key_up, handle).await {
                    log::warn!("failed to send key-up to client {handle}: {e}");
                }
            }
            // Reset the modifier mask too. The peer's input-emulation
            // layer keeps a separate XKB-style modifier state that's
            // updated by KeyboardEvent::Modifiers, distinct from the
            // pressed_keys set drained above. Without this, an
            // already-locked CapsLock would survive the release.
            let mods_zero = ProtoEvent::Input(Event::Keyboard(KeyboardEvent::Modifiers {
                depressed: 0,
                latched: 0,
                locked: 0,
                group: 0,
            }));
            if let Err(e) = self.conn.send(mods_zero, handle).await {
                log::warn!("failed to reset modifiers on client {handle}: {e}");
            }

            log::info!("sending Leave event to client {handle}");
            if let Err(e) = self.conn.send(ProtoEvent::Leave(0), handle).await {
                log::warn!("failed to send Leave to client {handle}: {e}");
            }
        }
        match point {
            Some((x, y)) => capture.release_at(x, y).await,
            None => capture.release().await,
        }
    }
}

thread_local! {
    static PREV_LOG: Cell<Option<Instant>> = const { Cell::new(None) };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    WaitingForAck,
    Sending,
}

fn to_capture_pos(pos: lan_mouse_ipc::Position) -> input_capture::Position {
    match pos {
        lan_mouse_ipc::Position::Left => input_capture::Position::Left,
        lan_mouse_ipc::Position::Right => input_capture::Position::Right,
        lan_mouse_ipc::Position::Top => input_capture::Position::Top,
        lan_mouse_ipc::Position::Bottom => input_capture::Position::Bottom,
    }
}

fn to_ipc_pos(pos: input_capture::Position) -> lan_mouse_ipc::Position {
    match pos {
        input_capture::Position::Left => lan_mouse_ipc::Position::Left,
        input_capture::Position::Right => lan_mouse_ipc::Position::Right,
        input_capture::Position::Top => lan_mouse_ipc::Position::Top,
        input_capture::Position::Bottom => lan_mouse_ipc::Position::Bottom,
    }
}

fn to_proto_pos(pos: input_capture::Position) -> lan_mouse_proto::Position {
    match pos {
        input_capture::Position::Left => lan_mouse_proto::Position::Left,
        input_capture::Position::Right => lan_mouse_proto::Position::Right,
        input_capture::Position::Top => lan_mouse_proto::Position::Top,
        input_capture::Position::Bottom => lan_mouse_proto::Position::Bottom,
    }
}

struct DropGuard<T> {
    tx: Sender<T>,
    on_drop: Option<T>,
}

impl<T> DropGuard<T> {
    fn new(tx: Sender<T>, on_new: T, on_drop: T) -> Self {
        tx.send(on_new).expect("channel closed");
        let on_drop = Some(on_drop);
        Self { tx, on_drop }
    }
}

impl<T> Drop for DropGuard<T> {
    fn drop(&mut self) {
        self.tx
            .send(self.on_drop.take().expect("item"))
            .expect("channel closed");
    }
}
