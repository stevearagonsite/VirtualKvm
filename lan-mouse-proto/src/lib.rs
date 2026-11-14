use input_event::{Event as InputEvent, KeyboardEvent, PointerEvent};
use num_enum::{IntoPrimitive, TryFromPrimitive, TryFromPrimitiveError};
use paste::paste;
use std::{
    fmt::{Debug, Display, Formatter},
    mem::size_of,
};
use thiserror::Error;

/// defines the maximum size an encoded event can take up
/// this is currently the pointer motion event
/// type: u8, time: u32, dx: f64, dy: f64
pub const MAX_EVENT_SIZE: usize = size_of::<u8>() + size_of::<u32>() + 2 * size_of::<f64>();

/// error type for protocol violations
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// event type does not exist
    #[error("invalid event id: `{0}`")]
    InvalidEventId(#[from] TryFromPrimitiveError<EventType>),
    /// position type does not exist
    #[error("invalid event id: `{0}`")]
    InvalidPosition(#[from] TryFromPrimitiveError<Position>),
    /// (VirtualKvm) a variable length data packet, see [`DataPacket`]
    #[error("data packet")]
    Data,
}

/// (VirtualKvm) largest datagram we send or accept (fits a typical LAN MTU after DTLS)
pub const MAX_DATAGRAM: usize = 1200;
const DATA_HEADER: usize = 1 + 1 + 4 + 2 + 2;
/// payload bytes per [`DataPacket`]
pub const MAX_DATA_PAYLOAD: usize = MAX_DATAGRAM - DATA_HEADER;

/// (VirtualKvm) one part of a larger message (clipboard, agent messages...).
/// A message is split into `parts` packets that share the same `id`; the
/// receiver answers [`ProtoEvent::DataAck`] once it has all of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataPacket {
    /// what the message is (application defined)
    pub kind: u8,
    pub id: u32,
    pub part: u16,
    pub parts: u16,
    pub payload: Vec<u8>,
}

impl DataPacket {
    /// whether a received datagram is a data packet
    pub fn is_data(buf: &[u8]) -> bool {
        buf.first() == Some(&(EventType::Data as u8))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(DATA_HEADER + self.payload.len());
        out.push(EventType::Data as u8);
        out.push(self.kind);
        out.extend_from_slice(&self.id.to_be_bytes());
        out.extend_from_slice(&self.part.to_be_bytes());
        out.extend_from_slice(&self.parts.to_be_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if !Self::is_data(buf) || buf.len() < DATA_HEADER {
            return None;
        }
        let u16_at = |i: usize| u16::from_be_bytes([buf[i], buf[i + 1]]);
        let packet = Self {
            kind: buf[1],
            id: u32::from_be_bytes([buf[2], buf[3], buf[4], buf[5]]),
            part: u16_at(6),
            parts: u16_at(8),
            payload: buf[DATA_HEADER..].to_vec(),
        };
        (packet.parts > 0 && packet.part < packet.parts).then_some(packet)
    }

    /// split `data` into packets (at most `u16::MAX` parts)
    pub fn split(kind: u8, id: u32, data: &[u8]) -> Vec<DataPacket> {
        let chunks: Vec<&[u8]> = if data.is_empty() {
            vec![&[]]
        } else {
            data.chunks(MAX_DATA_PAYLOAD).collect()
        };
        let parts = chunks.len().min(u16::MAX as usize) as u16;
        chunks
            .into_iter()
            .take(parts as usize)
            .enumerate()
            .map(|(i, c)| DataPacket {
                kind,
                id,
                part: i as u16,
                parts,
                payload: c.to_vec(),
            })
            .collect()
    }
}

/// (VirtualKvm) decode a received datagram: either a fixed size [`ProtoEvent`]
/// or a [`DataPacket`]
pub enum Datagram {
    Event(ProtoEvent),
    Data(DataPacket),
}

impl Datagram {
    pub fn decode(buf: &[u8]) -> Result<Self, ProtocolError> {
        if DataPacket::is_data(buf) {
            return DataPacket::decode(buf)
                .map(Datagram::Data)
                .ok_or(ProtocolError::Data);
        }
        let mut fixed = [0u8; MAX_EVENT_SIZE];
        let n = buf.len().min(MAX_EVENT_SIZE);
        fixed[..n].copy_from_slice(&buf[..n]);
        Ok(Datagram::Event(fixed.try_into()?))
    }
}

/// Position of a client
#[derive(Clone, Copy, Debug, TryFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum Position {
    Left,
    Right,
    Top,
    Bottom,
}

impl Display for Position {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let pos = match self {
            Position::Left => "left",
            Position::Right => "right",
            Position::Top => "top",
            Position::Bottom => "bottom",
        };
        write!(f, "{pos}")
    }
}

/// main lan-mouse protocol event type
#[derive(Clone, Copy, Debug)]
pub enum ProtoEvent {
    /// notify a client that the cursor entered its region at the given position
    /// [`ProtoEvent::Ack`] with the same serial is used for synchronization between devices
    Enter(Position),
    /// notify a client that the cursor left its region
    /// [`ProtoEvent::Ack`] with the same serial is used for synchronization between devices
    Leave(u32),
    /// acknowledge of an [`ProtoEvent::Enter`] or [`ProtoEvent::Leave`] event
    Ack(u32),
    /// Input event
    Input(InputEvent),
    /// Ping event for tracking unresponsive clients.
    /// A client has to respond with [`ProtoEvent::Pong`].
    Ping,
    /// Response to [`ProtoEvent::Ping`], true if emulation is enabled / available
    Pong(bool),
    /// Build identification for the sending peer. Sent by the
    /// connect side once after the connection authenticates, and
    /// echoed back by the listen side in reply, so each end can
    /// display the peer's build hash and warn (soft) on mismatch.
    /// `commit` is the 8-byte ASCII short commit hash from
    /// `shadow_rs`'s `SHORT_COMMIT`. Old peers that don't
    /// recognize the event type silently skip it per the
    /// forward-compat handling in the receive loop.
    Hello { commit: [u8; 8] },
    /// (VirtualKvm) one of the sender's displays, in the sender's own
    /// global desktop coordinates. A peer sends `count` of these
    /// (index `0..count`) after the `Hello` handshake and again when its
    /// display configuration changes.
    DisplayInfo {
        index: u8,
        count: u8,
        x: i32,
        y: i32,
        width: u16,
        height: u16,
        /// scale factor in percent (100 = 1.0, 200 = Retina)
        scale_pct: u16,
    },
    /// (VirtualKvm) like [`ProtoEvent::Enter`], but with the exact entry
    /// point in the *receiver's* global coordinates. The receiver warps
    /// its cursor there before applying further motion.
    EnterAt { pos: Position, x: f64, y: f64 },
    /// (VirtualKvm) like [`ProtoEvent::Leave`], with the point (in the
    /// sender's own coordinates) where the cursor left, so the other side
    /// can place its cursor at the matching spot.
    LeaveAt { x: f64, y: f64 },
    /// (VirtualKvm) the sender placed the receiver's displays at
    /// `sender = receiver + (dx, dy)`; the receiver stores the mirrored layout.
    /// `present == false` removes the layout.
    LayoutSync { present: bool, dx: i32, dy: i32 },
    /// (VirtualKvm) a [`DataPacket`] message with this id arrived complete
    DataAck { id: u32 },
}

impl Display for ProtoEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoEvent::Enter(s) => write!(f, "Enter({s})"),
            ProtoEvent::Leave(s) => write!(f, "Leave({s})"),
            ProtoEvent::Ack(s) => write!(f, "Ack({s})"),
            ProtoEvent::Input(e) => write!(f, "{e}"),
            ProtoEvent::Ping => write!(f, "ping"),
            ProtoEvent::Pong(alive) => {
                write!(
                    f,
                    "pong: {}",
                    if *alive { "alive" } else { "not available" }
                )
            }
            ProtoEvent::Hello { commit } => {
                let s = std::str::from_utf8(commit).unwrap_or("????????");
                write!(f, "Hello({s})")
            }
            ProtoEvent::DisplayInfo {
                index,
                count,
                x,
                y,
                width,
                height,
                scale_pct,
            } => write!(
                f,
                "DisplayInfo({index}/{count}: {width}x{height}@({x},{y}) {scale_pct}%)"
            ),
            ProtoEvent::EnterAt { pos, x, y } => write!(f, "EnterAt({pos}, {x:.0}, {y:.0})"),
            ProtoEvent::LayoutSync { present, dx, dy } => {
                write!(f, "LayoutSync({present}, {dx}, {dy})")
            }
            ProtoEvent::DataAck { id } => write!(f, "DataAck({id})"),
            ProtoEvent::LeaveAt { x, y } => {
                write!(f, "LeaveAt({x:.0}, {y:.0})")
            }
        }
    }
}

#[derive(TryFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum EventType {
    PointerMotion,
    PointerButton,
    PointerAxis,
    PointerAxisValue120,
    KeyboardKey,
    KeyboardModifiers,
    Ping,
    Pong,
    Enter,
    Leave,
    Ack,
    Hello,
    DisplayInfo,
    EnterAt,
    LeaveAt,
    LayoutSync,
    /// variable length [`DataPacket`] (not a [`ProtoEvent`])
    Data,
    DataAck,
}

impl ProtoEvent {
    fn event_type(&self) -> EventType {
        match self {
            ProtoEvent::Input(e) => match e {
                InputEvent::Pointer(p) => match p {
                    PointerEvent::Motion { .. } => EventType::PointerMotion,
                    PointerEvent::Button { .. } => EventType::PointerButton,
                    PointerEvent::Axis { .. } => EventType::PointerAxis,
                    PointerEvent::AxisDiscrete120 { .. } => EventType::PointerAxisValue120,
                },
                InputEvent::Keyboard(k) => match k {
                    KeyboardEvent::Key { .. } => EventType::KeyboardKey,
                    KeyboardEvent::Modifiers { .. } => EventType::KeyboardModifiers,
                },
            },
            ProtoEvent::Ping => EventType::Ping,
            ProtoEvent::Pong(_) => EventType::Pong,
            ProtoEvent::Enter(_) => EventType::Enter,
            ProtoEvent::Leave(_) => EventType::Leave,
            ProtoEvent::Ack(_) => EventType::Ack,
            ProtoEvent::Hello { .. } => EventType::Hello,
            ProtoEvent::DisplayInfo { .. } => EventType::DisplayInfo,
            ProtoEvent::EnterAt { .. } => EventType::EnterAt,
            ProtoEvent::LeaveAt { .. } => EventType::LeaveAt,
            ProtoEvent::LayoutSync { .. } => EventType::LayoutSync,
            ProtoEvent::DataAck { .. } => EventType::DataAck,
        }
    }
}

impl TryFrom<[u8; MAX_EVENT_SIZE]> for ProtoEvent {
    type Error = ProtocolError;

    fn try_from(buf: [u8; MAX_EVENT_SIZE]) -> Result<Self, Self::Error> {
        let mut buf = &buf[..];
        let event_type = decode_u8(&mut buf)?;
        match EventType::try_from(event_type)? {
            EventType::PointerMotion => {
                Ok(Self::Input(InputEvent::Pointer(PointerEvent::Motion {
                    time: decode_u32(&mut buf)?,
                    dx: decode_f64(&mut buf)?,
                    dy: decode_f64(&mut buf)?,
                })))
            }
            EventType::PointerButton => {
                Ok(Self::Input(InputEvent::Pointer(PointerEvent::Button {
                    time: decode_u32(&mut buf)?,
                    button: decode_u32(&mut buf)?,
                    state: decode_u32(&mut buf)?,
                })))
            }
            EventType::PointerAxis => Ok(Self::Input(InputEvent::Pointer(PointerEvent::Axis {
                time: decode_u32(&mut buf)?,
                axis: decode_u8(&mut buf)?,
                value: decode_f64(&mut buf)?,
            }))),
            EventType::PointerAxisValue120 => Ok(Self::Input(InputEvent::Pointer(
                PointerEvent::AxisDiscrete120 {
                    axis: decode_u8(&mut buf)?,
                    value: decode_i32(&mut buf)?,
                },
            ))),
            EventType::KeyboardKey => Ok(Self::Input(InputEvent::Keyboard(KeyboardEvent::Key {
                time: decode_u32(&mut buf)?,
                key: decode_u32(&mut buf)?,
                state: decode_u8(&mut buf)?,
            }))),
            EventType::KeyboardModifiers => Ok(Self::Input(InputEvent::Keyboard(
                KeyboardEvent::Modifiers {
                    depressed: decode_u32(&mut buf)?,
                    latched: decode_u32(&mut buf)?,
                    locked: decode_u32(&mut buf)?,
                    group: decode_u32(&mut buf)?,
                },
            ))),
            EventType::Ping => Ok(Self::Ping),
            EventType::Pong => Ok(Self::Pong(decode_u8(&mut buf)? != 0)),
            EventType::Enter => Ok(Self::Enter(decode_u8(&mut buf)?.try_into()?)),
            EventType::Leave => Ok(Self::Leave(decode_u32(&mut buf)?)),
            EventType::Ack => Ok(Self::Ack(decode_u32(&mut buf)?)),
            EventType::Hello => {
                let mut commit = [0u8; 8];
                for b in commit.iter_mut() {
                    *b = decode_u8(&mut buf)?;
                }
                Ok(Self::Hello { commit })
            }
            EventType::DisplayInfo => Ok(Self::DisplayInfo {
                index: decode_u8(&mut buf)?,
                count: decode_u8(&mut buf)?,
                x: decode_i32(&mut buf)?,
                y: decode_i32(&mut buf)?,
                width: decode_u16(&mut buf)?,
                height: decode_u16(&mut buf)?,
                scale_pct: decode_u16(&mut buf)?,
            }),
            EventType::EnterAt => Ok(Self::EnterAt {
                pos: decode_u8(&mut buf)?.try_into()?,
                x: decode_f64(&mut buf)?,
                y: decode_f64(&mut buf)?,
            }),
            EventType::LeaveAt => Ok(Self::LeaveAt {
                x: decode_f64(&mut buf)?,
                y: decode_f64(&mut buf)?,
            }),
            EventType::LayoutSync => Ok(Self::LayoutSync {
                present: decode_u8(&mut buf)? != 0,
                dx: decode_i32(&mut buf)?,
                dy: decode_i32(&mut buf)?,
            }),
            EventType::DataAck => Ok(Self::DataAck {
                id: decode_u32(&mut buf)?,
            }),
            // data packets are decoded with `DataPacket::decode`
            EventType::Data => Err(ProtocolError::Data),
        }
    }
}

impl From<ProtoEvent> for ([u8; MAX_EVENT_SIZE], usize) {
    fn from(event: ProtoEvent) -> Self {
        let mut buf = [0u8; MAX_EVENT_SIZE];
        let mut len = 0usize;
        {
            let mut buf = &mut buf[..];
            let buf = &mut buf;
            let len = &mut len;
            encode_u8(buf, len, event.event_type() as u8);
            match event {
                ProtoEvent::Input(event) => match event {
                    InputEvent::Pointer(p) => match p {
                        PointerEvent::Motion { time, dx, dy } => {
                            encode_u32(buf, len, time);
                            encode_f64(buf, len, dx);
                            encode_f64(buf, len, dy);
                        }
                        PointerEvent::Button {
                            time,
                            button,
                            state,
                        } => {
                            encode_u32(buf, len, time);
                            encode_u32(buf, len, button);
                            encode_u32(buf, len, state);
                        }
                        PointerEvent::Axis { time, axis, value } => {
                            encode_u32(buf, len, time);
                            encode_u8(buf, len, axis);
                            encode_f64(buf, len, value);
                        }
                        PointerEvent::AxisDiscrete120 { axis, value } => {
                            encode_u8(buf, len, axis);
                            encode_i32(buf, len, value);
                        }
                    },
                    InputEvent::Keyboard(k) => match k {
                        KeyboardEvent::Key { time, key, state } => {
                            encode_u32(buf, len, time);
                            encode_u32(buf, len, key);
                            encode_u8(buf, len, state);
                        }
                        KeyboardEvent::Modifiers {
                            depressed,
                            latched,
                            locked,
                            group,
                        } => {
                            encode_u32(buf, len, depressed);
                            encode_u32(buf, len, latched);
                            encode_u32(buf, len, locked);
                            encode_u32(buf, len, group);
                        }
                    },
                },
                ProtoEvent::Ping => {}
                ProtoEvent::Pong(alive) => encode_u8(buf, len, alive as u8),
                ProtoEvent::Enter(pos) => encode_u8(buf, len, pos as u8),
                ProtoEvent::Leave(serial) => encode_u32(buf, len, serial),
                ProtoEvent::Ack(serial) => encode_u32(buf, len, serial),
                ProtoEvent::Hello { commit } => {
                    for b in commit.iter() {
                        encode_u8(buf, len, *b);
                    }
                }
                ProtoEvent::DisplayInfo {
                    index,
                    count,
                    x,
                    y,
                    width,
                    height,
                    scale_pct,
                } => {
                    encode_u8(buf, len, index);
                    encode_u8(buf, len, count);
                    encode_i32(buf, len, x);
                    encode_i32(buf, len, y);
                    encode_u16(buf, len, width);
                    encode_u16(buf, len, height);
                    encode_u16(buf, len, scale_pct);
                }
                ProtoEvent::EnterAt { pos, x, y } => {
                    encode_u8(buf, len, pos as u8);
                    encode_f64(buf, len, x);
                    encode_f64(buf, len, y);
                }
                ProtoEvent::LeaveAt { x, y } => {
                    encode_f64(buf, len, x);
                    encode_f64(buf, len, y);
                }
                ProtoEvent::LayoutSync { present, dx, dy } => {
                    encode_u8(buf, len, present as u8);
                    encode_i32(buf, len, dx);
                    encode_i32(buf, len, dy);
                }
                ProtoEvent::DataAck { id } => encode_u32(buf, len, id),
            }
        }
        (buf, len)
    }
}

macro_rules! decode_impl {
    ($t:ty) => {
        paste! {
            fn [<decode_ $t>](data: &mut &[u8]) -> Result<$t, ProtocolError> {
                let (int_bytes, rest) = data.split_at(size_of::<$t>());
                *data = rest;
                Ok($t::from_be_bytes(int_bytes.try_into().unwrap()))
            }
        }
    };
}

decode_impl!(u8);
decode_impl!(u16);
decode_impl!(u32);
decode_impl!(i32);
decode_impl!(f64);

macro_rules! encode_impl {
    ($t:ty) => {
        paste! {
            fn [<encode_ $t>](buf: &mut &mut [u8], amt: &mut usize, n: $t) {
                let src = n.to_be_bytes();
                let data = std::mem::take(buf);
                let (int_bytes, rest) = data.split_at_mut(size_of::<$t>());
                int_bytes.copy_from_slice(&src);
                *amt += size_of::<$t>();
                *buf = rest
            }
        }
    };
}

encode_impl!(u8);
encode_impl!(u16);
encode_impl!(u32);
encode_impl!(i32);
encode_impl!(f64);

#[cfg(test)]
mod vkvm_tests {
    use super::*;

    fn roundtrip(e: ProtoEvent) -> ProtoEvent {
        let (buf, _len): ([u8; MAX_EVENT_SIZE], usize) = e.into();
        buf.try_into().expect("decode")
    }

    #[test]
    fn display_info_roundtrip() {
        match roundtrip(ProtoEvent::DisplayInfo {
            index: 1,
            count: 2,
            x: 927,
            y: -1080,
            width: 1920,
            height: 1080,
            scale_pct: 100,
        }) {
            ProtoEvent::DisplayInfo {
                index: 1,
                count: 2,
                x: 927,
                y: -1080,
                width: 1920,
                height: 1080,
                scale_pct: 100,
            } => {}
            e => panic!("unexpected {e}"),
        }
    }

    #[test]
    fn enter_at_roundtrip() {
        match roundtrip(ProtoEvent::EnterAt {
            pos: Position::Right,
            x: 1918.0,
            y: 540.5,
        }) {
            ProtoEvent::EnterAt {
                pos: Position::Right,
                x,
                y,
            } => assert_eq!((x, y), (1918.0, 540.5)),
            e => panic!("unexpected {e}"),
        }
    }

    #[test]
    fn layout_sync_roundtrip() {
        match roundtrip(ProtoEvent::LayoutSync {
            present: true,
            dx: -993,
            dy: -1080,
        }) {
            ProtoEvent::LayoutSync {
                present: true,
                dx: -993,
                dy: -1080,
            } => {}
            e => panic!("unexpected {e}"),
        }
    }

    #[test]
    fn data_packets_roundtrip() {
        let data: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
        let packets = DataPacket::split(1, 42, &data);
        assert_eq!(packets.len(), 3);
        let mut joined = Vec::new();
        for p in &packets {
            let enc = p.encode();
            assert!(enc.len() <= MAX_DATAGRAM);
            match Datagram::decode(&enc).unwrap() {
                Datagram::Data(d) => {
                    assert_eq!(&d, p);
                    joined.extend(d.payload);
                }
                Datagram::Event(_) => panic!("expected data"),
            }
        }
        assert_eq!(joined, data);
        // fixed events still decode through Datagram
        let (buf, len): ([u8; MAX_EVENT_SIZE], usize) = ProtoEvent::DataAck { id: 42 }.into();
        assert!(matches!(
            Datagram::decode(&buf[..len]).unwrap(),
            Datagram::Event(ProtoEvent::DataAck { id: 42 })
        ));
    }

    #[test]
    fn leave_at_roundtrip() {
        match roundtrip(ProtoEvent::LeaveAt { x: -1.0, y: 300.0 }) {
            ProtoEvent::LeaveAt { x, y } => assert_eq!((x, y), (-1.0, 300.0)),
            e => panic!("unexpected {e}"),
        }
    }
}
