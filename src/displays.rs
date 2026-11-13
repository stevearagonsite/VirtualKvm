//! (VirtualKvm) exchange of display geometry between peers.
//! See `docs/design/layout-gui.md`.

use lan_mouse_ipc::DisplayInfo;
use lan_mouse_proto::ProtoEvent;

/// `DisplayInfo` events describing the displays of this machine
pub(crate) fn local_display_events() -> Vec<ProtoEvent> {
    let displays = vkvm_displays::list();
    let count = displays.len().min(u8::MAX as usize) as u8;
    displays
        .iter()
        .take(count as usize)
        .enumerate()
        .map(|(i, d)| ProtoEvent::DisplayInfo {
            index: i as u8,
            count,
            x: d.x,
            y: d.y,
            width: d.width.min(u16::MAX as u32) as u16,
            height: d.height.min(u16::MAX as u32) as u16,
            scale_pct: (d.scale * 100.0).round().clamp(0.0, u16::MAX as f64) as u16,
        })
        .collect()
}

/// build a [`DisplayInfo`] from a received `DisplayInfo` event
pub(crate) fn remote_display(
    index: u8,
    x: i32,
    y: i32,
    width: u16,
    height: u16,
    scale_pct: u16,
) -> DisplayInfo {
    DisplayInfo {
        id: index.to_string(),
        name: format!("Monitor {}", index as u32 + 1),
        x,
        y,
        width: width as u32,
        height: height as u32,
        scale: scale_pct as f64 / 100.0,
        // both macOS and Windows put the primary display at the origin
        primary: x == 0 && y == 0,
    }
}

/// axis-aligned rectangle in global desktop coordinates
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl From<&DisplayInfo> for Rect {
    fn from(d: &DisplayInfo) -> Self {
        Self {
            x: d.x as f64,
            y: d.y as f64,
            w: d.width as f64,
            h: d.height as f64,
        }
    }
}

/// resolved geometry of a [`lan_mouse_ipc::DisplayLink`]:
/// a local display, the side where the remote display sits and where along
/// that edge it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LinkGeometry {
    /// side of `local` where the remote display is
    pub pos: lan_mouse_ipc::Position,
    pub local: Rect,
    /// remote display in the *peer's* coordinates (unknown until announced)
    pub remote: Option<Rect>,
    /// start of the remote display along the edge, relative to the start of `local`
    pub offset: f64,
}

impl LinkGeometry {
    pub(crate) fn resolve(
        link: &lan_mouse_ipc::DisplayLink,
        pos: lan_mouse_ipc::Position,
        local_displays: &[DisplayInfo],
        remote_displays: &[DisplayInfo],
    ) -> Option<Self> {
        let local = local_displays
            .iter()
            .find(|d| d.matches(&link.local_display))?;
        let remote = remote_displays
            .get(link.remote_display as usize)
            .map(Rect::from);
        Some(Self {
            pos,
            local: local.into(),
            remote,
            offset: link.offset as f64,
        })
    }

    fn horizontal(&self) -> bool {
        matches!(
            self.pos,
            lan_mouse_ipc::Position::Left | lan_mouse_ipc::Position::Right
        )
    }

    /// (start, length) of `r` along the shared edge
    fn along(&self, r: &Rect) -> (f64, f64) {
        if self.horizontal() {
            (r.y, r.h)
        } else {
            (r.x, r.w)
        }
    }

    /// start of the placed remote display along the edge, in local coordinates
    fn placed_start(&self) -> f64 {
        self.along(&self.local).0 + self.offset
    }

    fn remote_len(&self) -> f64 {
        match &self.remote {
            Some(r) => self.along(r).1,
            None => self.along(&self.local).1,
        }
    }

    /// active part `[from, to)` of the local edge (global local coordinates).
    /// Empty (`from >= to`) if the remote display doesn't touch the local one.
    pub(crate) fn segment(&self) -> (f64, f64) {
        let (start, len) = self.along(&self.local);
        let placed = self.placed_start();
        (
            start.max(placed),
            (start + len).min(placed + self.remote_len()),
        )
    }

    /// entry point on the remote display for a crossing at local `(x, y)`
    pub(crate) fn to_remote(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        use lan_mouse_ipc::Position::*;
        let r = self.remote?;
        let local_along = if self.horizontal() { y } else { x };
        let along = (local_along - self.placed_start()).clamp(0.0, self.remote_len() - 1.0);
        Some(match self.pos {
            // remote is left of us: enter it at its right edge
            Left => (r.x + r.w - 2.0, r.y + along),
            Right => (r.x + 1.0, r.y + along),
            Top => (r.x + along, r.y + r.h - 2.0),
            Bottom => (r.x + along, r.y + 1.0),
        })
    }

    /// where the local cursor should reappear when leaving the remote display at `(xr, yr)`
    pub(crate) fn to_local(&self, xr: f64, yr: f64) -> (f64, f64) {
        use lan_mouse_ipc::Position::*;
        let (from, to) = self.segment();
        let along = match &self.remote {
            Some(r) if self.horizontal() => yr - r.y,
            Some(r) => xr - r.x,
            None if self.horizontal() => yr,
            None => xr,
        };
        let local_along = (self.placed_start() + along).clamp(from, (to - 1.0).max(from));
        let l = &self.local;
        match self.pos {
            Left => (l.x + 1.0, local_along),
            Right => (l.x + l.w - 2.0, local_along),
            Top => (local_along, l.y + 1.0),
            Bottom => (local_along, l.y + l.h - 2.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lan_mouse_ipc::{DisplayLink, Position};

    fn display(id: &str, x: i32, y: i32, w: u32, h: u32) -> DisplayInfo {
        DisplayInfo {
            id: id.into(),
            name: id.into(),
            x,
            y,
            width: w,
            height: h,
            scale: 1.0,
            primary: x == 0 && y == 0,
        }
    }

    /// Steve's setup: Windows (1920x1080) left of the Mac's LF27T35, which
    /// sits above the built-in display.
    fn steve(offset: i32) -> LinkGeometry {
        let local = [
            display("1", 0, 0, 1512, 982),
            display("LF27T35", 927, -1080, 1920, 1080),
        ];
        let remote = [display("0", 0, 0, 1920, 1080)];
        let link = DisplayLink {
            local_display: "LF27T35".into(),
            remote_display: 0,
            offset,
        };
        LinkGeometry::resolve(&link, Position::Left, &local, &remote).unwrap()
    }

    #[test]
    fn aligned_segment_is_whole_edge() {
        assert_eq!(steve(0).segment(), (-1080.0, 0.0));
    }

    #[test]
    fn aligned_entry_is_one_to_one() {
        let g = steve(0);
        assert_eq!(g.to_remote(927.0, -540.0), Some((1918.0, 540.0)));
        assert_eq!(g.to_remote(927.0, -1080.0), Some((1918.0, 0.0)));
        assert_eq!(g.to_remote(927.0, -1.0), Some((1918.0, 1079.0)));
    }

    #[test]
    fn aligned_return_is_one_to_one() {
        let g = steve(0);
        assert_eq!(g.to_local(1.0, 300.0), (928.0, -780.0));
    }

    #[test]
    fn offset_shifts_and_clips_segment() {
        // remote placed 200 px lower than the local display
        let g = steve(200);
        assert_eq!(g.segment(), (-880.0, 0.0));
        assert_eq!(g.to_remote(927.0, -880.0), Some((1918.0, 0.0)));
        // returning from below the shared part clamps into the segment
        assert_eq!(g.to_local(1.0, 1000.0), (928.0, -1.0));
    }

    #[test]
    fn unknown_remote_has_no_entry_point() {
        let local = [display("LF27T35", 927, -1080, 1920, 1080)];
        let link = DisplayLink {
            local_display: "lf27t35".into(),
            remote_display: 0,
            offset: 0,
        };
        let g = LinkGeometry::resolve(&link, Position::Left, &local, &[]).unwrap();
        assert_eq!(g.segment(), (-1080.0, 0.0));
        assert_eq!(g.to_remote(927.0, -10.0), None);
    }
}
