use crate::{
    capture::{Capture, CaptureType, ICaptureEvent},
    client::ClientManager,
    config::{Config, ConfigClient},
    connect::LanMouseConnection,
    crypto,
    dns::{DnsEvent, DnsResolver},
    emulation::{Emulation, EmulationEvent},
    listen::{LanMouseListener, ListenerCreationError},
};
use futures::StreamExt;
use lan_mouse_ipc::{
    AsyncFrontendListener, ClientHandle, FrontendEvent, FrontendRequest, IpcError,
    IpcListenerCreationError, Position, Status,
};
use log;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    net::{IpAddr, SocketAddr},
    sync::{Arc, RwLock},
};
use thiserror::Error;
use tokio::{process::Command, signal, sync::Notify};

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    IpcListen(#[from] IpcListenerCreationError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    ListenError(#[from] ListenerCreationError),
    #[error("failed to load certificate: `{0}`")]
    Certificate(#[from] crypto::Error),
}

pub struct Service {
    /// configuration
    config: Config,
    /// input capture
    capture: Capture,
    /// input emulation
    emulation: Emulation,
    /// dns resolver
    resolver: DnsResolver,
    /// frontend listener
    frontend_listener: AsyncFrontendListener,
    /// authorized public key sha256 fingerprints
    authorized_keys: Arc<RwLock<HashMap<String, String>>>,
    /// (outgoing) client information
    client_manager: ClientManager,
    /// current port
    port: u16,
    /// the public key fingerprint for (D)TLS
    public_key_fingerprint: String,
    /// notify for pending frontend events
    frontend_event_pending: Notify,
    /// frontend events queued for sending
    pending_frontend_events: VecDeque<FrontendEvent>,
    /// status of input capture (enabled / disabled)
    capture_status: Status,
    /// status of input emulation (enabled / disabled)
    emulation_status: Status,
    /// keep track of registered connections to avoid duplicate barriers
    incoming_conns: HashSet<SocketAddr>,
    /// map from capture handle to connection info
    incoming_conn_info: HashMap<ClientHandle, Incoming>,
    next_trigger_handle: u64,
    /// (VirtualKvm) reliable messages over the DTLS connections
    data: crate::data::DataHub,
    /// (VirtualKvm) data packets / acks from outgoing connections
    data_rx: local_channel::mpsc::Receiver<crate::data::DataIn>,
    /// (VirtualKvm) retransmission timer for unacknowledged messages
    data_tick: tokio::time::Interval,
    /// (VirtualKvm) hash of the clipboard text last synced in either direction
    clipboard_hash: Option<u64>,
    /// (VirtualKvm) messages for the local AI agent (see `vkvm mcp`)
    agent_inbox: VecDeque<lan_mouse_ipc::AgentMessage>,
    /// (VirtualKvm) (peer, mailbox) pairs that have written to us
    agent_mailboxes: Vec<(String, String)>,
    /// (VirtualKvm) local mailboxes: name -> (owner instance, last renewal)
    local_mailboxes: HashMap<String, (String, std::time::Instant)>,
}

#[derive(Debug)]
struct Incoming {
    fingerprint: String,
    addr: SocketAddr,
    pos: Position,
}

impl Service {
    pub async fn new(config: Config) -> Result<Self, ServiceError> {
        let client_manager = ClientManager::default();
        for client in config.clients() {
            client_manager.add_with_config(client);
        }

        // load certificate
        let cert = crypto::load_or_generate_key_and_cert(config.cert_path())?;
        let public_key_fingerprint = crypto::certificate_fingerprint(&cert);

        // create frontend communication adapter, exit if already running
        let frontend_listener = AsyncFrontendListener::new().await?;

        let authorized_keys = Arc::new(RwLock::new(config.authorized_fingerprints()));
        // listener + connection
        let listener =
            LanMouseListener::new(config.port(), cert.clone(), authorized_keys.clone()).await?;
        let (data_tx, data_rx) = local_channel::mpsc::channel();
        let conn = LanMouseConnection::new(cert.clone(), client_manager.clone(), data_tx);

        // input capture + emulation
        let capture_backend = config.capture_backend().map(|b| b.into());
        let capture = Capture::new(
            capture_backend,
            conn,
            config.release_bind(),
            client_manager.clone(),
        );
        let emulation_backend = config.emulation_backend().map(|b| b.into());
        let emulation = Emulation::new(emulation_backend, listener);
        emulation.set_invert_scroll(config.invert_scroll());

        // create dns resolver
        let resolver = DnsResolver::new()?;

        let port = config.port();
        let service = Self {
            config,
            capture,
            emulation,
            frontend_listener,
            resolver,
            authorized_keys,
            public_key_fingerprint,
            client_manager,
            frontend_event_pending: Default::default(),
            port,
            pending_frontend_events: Default::default(),
            capture_status: Default::default(),
            emulation_status: Default::default(),
            incoming_conn_info: Default::default(),
            incoming_conns: Default::default(),
            next_trigger_handle: 0,
            data: Default::default(),
            data_rx,
            data_tick: tokio::time::interval(std::time::Duration::from_millis(350)),
            clipboard_hash: None,
            agent_inbox: Default::default(),
            agent_mailboxes: Default::default(),
            local_mailboxes: Default::default(),
        };
        Ok(service)
    }

    pub async fn run(&mut self) -> Result<(), ServiceError> {
        let active = self.client_manager.active_clients();
        for handle in active.iter() {
            // small hack: `activate_client()` checks, if the client
            // is already active in client_manager and does not create a
            // capture barrier in that case so we have to deactivate it first
            self.client_manager.deactivate_client(*handle);
        }

        for handle in active {
            self.activate_client(handle);
        }

        loop {
            tokio::select! {
                request = self.frontend_listener.next() => self.handle_frontend_request(request),
                _ = self.frontend_event_pending.notified() => self.handle_frontend_pending().await,
                event = self.emulation.event() => self.handle_emulation_event(event),
                event = self.capture.event() => self.handle_capture_event(event),
                event = self.resolver.event() => self.handle_resolver_event(event),
                data = self.data_rx.recv() => self.handle_data_in(data.expect("channel closed")),
                _ = self.data_tick.tick() => self.retry_data(),
                _ = self.config.changed() => self.handle_config_change(),
                r = signal::ctrl_c() => break r.expect("failed to wait for CTRL+C"),
            }
        }

        log::info!("terminating service ...");
        log::debug!("terminating capture ...");
        self.capture.terminate().await;
        log::debug!("terminating emulation ...");
        self.emulation.terminate().await;
        log::debug!("terminating dns resolver ...");
        self.resolver.terminate().await;

        Ok(())
    }

    fn handle_frontend_request(&mut self, request: Option<Result<FrontendRequest, IpcError>>) {
        let request = match request.expect("frontend listener closed") {
            Ok(r) => r,
            Err(e) => return log::error!("error receiving request: {e}"),
        };
        match request {
            FrontendRequest::Activate(handle, active) => {
                self.set_client_active(handle, active);
                self.save_config();
            }
            FrontendRequest::AuthorizeKey(desc, fp) => {
                self.add_authorized_key(desc, fp);
                self.save_config();
            }
            FrontendRequest::ChangePort(port) => self.change_port(port),
            FrontendRequest::Create => {
                self.add_client();
                self.save_config();
            }
            FrontendRequest::Delete(handle) => {
                self.remove_client(handle);
                self.save_config();
            }
            FrontendRequest::EnableCapture => self.capture.reenable(),
            FrontendRequest::EnableEmulation => self.emulation.reenable(),
            FrontendRequest::Enumerate() => self.enumerate(),
            FrontendRequest::SendAgentMessage { peer, text } => {
                self.send_agent_message(peer, None, None, text)
            }
            FrontendRequest::SendAgentMessageTo {
                peer,
                from,
                to,
                text,
            } => self.send_agent_message(peer, from, to, text),
            FrontendRequest::TakeAgentMessages => {
                let messages = self.agent_inbox.drain(..).collect();
                self.notify_frontend(FrontendEvent::AgentMessages(messages));
            }
            FrontendRequest::RegisterMailbox { instance, wanted } => {
                let name = self.register_mailbox(&instance, &wanted);
                self.notify_frontend(FrontendEvent::MailboxRegistered { instance, name });
            }
            FrontendRequest::ReleaseMailbox { instance } => {
                self.local_mailboxes.retain(|name, (owner, _)| {
                    let keep = *owner != instance;
                    if !keep {
                        log::info!("agent mailbox `{name}` released");
                    }
                    keep
                });
            }
            FrontendRequest::TakeAgentMessagesFor {
                mailbox,
                include_unaddressed,
            } => {
                // addressed to this mailbox (and, optionally, to nobody in particular)
                let (mine, rest): (Vec<_>, Vec<_>) =
                    self.agent_inbox
                        .drain(..)
                        .partition(|m| match m.to.as_deref() {
                            Some(to) => to.eq_ignore_ascii_case(&mailbox),
                            None => include_unaddressed,
                        });
                self.agent_inbox = rest.into();
                self.notify_frontend(FrontendEvent::MailboxMessages {
                    mailbox,
                    messages: mine,
                });
            }
            FrontendRequest::GetSettings => self.notify_settings(),
            FrontendRequest::UpdateSettings(s) => {
                let language = if s.language == "en" { "en" } else { "es" };
                log::info!(
                    "settings: language={language} clipboard={} invert_scroll={}",
                    s.clipboard,
                    s.invert_scroll
                );
                let meta_tap_url = s.meta_tap_raycast.then(|| "raycast://".to_owned());
                self.config
                    .set_settings(language, s.clipboard, s.invert_scroll, meta_tap_url);
                self.emulation.set_invert_scroll(s.invert_scroll);
                self.save_config();
                self.notify_settings();
            }
            FrontendRequest::GetAgentPeers => {
                let peers = self
                    .client_manager
                    .get_client_states()
                    .into_iter()
                    .map(|(h, c, s)| {
                        let name = peer_name(h, &c);
                        let mailboxes = self
                            .agent_mailboxes
                            .iter()
                            .filter(|(p, _)| *p == name)
                            .map(|(_, m)| m.clone())
                            .collect();
                        lan_mouse_ipc::AgentPeer {
                            name,
                            connected: s.active_addr.is_some(),
                            displays: s.remote_displays.len(),
                            mailboxes,
                        }
                    })
                    .collect();
                self.notify_frontend(FrontendEvent::AgentPeers(peers));
            }
            FrontendRequest::GetDisplays => {
                self.notify_frontend(FrontendEvent::Displays(vkvm_displays::list()))
            }
            FrontendRequest::UpdateLayout(handle, layout) => {
                log::info!("client ({handle}) layout: {layout:?}");
                if let Some(l) = layout {
                    let remote = self.client_manager.remote_displays(handle);
                    let g = crate::displays::Layout::new(l, &vkvm_displays::list(), &remote);
                    if g.overlaps() {
                        self.notify_frontend(FrontendEvent::Error(
                            "los monitores se superponen".into(),
                        ));
                        return;
                    }
                }
                self.apply_layout(handle, layout);
                // the peer stores the mirrored layout
                self.capture.sync_layout(handle);
                self.save_config();
            }
            FrontendRequest::UpdateFixIps(handle, fix_ips) => {
                self.update_fix_ips(handle, fix_ips);
                self.save_config();
            }
            FrontendRequest::UpdateHostname(handle, host) => {
                self.update_hostname(handle, host);
                self.save_config();
            }
            FrontendRequest::UpdatePort(handle, port) => {
                self.update_port(handle, port);
                self.save_config();
            }
            FrontendRequest::UpdatePosition(handle, pos) => {
                self.update_pos(handle, pos);
                self.save_config();
            }
            FrontendRequest::ResolveDns(handle) => self.resolve(handle),
            FrontendRequest::Sync => self.sync_frontend(),
            FrontendRequest::RemoveAuthorizedKey(key) => {
                self.remove_authorized_key(key);
                self.save_config();
            }
            FrontendRequest::UpdateEnterHook(handle, enter_hook) => {
                self.update_enter_hook(handle, enter_hook)
            }
            FrontendRequest::UpdateLeaveHook(handle, leave_hook) => {
                self.update_leave_hook(handle, leave_hook)
            }
            FrontendRequest::SaveConfiguration => self.save_config(),
        }
    }

    fn save_config(&mut self) {
        let clients = self.client_manager.clients();
        let clients = clients
            .into_iter()
            .map(|(c, s)| ConfigClient {
                ips: HashSet::from_iter(c.fix_ips),
                hostname: c.hostname,
                port: c.port,
                pos: c.pos,
                active: s.active,
                enter_hook: c.cmd,
                leave_hook: c.leave_cmd,
                layout: c.layout,
            })
            .collect();
        self.config.set_clients(clients);
        let authorized_keys = self.authorized_keys.read().expect("lock").clone();
        self.config.set_authorized_keys(authorized_keys);
        if let Err(e) = self.config.write_back() {
            log::warn!("failed to write config: {e}");
        }
    }

    fn handle_config_change(&mut self) {
        for h in self.client_manager.registered_clients() {
            self.remove_client(h);
        }
        for c in self.config.clients() {
            let handle = self.client_manager.add_with_config(c);
            log::info!("added client {handle}");
            let (c, s) = self.client_manager.get_state(handle).unwrap();
            if s.active {
                self.client_manager.deactivate_client(handle);
                self.activate_client(handle);
            }
            self.notify_frontend(FrontendEvent::Created(handle, c, s));
        }
        let release_bind = self.config.release_bind();
        self.capture.set_release_bind(release_bind);
        let authorized_keys = self.config.authorized_fingerprints();
        self.authorized_keys
            .write()
            .unwrap()
            .clone_from(&authorized_keys);
        self.sync_frontend();
    }

    async fn handle_frontend_pending(&mut self) {
        while let Some(event) = self.pending_frontend_events.pop_front() {
            self.frontend_listener.broadcast(event).await;
        }
    }

    fn handle_emulation_event(&mut self, event: EmulationEvent) {
        match event {
            EmulationEvent::ConnectionAttempt { fingerprint } => {
                self.notify_frontend(FrontendEvent::ConnectionAttempt { fingerprint });
            }
            EmulationEvent::Entered {
                addr,
                pos,
                fingerprint,
            } => {
                // check if already registered
                if !self.incoming_conns.contains(&addr) {
                    self.add_incoming(addr, pos, fingerprint.clone());
                    self.notify_frontend(FrontendEvent::DeviceEntered {
                        fingerprint,
                        addr,
                        pos,
                    });
                } else {
                    self.update_incoming(addr, pos, fingerprint);
                }
            }
            EmulationEvent::Disconnected { addr } => {
                if let Some(addr) = self.remove_incoming(addr) {
                    self.notify_frontend(FrontendEvent::IncomingDisconnected(addr));
                }
            }
            EmulationEvent::PortChanged(port) => match port {
                Ok(port) => {
                    self.port = port;
                    self.notify_frontend(FrontendEvent::PortChanged(port, None));
                }
                Err(e) => self
                    .notify_frontend(FrontendEvent::PortChanged(self.port, Some(format!("{e}")))),
            },
            EmulationEvent::EmulationDisabled => {
                self.emulation_status = Status::Disabled;
                self.notify_frontend(FrontendEvent::EmulationStatus(self.emulation_status));
            }
            EmulationEvent::EmulationEnabled => {
                self.emulation_status = Status::Enabled;
                self.notify_frontend(FrontendEvent::EmulationStatus(self.emulation_status));
            }
            EmulationEvent::ReleaseNotify => self.capture.release(),
            EmulationEvent::Connected { addr, fingerprint } => {
                self.notify_frontend(FrontendEvent::DeviceConnected { addr, fingerprint });
            }
            EmulationEvent::PeerDisplay {
                addr,
                index,
                count,
                display,
            } => {
                log::info!("{addr} display {index}/{count}: {display:?}");
                // also for inactive clients: the layout UI needs the peer's displays
                if let Some(handle) = self.peer_client(addr) {
                    self.client_manager
                        .set_remote_display(handle, index, count, display);
                    if index + 1 == count {
                        // the primary side may change now that the displays are known
                        let layout = self.client_manager.get_layout(handle);
                        self.apply_layout(handle, layout);
                    } else {
                        self.broadcast_client(handle);
                    }
                }
            }
            EmulationEvent::Data { addr, packet } => {
                let from = crate::data::Peer::Addr(addr);
                self.handle_data_in(crate::data::DataIn::Packet { from, packet })
            }
            EmulationEvent::DataAck { id } => self.data.acked(id),
            EmulationEvent::MetaTap => {
                if let Some(url) = self.config.meta_tap_url() {
                    log::info!("Meta tapped alone: opening {url}");
                    open_url(&url);
                }
            }
            EmulationEvent::PeerLayout { addr, layout } => {
                let Some(handle) = self.peer_client(addr) else {
                    log::warn!("{addr} sent a layout, but it is not a configured client");
                    return;
                };
                let layout = layout.map(|l| l.mirrored());
                log::info!("client ({handle}) layout from peer: {layout:?}");
                if self.client_manager.get_layout(handle) != layout {
                    self.apply_layout(handle, layout);
                    self.save_config();
                }
            }
            EmulationEvent::PeerHello { addr, commit } => {
                // Map the peer's source addr back to its client handle
                // and stamp the commit. Skip if we don't have an
                // outgoing client configured for this peer (incoming-
                // only setup) — there's nowhere to display the version
                // in that case anyway.
                if let Some(handle) = self.client_manager.get_client(addr) {
                    self.client_manager.set_peer_commit(handle, Some(commit));
                    self.broadcast_client(handle);
                }
            }
        }
    }

    fn handle_capture_event(&mut self, event: ICaptureEvent) {
        match event {
            ICaptureEvent::CaptureBegin(handle) => {
                // we entered the capture zone for an incoming connection
                // => notify it that its capture should be released
                if let Some(addr) = self.incoming_conn_info.get(&handle).map(|i| i.addr) {
                    // (VirtualKvm) our clipboard goes along with the cursor
                    self.share_clipboard(crate::data::Peer::Addr(addr));
                    self.emulation.send_leave_event(addr);
                }
            }
            ICaptureEvent::CaptureDisabled => {
                self.capture_status = Status::Disabled;
                self.notify_frontend(FrontendEvent::CaptureStatus(self.capture_status));
            }
            ICaptureEvent::CaptureEnabled => {
                self.capture_status = Status::Enabled;
                self.notify_frontend(FrontendEvent::CaptureStatus(self.capture_status));
            }
            ICaptureEvent::ClientEntered(handle) => {
                log::info!("entering client {handle} ...");
                // (VirtualKvm) our clipboard goes along with the cursor
                self.share_clipboard(crate::data::Peer::Client(handle));
                self.spawn_hook_command(handle, HookKind::Enter);
            }
            ICaptureEvent::ClientLeft(handle) => {
                log::info!("leaving client {handle} ...");
                self.spawn_hook_command(handle, HookKind::Leave);
            }
        }
    }

    fn handle_resolver_event(&mut self, event: DnsEvent) {
        let handle = match event {
            DnsEvent::Resolving(handle) => {
                self.client_manager.set_resolving(handle, true);
                handle
            }
            DnsEvent::Resolved(handle, hostname, ips) => {
                self.client_manager.set_resolving(handle, false);
                if let Err(e) = &ips {
                    log::warn!("could not resolve {hostname}: {e}");
                }
                let ips = ips.unwrap_or_default();
                self.client_manager.set_dns_ips(handle, ips);
                handle
            }
        };
        self.broadcast_client(handle);
    }

    fn resolve(&self, handle: ClientHandle) {
        if let Some(hostname) = self.client_manager.get_hostname(handle) {
            self.resolver.resolve(handle, hostname);
        }
    }

    fn sync_frontend(&mut self) {
        self.enumerate();
        self.notify_frontend(FrontendEvent::EmulationStatus(self.emulation_status));
        self.notify_frontend(FrontendEvent::CaptureStatus(self.capture_status));
        self.notify_frontend(FrontendEvent::PortChanged(self.port, None));
        self.notify_frontend(FrontendEvent::PublicKeyFingerprint(
            self.public_key_fingerprint.clone(),
        ));
        let keys = self.authorized_keys.read().expect("lock").clone();
        self.notify_frontend(FrontendEvent::AuthorizedUpdated(keys));
    }

    const ENTER_HANDLE_BEGIN: u64 = u64::MAX / 2 + 1;

    fn add_incoming(&mut self, addr: SocketAddr, pos: Position, fingerprint: String) {
        let handle = Self::ENTER_HANDLE_BEGIN + self.next_trigger_handle;
        self.next_trigger_handle += 1;
        self.capture.create(handle, pos, CaptureType::EnterOnly);
        // (VirtualKvm) the return barrier follows our layout for this peer
        if let Some(client) = self.peer_client(addr) {
            self.capture.set_layout_source(handle, client);
        }
        self.incoming_conns.insert(addr);
        self.incoming_conn_info.insert(
            handle,
            Incoming {
                fingerprint,
                addr,
                pos,
            },
        );
    }

    fn update_incoming(&mut self, addr: SocketAddr, pos: Position, fingerprint: String) {
        let incoming = self
            .incoming_conn_info
            .iter_mut()
            .find(|(_, i)| i.addr == addr)
            .map(|(_, i)| i)
            .expect("no such client");
        let mut changed = false;
        if incoming.fingerprint != fingerprint {
            incoming.fingerprint = fingerprint.clone();
            changed = true;
        }
        if incoming.pos != pos {
            incoming.pos = pos;
            changed = true;
        }
        if changed {
            self.remove_incoming(addr);
            self.add_incoming(addr, pos, fingerprint.clone());
            self.notify_frontend(FrontendEvent::IncomingDisconnected(addr));
            self.notify_frontend(FrontendEvent::DeviceEntered {
                fingerprint,
                addr,
                pos,
            });
        }
    }

    fn remove_incoming(&mut self, addr: SocketAddr) -> Option<SocketAddr> {
        let handle = self
            .incoming_conn_info
            .iter()
            .find(|(_, incoming)| incoming.addr == addr)
            .map(|(k, _)| *k)?;
        self.capture.destroy(handle);
        self.incoming_conns.remove(&addr);
        self.incoming_conn_info
            .remove(&handle)
            .map(|incoming| incoming.addr)
    }

    fn notify_frontend(&mut self, event: FrontendEvent) {
        self.pending_frontend_events.push_back(event);
        self.frontend_event_pending.notify_one();
    }

    fn add_authorized_key(&mut self, desc: String, fp: String) {
        self.authorized_keys.write().expect("lock").insert(fp, desc);
        let keys = self.authorized_keys.read().expect("lock").clone();
        self.notify_frontend(FrontendEvent::AuthorizedUpdated(keys));
    }

    fn remove_authorized_key(&mut self, fp: String) {
        self.authorized_keys.write().expect("lock").remove(&fp);
        let keys = self.authorized_keys.read().expect("lock").clone();
        self.notify_frontend(FrontendEvent::AuthorizedUpdated(keys));
    }

    fn enumerate(&mut self) {
        let clients = self.client_manager.get_client_states();
        self.notify_frontend(FrontendEvent::Enumerate(clients));
    }

    fn add_client(&mut self) {
        let handle = self.client_manager.add_client();
        log::info!("added client {handle}");
        let (c, s) = self.client_manager.get_state(handle).unwrap();
        self.notify_frontend(FrontendEvent::Created(handle, c, s));
    }

    fn set_client_active(&mut self, handle: ClientHandle, active: bool) {
        if active {
            self.activate_client(handle);
        } else {
            self.deactivate_client(handle);
        }
    }

    /// (VirtualKvm) send a message to a peer (retried until acknowledged)
    fn send_message(&mut self, to: crate::data::Peer, kind: u8, data: &[u8]) {
        if let Some(packets) = self.data.send(to, kind, data) {
            self.send_packets(to, packets);
        }
    }

    fn send_packets(&mut self, to: crate::data::Peer, packets: Vec<lan_mouse_proto::DataPacket>) {
        let datagrams = packets.iter().map(|p| p.encode()).collect();
        match to {
            crate::data::Peer::Client(h) => self.capture.send_raw(h, datagrams),
            crate::data::Peer::Addr(a) => self.emulation.send_raw(a, datagrams),
        }
    }

    fn retry_data(&mut self) {
        for (to, packets) in self.data.retries(std::time::Instant::now()) {
            log::debug!("retrying message to {to:?}");
            self.send_packets(to, packets);
        }
    }

    fn handle_data_in(&mut self, data: crate::data::DataIn) {
        use crate::data::{DataIn, Peer};
        match data {
            DataIn::Ack { id } => self.data.acked(id),
            DataIn::Packet { from, packet } => {
                let received = self.data.received(from, packet);
                if let Some((to, id)) = received.ack {
                    let ack = lan_mouse_proto::ProtoEvent::DataAck { id };
                    match to {
                        Peer::Client(h) => {
                            let (buf, len): ([u8; lan_mouse_proto::MAX_EVENT_SIZE], usize) =
                                ack.into();
                            self.capture.send_raw(h, vec![buf[..len].to_vec()]);
                        }
                        Peer::Addr(a) => self.emulation.send_event(a, ack),
                    }
                }
                if let Some((from, kind, data)) = received.message {
                    self.handle_message(from, kind, data);
                }
            }
        }
    }

    fn handle_message(&mut self, from: crate::data::Peer, kind: u8, data: Vec<u8>) {
        match kind {
            crate::data::KIND_CLIPBOARD => {
                if !self.config.clipboard_sharing() {
                    return;
                }
                let Ok(text) = String::from_utf8(data) else {
                    log::warn!("clipboard from {from:?} is not UTF-8");
                    return;
                };
                self.clipboard_hash = Some(text_hash(&text));
                if vkvm_clipboard::set_text(&text) {
                    log::info!("clipboard received from {from:?} ({} bytes)", text.len());
                }
            }
            crate::data::KIND_AGENT | crate::data::KIND_AGENT_ADDRESSED => {
                let Ok(payload) = String::from_utf8(data) else {
                    log::warn!("agent message from {from:?} is not UTF-8");
                    return;
                };
                let (from_mailbox, to, text) = if kind == crate::data::KIND_AGENT_ADDRESSED {
                    #[derive(serde::Deserialize)]
                    struct Addressed {
                        from: Option<String>,
                        to: Option<String>,
                        text: String,
                    }
                    match serde_json::from_str::<Addressed>(&payload) {
                        Ok(a) => (a.from, a.to, a.text),
                        Err(e) => {
                            log::warn!("bad addressed agent message from {from:?}: {e}");
                            return;
                        }
                    }
                } else {
                    (None, None, payload)
                };
                let from = match from {
                    crate::data::Peer::Client(h) => self
                        .client_manager
                        .get_state(h)
                        .map(|(c, _)| peer_name(h, &c)),
                    crate::data::Peer::Addr(a) => self.peer_client(a).and_then(|h| {
                        self.client_manager
                            .get_state(h)
                            .map(|(c, _)| peer_name(h, &c))
                    }),
                }
                .unwrap_or_else(|| "?".into());
                log::info!(
                    "agent message from {from}/{} to {} ({} bytes)",
                    from_mailbox.as_deref().unwrap_or("?"),
                    to.as_deref().unwrap_or("*"),
                    text.len()
                );
                if let Some(m) = &from_mailbox {
                    let key = (from.clone(), m.clone());
                    if !self.agent_mailboxes.contains(&key) {
                        self.agent_mailboxes.push(key);
                    }
                }
                const INBOX_MAX: usize = 100;
                if self.agent_inbox.len() == INBOX_MAX {
                    self.agent_inbox.pop_front();
                }
                let received_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or_default();
                self.agent_inbox.push_back(lan_mouse_ipc::AgentMessage {
                    from,
                    text,
                    received_at,
                    from_mailbox,
                    to,
                });
            }
            kind => log::debug!("ignoring message of kind {kind} from {from:?}"),
        }
    }

    fn notify_settings(&mut self) {
        let settings = lan_mouse_ipc::Settings {
            language: self.config.language(),
            clipboard: self.config.clipboard_sharing(),
            invert_scroll: self.config.invert_scroll(),
            meta_tap_raycast: self.config.meta_tap_url().is_some(),
        };
        self.notify_frontend(FrontendEvent::Settings(settings));
    }

    /// (VirtualKvm) give an agent session a mailbox no other live session holds.
    /// Leases last 90 s; `vkvm mcp` renews every 30 s.
    fn register_mailbox(&mut self, instance: &str, wanted: &str) -> String {
        const LEASE: std::time::Duration = std::time::Duration::from_secs(90);
        let now = std::time::Instant::now();
        self.local_mailboxes
            .retain(|_, (_, seen)| now.duration_since(*seen) < LEASE);
        let free = |name: &str, boxes: &HashMap<String, (String, std::time::Instant)>| {
            boxes.get(name).is_none_or(|(owner, _)| owner == instance)
        };
        let mut name = wanted.to_owned();
        let mut n = 2;
        while !free(&name, &self.local_mailboxes) {
            name = format!("{wanted}-{n}");
            n += 1;
        }
        // one mailbox per instance: renaming releases the previous one
        self.local_mailboxes
            .retain(|other, (owner, _)| owner != instance || *other == name);
        if !self.local_mailboxes.contains_key(&name) {
            log::info!("agent mailbox `{name}` registered");
        }
        self.local_mailboxes
            .insert(name.clone(), (instance.to_owned(), now));
        name
    }

    /// (VirtualKvm) relay a text message to the AI agent on a peer
    fn send_agent_message(
        &mut self,
        peer: Option<String>,
        from: Option<String>,
        to: Option<String>,
        text: String,
    ) {
        let clients = self.client_manager.get_client_states();
        let target = match &peer {
            Some(p) => clients.iter().find(|(h, c, _)| {
                peer_name(*h, c).eq_ignore_ascii_case(p)
                    || c.fix_ips.iter().any(|ip| ip.to_string() == *p)
            }),
            None => clients.first(),
        };
        let Some((handle, config, _)) = target else {
            let error = Some(match peer {
                Some(p) => format!("no peer named `{p}`"),
                None => "no peers configured".into(),
            });
            self.notify_frontend(FrontendEvent::AgentMessageSent {
                to: None,
                error,
                from: from.clone(),
            });
            return;
        };
        let (handle, name) = (*handle, peer_name(*handle, config));
        log::info!("agent message to {name} ({} bytes)", text.len());
        let payload = serde_json::json!({ "from": from, "to": to, "text": text }).to_string();
        self.send_message(
            crate::data::Peer::Client(handle),
            crate::data::KIND_AGENT_ADDRESSED,
            payload.as_bytes(),
        );
        self.notify_frontend(FrontendEvent::AgentMessageSent {
            to: Some(name),
            error: None,
            from,
        });
    }

    /// (VirtualKvm) send our clipboard text to the peer the cursor moves to,
    /// unless it is what we already synced
    fn share_clipboard(&mut self, to: crate::data::Peer) {
        if !self.config.clipboard_sharing() {
            return;
        }
        let Some(text) = vkvm_clipboard::get_text() else {
            return;
        };
        let hash = text_hash(&text);
        if self.clipboard_hash == Some(hash) {
            return;
        }
        self.clipboard_hash = Some(hash);
        log::info!("sharing clipboard with {to:?} ({} bytes)", text.len());
        self.send_message(to, crate::data::KIND_CLIPBOARD, text.as_bytes());
    }

    /// (VirtualKvm) client of an authorized incoming peer. Peers are matched by
    /// ip; if the ip is unknown (DHCP gave the peer a new one) and there is only
    /// one client, that is the peer: its new ip is added to the client's config.
    fn peer_client(&mut self, addr: std::net::SocketAddr) -> Option<ClientHandle> {
        if let Some(handle) = self.client_manager.get_client_by_ip(addr.ip()) {
            return Some(handle);
        }
        let clients = self.client_manager.get_client_states();
        let [(handle, config, _)] = clients.as_slice() else {
            log::warn!("{addr}: unknown peer ip and several clients configured");
            return None;
        };
        log::info!(
            "client ({handle}): peer seen at new ip {}, adding it",
            addr.ip()
        );
        let mut ips = config.fix_ips.clone();
        ips.push(addr.ip());
        self.update_fix_ips(*handle, ips);
        self.save_config();
        Some(*handle)
    }

    /// (VirtualKvm) store a display layout and re-apply it. The client position
    /// follows the longest touching edge; an inactive client stays inactive.
    fn apply_layout(&mut self, handle: ClientHandle, layout: Option<lan_mouse_ipc::DisplayLayout>) {
        self.client_manager.set_layout(handle, layout);
        if let Some(l) = layout {
            let remote = self.client_manager.remote_displays(handle);
            let side =
                crate::displays::Layout::new(l, &vkvm_displays::list(), &remote).primary_side();
            if let Some(side) = side {
                let active = self.client_manager.is_active(handle);
                if self.client_manager.get_pos(handle) != Some(side) {
                    if active {
                        self.deactivate_client(handle);
                    }
                    self.client_manager.set_pos(handle, side);
                    if active {
                        self.activate_client(handle);
                    }
                }
            }
        }
        self.capture.update_layout(handle);
        self.broadcast_client(handle);
    }

    fn deactivate_client(&mut self, handle: ClientHandle) {
        log::debug!("deactivating client {handle}");
        if self.client_manager.deactivate_client(handle) {
            self.capture.destroy(handle);
            self.broadcast_client(handle);
            log::info!("deactivated client {handle}");
        }
    }

    fn activate_client(&mut self, handle: ClientHandle) {
        log::debug!("activating client {handle}");

        /* resolve dns on activate */
        self.resolve(handle);

        /* deactivate potential other client at this position */
        let Some(pos) = self.client_manager.get_pos(handle) else {
            return;
        };

        if let Some(other) = self.client_manager.client_at(pos) {
            if other != handle {
                self.deactivate_client(other);
            }
        }

        /* activate the client */
        if self.client_manager.activate_client(handle) {
            /* notify capture and frontends */
            self.capture.create(handle, pos, CaptureType::Default);
            self.broadcast_client(handle);
            log::info!("activated client {handle} ({pos})");
        }
    }

    fn change_port(&mut self, port: u16) {
        if self.port != port {
            self.emulation.request_port_change(port);
        } else {
            self.notify_frontend(FrontendEvent::PortChanged(self.port, None));
        }
    }

    fn remove_client(&mut self, handle: ClientHandle) {
        if self
            .client_manager
            .remove_client(handle)
            .map(|(_, s)| s.active)
            .unwrap_or(false)
        {
            self.capture.destroy(handle);
        }
        self.notify_frontend(FrontendEvent::Deleted(handle));
    }

    fn update_fix_ips(&mut self, handle: ClientHandle, fix_ips: Vec<IpAddr>) {
        self.client_manager.set_fix_ips(handle, fix_ips);
        self.broadcast_client(handle);
    }

    fn update_hostname(&mut self, handle: ClientHandle, hostname: Option<String>) {
        log::info!("hostname changed: {hostname:?}");
        if self.client_manager.set_hostname(handle, hostname.clone()) {
            self.resolve(handle);
        }
        self.broadcast_client(handle);
    }

    fn update_port(&mut self, handle: ClientHandle, port: u16) {
        self.client_manager.set_port(handle, port);
        self.broadcast_client(handle);
    }

    fn update_pos(&mut self, handle: ClientHandle, pos: Position) {
        // update state in event input emulator & input capture
        if self.client_manager.set_pos(handle, pos) {
            self.deactivate_client(handle);
            self.activate_client(handle);
        }
        self.broadcast_client(handle);
    }

    fn update_enter_hook(&mut self, handle: ClientHandle, enter_hook: Option<String>) {
        self.client_manager.set_enter_hook(handle, enter_hook);
        self.broadcast_client(handle);
    }

    fn update_leave_hook(&mut self, handle: ClientHandle, leave_hook: Option<String>) {
        self.client_manager.set_leave_hook(handle, leave_hook);
        self.broadcast_client(handle);
    }

    fn broadcast_client(&mut self, handle: ClientHandle) {
        let event = self
            .client_manager
            .get_state(handle)
            .map(|(c, s)| FrontendEvent::State(handle, c, s))
            .unwrap_or(FrontendEvent::NoSuchClient(handle));
        self.notify_frontend(event);
    }

    fn spawn_hook_command(&self, handle: ClientHandle, kind: HookKind) {
        let cmd = match kind {
            HookKind::Enter => self.client_manager.get_enter_cmd(handle),
            HookKind::Leave => self.client_manager.get_leave_cmd(handle),
        };
        let Some(cmd) = cmd else { return };
        tokio::task::spawn_local(async move {
            log::info!("spawning {kind} hook for client {handle}");
            let mut child = match Command::new("sh").arg("-c").arg(cmd.as_str()).spawn() {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("could not execute {kind} hook for client {handle}: {e}");
                    return;
                }
            };
            match child.wait().await {
                Ok(s) => {
                    if s.success() {
                        log::info!("{kind} hook for client {handle} ({cmd}) exited successfully");
                    } else {
                        log::warn!("{kind} hook for client {handle} ({cmd}) exited with {s}");
                    }
                }
                Err(e) => log::warn!("{kind} hook for client {handle} ({cmd}): {e}"),
            }
        });
    }
}

#[derive(Clone, Copy, Debug)]
enum HookKind {
    Enter,
    Leave,
}

impl std::fmt::Display for HookKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookKind::Enter => f.write_str("enter"),
            HookKind::Leave => f.write_str("leave"),
        }
    }
}

/// (VirtualKvm) cheap fingerprint of a clipboard text (change detection only)
fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// (VirtualKvm) display name of a peer
fn peer_name(handle: ClientHandle, config: &lan_mouse_ipc::ClientConfig) -> String {
    config
        .hostname
        .clone()
        .or_else(|| config.fix_ips.first().map(|ip| ip.to_string()))
        .unwrap_or_else(|| format!("client {handle}"))
}

/// (VirtualKvm) open a URL with the desktop's handler. On Windows it goes through
/// explorer.exe so the target app starts in the user's normal (not elevated) context.
fn open_url(url: &str) {
    #[cfg(windows)]
    let r = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe")
            .arg(url)
            .creation_flags(0x0800_0000)
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = r {
        log::warn!("could not open {url}: {e}");
    }
}
