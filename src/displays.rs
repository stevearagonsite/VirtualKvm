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

/// side of a local display where a peer display touches it
pub(crate) type Side = lan_mouse_ipc::Position;

/// a stretch of a local display edge where the cursor crosses into the peer
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Contact {
    pub side: Side,
    /// the local display whose edge this is
    pub display: Rect,
    /// range `[from, to)` along the edge (y for left/right, x for top/bottom)
    pub from: f64,
    pub to: f64,
}

/// distance (px) under which two edges count as touching
const TOUCH: f64 = 2.0;

/// resolved geometry of a [`lan_mouse_ipc::DisplayLayout`]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    /// our displays (local coordinates)
    pub local: Vec<Rect>,
    /// the peer's displays (peer coordinates)
    pub remote: Vec<Rect>,
    /// peer point + (dx, dy) = local point
    pub dx: f64,
    pub dy: f64,
}

impl Layout {
    pub(crate) fn new(
        layout: lan_mouse_ipc::DisplayLayout,
        local: &[DisplayInfo],
        remote: &[DisplayInfo],
    ) -> Self {
        Self {
            local: local.iter().map(Rect::from).collect(),
            remote: remote.iter().map(Rect::from).collect(),
            dx: layout.dx as f64,
            dy: layout.dy as f64,
        }
    }

    /// the peer's displays in local coordinates
    pub(crate) fn placed(&self) -> Vec<Rect> {
        self.remote
            .iter()
            .map(|r| Rect {
                x: r.x + self.dx,
                y: r.y + self.dy,
                ..*r
            })
            .collect()
    }

    /// whether a peer display (local coordinates) is covered by one of ours.
    /// With a physical KVM the same monitor can switch to this machine: then it
    /// shows up locally right where the layout puts the peer's display, which
    /// is dark for the peer. The cursor must stay here instead of crossing.
    fn covered(&self, placed: &Rect) -> bool {
        self.local.iter().any(|l| {
            (l.x + l.w).min(placed.x + placed.w) - l.x.max(placed.x) > TOUCH
                && (l.y + l.h).min(placed.y + placed.h) - l.y.max(placed.y) > TOUCH
        })
    }

    /// the peer's displays the cursor can cross into (local coordinates)
    fn reachable(&self) -> Vec<Rect> {
        self.placed()
            .into_iter()
            .filter(|r| !self.covered(r))
            .collect()
    }

    /// every stretch of a local edge touched by a reachable peer display
    pub(crate) fn contacts(&self) -> Vec<Contact> {
        use lan_mouse_ipc::Position::*;
        let mut out = Vec::new();
        for l in &self.local {
            for r in self.reachable() {
                let y_overlap = (l.y.max(r.y), (l.y + l.h).min(r.y + r.h));
                let x_overlap = (l.x.max(r.x), (l.x + l.w).min(r.x + r.w));
                let candidates = [
                    (Left, (r.x + r.w - l.x).abs(), y_overlap),
                    (Right, (r.x - (l.x + l.w)).abs(), y_overlap),
                    (Top, (r.y + r.h - l.y).abs(), x_overlap),
                    (Bottom, (r.y - (l.y + l.h)).abs(), x_overlap),
                ];
                for (side, gap, (from, to)) in candidates {
                    if gap <= TOUCH && to > from {
                        out.push(Contact {
                            side,
                            display: *l,
                            from,
                            to,
                        });
                    }
                }
            }
        }
        out
    }

    /// side with the longest contact (used as the client position)
    pub(crate) fn primary_side(&self) -> Option<Side> {
        self.contacts()
            .into_iter()
            .max_by(|a, b| (a.to - a.from).total_cmp(&(b.to - b.from)))
            .map(|c| c.side)
    }

    /// whether a peer display overlaps one of ours (invalid layout)
    pub(crate) fn overlaps(&self) -> bool {
        self.placed().iter().any(|r| self.covered(r))
    }

    /// entry point (peer coordinates) for a crossing at local `(x, y)` through `side`
    pub(crate) fn to_remote(&self, side: Side, x: f64, y: f64) -> Option<(f64, f64)> {
        use lan_mouse_ipc::Position::*;
        // a point just beyond the local edge
        let (tx, ty) = match side {
            Left => (x - 1.0, y),
            Right => (x + 1.0, y),
            Top => (x, y - 1.0),
            Bottom => (x, y + 1.0),
        };
        let (rx, ry) = (tx - self.dx, ty - self.dy);
        let r = self
            .remote
            .iter()
            .filter(|r| {
                !self.covered(&Rect {
                    x: r.x + self.dx,
                    y: r.y + self.dy,
                    ..**r
                })
            })
            .filter(|r| {
                rx >= r.x - TOUCH - 1.0
                    && rx < r.x + r.w + TOUCH + 1.0
                    && ry >= r.y - TOUCH - 1.0
                    && ry < r.y + r.h + TOUCH + 1.0
            })
            .min_by(|a, b| dist2(a, rx, ry).total_cmp(&dist2(b, rx, ry)))?;
        let cx = rx.clamp(r.x, r.x + r.w - 1.0);
        let cy = ry.clamp(r.y, r.y + r.h - 1.0);
        // enter just inside the edge we come through
        Some(match side {
            Left => (r.x + r.w - 2.0, cy),
            Right => (r.x + 1.0, cy),
            Top => (cx, r.y + r.h - 2.0),
            Bottom => (cx, r.y + 1.0),
        })
    }

    /// where our cursor should reappear when it leaves the peer at peer `(xr, yr)`
    pub(crate) fn to_local(&self, xr: f64, yr: f64) -> Option<(f64, f64)> {
        let (x, y) = (xr + self.dx, yr + self.dy);
        self.local
            .iter()
            .min_by(|a, b| dist2(a, x, y).total_cmp(&dist2(b, x, y)))
            .map(|l| {
                (
                    x.clamp(l.x + 1.0, l.x + l.w - 2.0),
                    y.clamp(l.y + 1.0, l.y + l.h - 2.0),
                )
            })
    }
}

/// squared distance from `(x, y)` to rectangle `r` (0 inside)
fn dist2(r: &Rect, x: f64, y: f64) -> f64 {
    let dx = (r.x - x).max(0.0).max(x - (r.x + r.w));
    let dy = (r.y - y).max(0.0).max(y - (r.y + r.h));
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;
    use lan_mouse_ipc::{DisplayLayout, Position};

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

    /// Steve's setup seen from the Mac: the Windows display (1920x1080) sits left
    /// of the LF27T35, above the free left part of the built-in display.
    fn mac() -> Layout {
        let local = [
            display("Built-in", 0, 0, 1512, 982),
            display("LF27T35", 927, -1080, 1920, 1080),
        ];
        let remote = [display("DISPLAY1", 0, 0, 1920, 1080)];
        Layout::new(
            DisplayLayout {
                dx: -993,
                dy: -1080,
            },
            &local,
            &remote,
        )
    }

    fn windows() -> Layout {
        let local = [display("DISPLAY1", 0, 0, 1920, 1080)];
        let remote = [
            display("Built-in", 0, 0, 1512, 982),
            display("LF27T35", 927, -1080, 1920, 1080),
        ];
        Layout::new(
            DisplayLayout {
                dx: -993,
                dy: -1080,
            }
            .mirrored(),
            &local,
            &remote,
        )
    }

    #[test]
    fn both_touching_edges_are_contacts() {
        let mut c = mac().contacts();
        c.sort_by(|a, b| a.from.total_cmp(&b.from));
        assert_eq!(c.len(), 2);
        // left edge of the LF27T35, full height
        assert_eq!(
            (c[0].side, c[0].from, c[0].to),
            (Position::Left, -1080.0, 0.0)
        );
        // top edge of the built-in display, the free part left of the LF27T35
        assert_eq!((c[1].side, c[1].from, c[1].to), (Position::Top, 0.0, 927.0));
        assert_eq!(mac().primary_side(), Some(Position::Left));
        assert!(!mac().overlaps());
    }

    #[test]
    fn entry_is_aligned_one_to_one() {
        let g = mac();
        // from the LF27T35's left edge at mid height
        assert_eq!(
            g.to_remote(Position::Left, 927.0, -540.0),
            Some((1918.0, 540.0))
        );
        // up from the built-in display at x = 100 -> bottom of the Windows display
        assert_eq!(
            g.to_remote(Position::Top, 100.0, 0.0),
            Some((1093.0, 1078.0))
        );
    }

    #[test]
    fn return_is_aligned_one_to_one() {
        let g = mac();
        // leaving the Windows display through its right edge at y = 300
        assert_eq!(g.to_local(1919.0, 300.0), Some((928.0, -780.0)));
        // leaving through its bottom edge at x = 1093
        assert_eq!(g.to_local(1093.0, 1079.0), Some((100.0, 1.0)));
    }

    #[test]
    fn mirrored_layout_matches_from_windows() {
        let w = windows();
        let mut c = w.contacts();
        c.sort_by(|a, b| a.from.total_cmp(&b.from));
        assert_eq!(c.len(), 2);
        assert_eq!(
            (c[0].side, c[0].from, c[0].to),
            (Position::Right, 0.0, 1080.0)
        );
        assert_eq!(
            (c[1].side, c[1].from, c[1].to),
            (Position::Bottom, 993.0, 1920.0)
        );
        assert_eq!(
            w.to_remote(Position::Right, 1919.0, 540.0),
            Some((928.0, -540.0))
        );
    }

    /// the physical KVM switched the Windows monitor to the Mac: it appears
    /// as a third Mac display exactly where the layout places the Windows one
    fn mac_with_switched_monitor() -> Layout {
        let local = [
            display("Built-in", 0, 0, 1512, 982),
            display("LF27T35", -993, -1080, 1920, 1080),
            display("LF27T35", 927, -1080, 1920, 1080),
        ];
        let remote = [display("DISPLAY1", 0, 0, 1920, 1080)];
        Layout::new(
            DisplayLayout {
                dx: -993,
                dy: -1080,
            },
            &local,
            &remote,
        )
    }

    #[test]
    fn switched_monitor_stays_local() {
        let g = mac_with_switched_monitor();
        // no edge leads to the (dark) Windows display: the cursor stays on the Mac
        assert!(g.contacts().is_empty());
        assert_eq!(g.primary_side(), None);
        assert_eq!(g.to_remote(Position::Left, 927.0, -540.0), None);
        // coming back from the Windows still lands on a Mac display
        assert_eq!(g.to_local(1919.0, 300.0), Some((925.0, -780.0)));
    }

    #[test]
    fn switching_back_restores_the_edges() {
        // same layout, the monitor went back to the Windows
        assert_eq!(mac().contacts().len(), 2);
    }

    #[test]
    fn overlapping_layout_is_detected() {
        let local = [display("A", 0, 0, 1000, 1000)];
        let remote = [display("B", 0, 0, 1000, 1000)];
        assert!(Layout::new(DisplayLayout { dx: -500, dy: 0 }, &local, &remote).overlaps());
        assert!(!Layout::new(DisplayLayout { dx: -1000, dy: 0 }, &local, &remote).overlaps());
    }
}
