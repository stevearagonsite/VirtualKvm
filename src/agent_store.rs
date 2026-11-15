//! (VirtualKvm) durable state of the agent channel (`vkvm mcp`, vkvm-worker):
//! the inbox, the outgoing messages not acknowledged yet, and the ids already
//! received. It survives daemon restarts, so a restart in the middle of a job
//! no longer loses a reply, and a message resent after a restart is recognized
//! and dropped as a duplicate.

use std::{
    collections::VecDeque,
    fs, io,
    path::{Path, PathBuf},
    time::Instant,
};

use lan_mouse_ipc::AgentMessage;
use serde::{Deserialize, Serialize};

/// ids of received messages remembered for de-duplication
const SEEN_MAX: usize = 1000;
/// messages kept in the inbox
pub(crate) const INBOX_MAX: usize = 100;
/// unacknowledged messages kept for retry
const OUTBOX_MAX: usize = 100;

/// an agent message waiting for the peer's acknowledgement
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Outgoing {
    /// message id (also inside `payload`)
    pub id: String,
    /// peer (client) name it goes to
    pub peer: String,
    /// encoded KIND_AGENT_ADDRESSED payload
    pub payload: String,
    /// transport id of the current attempt
    #[serde(skip)]
    pub in_flight: Option<u32>,
    #[serde(skip)]
    pub last_try: Option<Instant>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct AgentState {
    #[serde(default)]
    pub inbox: VecDeque<AgentMessage>,
    #[serde(default)]
    pub outbox: Vec<Outgoing>,
    /// `peer:id` of received messages
    #[serde(default)]
    pub seen: VecDeque<String>,
}

impl AgentState {
    pub(crate) fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("agent-state.json")
    }

    /// load the state; a missing or damaged file gives an empty state
    pub(crate) fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                log::warn!("ignoring damaged agent state {path:?}: {e}");
                Self::default()
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                log::warn!("could not read agent state {path:?}: {e}");
                Self::default()
            }
        }
    }

    /// write atomically (temporary file + rename)
    pub(crate) fn save(&self, path: &Path) {
        let tmp = path.with_extension("json.tmp");
        let result = serde_json::to_vec(self)
            .map_err(io::Error::other)
            .and_then(|data| fs::write(&tmp, data))
            .and_then(|()| fs::rename(&tmp, path));
        if let Err(e) = result {
            log::warn!("could not save agent state {path:?}: {e}");
        }
    }

    /// remember a received message; false if it was already received
    pub(crate) fn first_time(&mut self, peer: &str, id: &str) -> bool {
        let key = format!("{peer}:{id}");
        if self.seen.contains(&key) {
            return false;
        }
        if self.seen.len() >= SEEN_MAX {
            self.seen.pop_front();
        }
        self.seen.push_back(key);
        true
    }

    pub(crate) fn push_inbox(&mut self, message: AgentMessage) {
        if self.inbox.len() >= INBOX_MAX {
            self.inbox.pop_front();
        }
        self.inbox.push_back(message);
    }

    pub(crate) fn push_outbox(&mut self, outgoing: Outgoing) {
        if self.outbox.len() >= OUTBOX_MAX {
            let dropped = self.outbox.remove(0);
            log::warn!("dropping unacknowledged agent message {}", dropped.id);
        }
        self.outbox.push(outgoing);
    }
}

/// id for a new outgoing message (unique per sender, also across restarts)
pub(crate) fn new_message_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}-{:x}", SEQ.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> AgentMessage {
        AgentMessage {
            from: "pc".into(),
            text: text.into(),
            received_at: 1,
            from_mailbox: Some("blender".into()),
            to: Some("coordinator".into()),
            id: Some("abc".into()),
        }
    }

    #[test]
    fn state_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("vkvm-agent-test-{}", new_message_id()));
        fs::create_dir_all(&dir).unwrap();
        let path = AgentState::path(&dir);
        let mut state = AgentState::default();
        state.push_inbox(message("hola"));
        state.push_outbox(Outgoing {
            id: "1".into(),
            peer: "pc".into(),
            payload: "{}".into(),
            in_flight: Some(7),
            last_try: None,
        });
        assert!(state.first_time("pc", "abc"));
        state.save(&path);

        let mut loaded = AgentState::load(&path);
        assert_eq!(loaded.inbox.len(), 1);
        assert_eq!(loaded.inbox[0].text, "hola");
        assert_eq!(loaded.outbox.len(), 1);
        assert_eq!(loaded.outbox[0].in_flight, None); // retried after the restart
        assert!(!loaded.first_time("pc", "abc")); // duplicate after the restart
        assert!(loaded.first_time("mac", "abc")); // same id from another peer
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn damaged_file_gives_empty_state() {
        let dir = std::env::temp_dir().join(format!("vkvm-agent-test-{}", new_message_id()));
        fs::create_dir_all(&dir).unwrap();
        let path = AgentState::path(&dir);
        fs::write(&path, "{ not json").unwrap();
        assert!(AgentState::load(&path).inbox.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ids_are_unique() {
        let a = new_message_id();
        let b = new_message_id();
        assert_ne!(a, b);
    }
}
