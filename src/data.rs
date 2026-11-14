//! (VirtualKvm) reliable messages on top of the DTLS datagrams: clipboard
//! contents, agent messages... A message is split into [`DataPacket`]s, the
//! receiver reassembles it and answers [`ProtoEvent::DataAck`]; the sender
//! retries a few times when the ack does not arrive.
//!
//! [`ProtoEvent::DataAck`]: lan_mouse_proto::ProtoEvent::DataAck

use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    time::{Duration, Instant},
};

use lan_mouse_ipc::ClientHandle;
use lan_mouse_proto::DataPacket;

/// clipboard text (UTF-8)
pub(crate) const KIND_CLIPBOARD: u8 = 1;
/// message for the AI agent on the peer (UTF-8), see `vkvm mcp`
pub(crate) const KIND_AGENT: u8 = 2;

/// largest message we send or accept
pub(crate) const MAX_MESSAGE: usize = 512 * 1024;

const RETRY_AFTER: Duration = Duration::from_millis(700);
const MAX_ATTEMPTS: u32 = 4;
const DONE_MEMORY: usize = 256;

/// how a message reaches the peer: through our outgoing connection to a
/// client, or as a reply on the peer's incoming connection
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Peer {
    Client(ClientHandle),
    Addr(SocketAddr),
}

/// something received from the network for the hub
pub(crate) enum DataIn {
    Packet { from: Peer, packet: DataPacket },
    Ack { id: u32 },
}

struct Outgoing {
    to: Peer,
    packets: Vec<DataPacket>,
    sent: Instant,
    attempts: u32,
}

struct Assembly {
    kind: u8,
    parts: Vec<Option<Vec<u8>>>,
    size: usize,
}

/// what the hub wants the service to do after an incoming packet
pub(crate) struct Received {
    /// acknowledge message `id` to `to`
    pub ack: Option<(Peer, u32)>,
    /// a complete, new message
    pub message: Option<(Peer, u8, Vec<u8>)>,
}

#[derive(Default)]
pub(crate) struct DataHub {
    next_id: u32,
    outgoing: HashMap<u32, Outgoing>,
    incoming: HashMap<(Peer, u32), Assembly>,
    done: VecDeque<(Peer, u32)>,
    done_set: HashSet<(Peer, u32)>,
}

impl DataHub {
    /// queue a message; returns its packets (to be sent now)
    pub(crate) fn send(&mut self, to: Peer, kind: u8, data: &[u8]) -> Option<Vec<DataPacket>> {
        if data.len() > MAX_MESSAGE {
            log::warn!("not sending {} bytes (kind {kind}): too large", data.len());
            return None;
        }
        // random-ish start so ids from a restarted daemon don't look like duplicates
        if self.next_id == 0 {
            self.next_id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() | 1)
                .unwrap_or(1);
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let packets = DataPacket::split(kind, id, data);
        self.outgoing.insert(
            id,
            Outgoing {
                to,
                packets: packets.clone(),
                sent: Instant::now(),
                attempts: 1,
            },
        );
        Some(packets)
    }

    pub(crate) fn acked(&mut self, id: u32) {
        self.outgoing.remove(&id);
    }

    /// messages to send again (and give up on the ones retried too often)
    pub(crate) fn retries(&mut self, now: Instant) -> Vec<(Peer, Vec<DataPacket>)> {
        let mut out = Vec::new();
        self.outgoing.retain(|id, o| {
            if now.duration_since(o.sent) < RETRY_AFTER {
                return true;
            }
            if o.attempts >= MAX_ATTEMPTS {
                log::warn!("message {id} to {:?} was never acknowledged", o.to);
                return false;
            }
            o.attempts += 1;
            o.sent = now;
            out.push((o.to, o.packets.clone()));
            true
        });
        out
    }

    pub(crate) fn received(&mut self, from: Peer, packet: DataPacket) -> Received {
        let key = (from, packet.id);
        // duplicate of a completed message (our ack got lost): ack again
        if self.done_set.contains(&key) {
            return Received {
                ack: Some((from, packet.id)),
                message: None,
            };
        }
        let parts = packet.parts as usize;
        let assembly = self.incoming.entry(key).or_insert_with(|| Assembly {
            kind: packet.kind,
            parts: vec![None; parts],
            size: 0,
        });
        if assembly.parts.len() != parts || assembly.kind != packet.kind {
            log::warn!("inconsistent data packet {} from {from:?}", packet.id);
            self.incoming.remove(&key);
            return Received {
                ack: None,
                message: None,
            };
        }
        let slot = &mut assembly.parts[packet.part as usize];
        if slot.is_none() {
            assembly.size += packet.payload.len();
            *slot = Some(packet.payload);
        }
        if assembly.size > MAX_MESSAGE {
            log::warn!("dropping message {} from {from:?}: too large", packet.id);
            self.incoming.remove(&key);
            return Received {
                ack: None,
                message: None,
            };
        }
        if assembly.parts.iter().any(Option::is_none) {
            return Received {
                ack: None,
                message: None,
            };
        }
        let assembly = self.incoming.remove(&key).expect("assembly");
        let data = assembly.parts.into_iter().flatten().flatten().collect();
        self.done.push_back(key);
        self.done_set.insert(key);
        if self.done.len() > DONE_MEMORY {
            if let Some(old) = self.done.pop_front() {
                self.done_set.remove(&old);
            }
        }
        Received {
            ack: Some((from, packet.id)),
            message: Some((from, assembly.kind, data)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_is_reassembled_out_of_order_and_acked_once() {
        let mut tx = DataHub::default();
        let mut rx = DataHub::default();
        let data: Vec<u8> = (0..5000u32).map(|i| i as u8).collect();
        let mut packets = tx.send(Peer::Client(0), KIND_CLIPBOARD, &data).unwrap();
        packets.reverse();
        let from = Peer::Client(7);
        let mut got = None;
        for p in packets.clone() {
            let r = rx.received(from, p);
            if let Some(m) = r.message {
                got = Some(m);
                let (_, id) = r.ack.unwrap();
                tx.acked(id);
            }
        }
        assert_eq!(got, Some((from, KIND_CLIPBOARD, data)));
        assert!(tx.outgoing.is_empty());
        // a retransmission is acked again but not delivered twice
        let r = rx.received(from, packets[0].clone());
        assert!(r.ack.is_some() && r.message.is_none());
    }

    #[test]
    fn unacked_messages_are_retried_then_dropped() {
        let mut tx = DataHub::default();
        tx.send(Peer::Client(0), KIND_AGENT, b"hola").unwrap();
        let mut t = Instant::now();
        let mut retries = 0;
        for _ in 0..10 {
            t += RETRY_AFTER;
            retries += tx.retries(t).len();
        }
        assert_eq!(retries as u32, MAX_ATTEMPTS - 1);
        assert!(tx.outgoing.is_empty());
    }
}
