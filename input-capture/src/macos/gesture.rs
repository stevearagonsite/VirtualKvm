//! (VirtualKvm) Mission Control gesture while the cursor is on a peer.
//!
//! The three-finger vertical swipe (Mission Control, Ctrl+Up on the Mac) reaches
//! the event tap as a private "dock control" event. While the cursor is on a
//! peer it must not act on the Mac: the peer gets Win+Tab (Task View) instead,
//! once per swipe, so swiping up opens it and swiping down closes it. The
//! horizontal swipe (switch desktops, Ctrl+Left/Right on the Mac) becomes
//! Win+Ctrl+Left/Right, which switches virtual desktops on Windows. With the
//! cursor on the Mac nothing is touched.
//!
//! Blocking only the dock control events is not enough: the Dock also reads
//! the raw gesture events and still opened Mission Control (or switched
//! desktops sideways). So while the cursor is on a peer every trackpad gesture
//! (both types) is kept from the Mac.
//!
//! `core-graphics`' `CGEventType` has no variant for these private types (one
//! arriving through it would be an invalid enum value), so this tap is created
//! through the raw API.

use super::{CGEventTapEnable, CaptureEvent, InputCaptureState, Position};
use core_foundation::{
    base::TCFType,
    mach_port::{CFMachPort, CFMachPortRef},
    runloop::{CFRunLoop, kCFRunLoopCommonModes},
};
use input_event::{Event, KeyboardEvent, scancode::Linux};
use libc::c_void;
use std::{cell::Cell, sync::Arc};
use tokio::sync::{Mutex, mpsc::Sender};

type CGEventRef = *mut c_void;
type TapCallback = extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;

extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
}

const SESSION_EVENT_TAP: u32 = 1;
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const DEFAULT_TAP: u32 = 0;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;

/// private event type of the raw trackpad gestures (`NSEventTypeGesture`)
const GESTURE: u32 = 29;
/// private event type of the Dock gestures (`kCGSEventDockControl`)
const DOCK_CONTROL: u32 = 30;
/// private fields of the gesture events, as macOS 26 sends them
const FIELD_HID_TYPE: u32 = 110;
const FIELD_SWIPE_MOTION: u32 = 123;
const FIELD_SWIPE_OFFSET: u32 = 124;
const FIELD_PHASE: u32 = 132;
/// `kIOHIDEventTypeDockSwipe`
const HID_DOCK_SWIPE: i64 = 23;
const MOTION_HORIZONTAL: i64 = 1;
const MOTION_VERTICAL: i64 = 2;
/// a horizontal swipe shorter than this (fraction of the screen) is not a
/// desktop switch
const MIN_HORIZONTAL_OFFSET: f64 = 0.05;
const PHASE_BEGAN: i64 = 1;
const PHASE_ENDED: i64 = 4;

struct Context {
    state: Arc<Mutex<InputCaptureState>>,
    event_tx: Sender<(Position, CaptureEvent)>,
    port: Cell<*mut c_void>,
    /// whether the dock swipe in progress began with the cursor on a peer: the
    /// whole swipe goes to the same side, even if the cursor crosses midway
    on_peer: Cell<bool>,
}

/// a shortcut: the keys pressed in order, then released in reverse
fn shortcut(keys: &[Linux]) -> Vec<CaptureEvent> {
    let key = |key: Linux, state| {
        CaptureEvent::Input(Event::Keyboard(KeyboardEvent::Key {
            time: 0,
            key: key as u32,
            state,
        }))
    };
    let press = keys.iter().map(|&k| key(k, 1));
    let release = keys.iter().rev().map(|&k| key(k, 0));
    press.chain(release).collect()
}

extern "C" {
    fn CGEventGetDoubleValueField(event: CGEventRef, field: u32) -> f64;
}

/// what a finished dock swipe does on the peer
fn swipe_shortcut(motion: i64, offset: f64) -> Option<Vec<CaptureEvent>> {
    use Linux::*;
    match motion {
        // Mission Control -> Task View
        MOTION_VERTICAL => Some(shortcut(&[KeyLeftMeta, KeyTab])),
        // fingers to the left (negative offset) go to the desktop on the right
        MOTION_HORIZONTAL if offset <= -MIN_HORIZONTAL_OFFSET => {
            Some(shortcut(&[KeyLeftMeta, KeyLeftCtrl, KeyRight]))
        }
        MOTION_HORIZONTAL if offset >= MIN_HORIZONTAL_OFFSET => {
            Some(shortcut(&[KeyLeftMeta, KeyLeftCtrl, KeyLeft]))
        }
        _ => None,
    }
}

extern "C" fn callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    // SAFETY: user_info is the `Context` box owned by the `GestureTap`, which
    // outlives the run loop this callback runs on
    let ctx = unsafe { &*(user_info as *const Context) };
    if event_type == TAP_DISABLED_BY_TIMEOUT {
        log::warn!("gesture tap disabled by timeout — re-enabling");
        unsafe { CGEventTapEnable(ctx.port.get(), true) };
        return event;
    }
    if event_type != GESTURE && event_type != DOCK_CONTROL {
        return event;
    }
    let current_pos = ctx.state.blocking_lock().current_pos;
    let field = |f| unsafe { CGEventGetIntegerValueField(event, f) };
    let on_peer = if event_type == DOCK_CONTROL && field(FIELD_HID_TYPE) == HID_DOCK_SWIPE {
        let phase = field(FIELD_PHASE);
        if phase == PHASE_BEGAN {
            ctx.on_peer.set(current_pos.is_some());
            log::debug!(
                "dock swipe begins, cursor on the peer: {}",
                ctx.on_peer.get()
            );
        }
        if let (PHASE_ENDED, true, Some(pos)) = (phase, ctx.on_peer.get(), current_pos) {
            let motion = field(FIELD_SWIPE_MOTION);
            let offset = unsafe { CGEventGetDoubleValueField(event, FIELD_SWIPE_OFFSET) };
            let events = swipe_shortcut(motion, offset);
            log::info!(
                "dock swipe on the peer (motion {motion}, offset {offset:.2}): {}",
                if events.is_some() {
                    "sending its shortcut"
                } else {
                    "ignored"
                }
            );
            for e in events.into_iter().flatten() {
                let _ = ctx.event_tx.blocking_send((pos, e));
            }
        }
        ctx.on_peer.get()
    } else {
        current_pos.is_some()
    };
    // with the cursor on the peer, no trackpad gesture acts on the Mac
    if on_peer { std::ptr::null_mut() } else { event }
}

/// the gesture tap, installed on the current thread's run loop
pub(super) struct GestureTap {
    port: CFMachPort,
    ctx: *mut Context,
}

impl GestureTap {
    pub(super) fn new(
        state: Arc<Mutex<InputCaptureState>>,
        event_tx: Sender<(Position, CaptureEvent)>,
    ) -> Option<Self> {
        let ctx = Box::into_raw(Box::new(Context {
            state,
            event_tx,
            port: Cell::new(std::ptr::null_mut()),
            on_peer: Cell::new(false),
        }));
        let port = unsafe {
            CGEventTapCreate(
                SESSION_EVENT_TAP,
                HEAD_INSERT_EVENT_TAP,
                DEFAULT_TAP,
                (1 << GESTURE) | (1 << DOCK_CONTROL),
                callback,
                ctx as *mut c_void,
            )
        };
        if port.is_null() {
            drop(unsafe { Box::from_raw(ctx) });
            return None;
        }
        let port = unsafe { CFMachPort::wrap_under_create_rule(port) };
        unsafe { (*ctx).port.set(port.as_concrete_TypeRef() as *mut c_void) };
        let tap = Self { port, ctx };
        let source = tap.port.create_runloop_source(0).ok()?;
        unsafe { CFRunLoop::get_current().add_source(&source, kCFRunLoopCommonModes) };
        Some(tap)
    }
}

impl Drop for GestureTap {
    fn drop(&mut self) {
        unsafe {
            CGEventTapEnable(self.port.as_concrete_TypeRef() as *mut c_void, false);
            core_foundation_sys::mach_port::CFMachPortInvalidate(self.port.as_concrete_TypeRef());
            drop(Box::from_raw(self.ctx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(events: Vec<CaptureEvent>) -> Vec<(u32, u8)> {
        events
            .into_iter()
            .map(|e| match e {
                CaptureEvent::Input(Event::Keyboard(KeyboardEvent::Key { key, state, .. })) => {
                    (key, state)
                }
                e => panic!("unexpected {e}"),
            })
            .collect()
    }

    #[test]
    fn vertical_swipe_is_task_view() {
        let (meta, tab) = (Linux::KeyLeftMeta as u32, Linux::KeyTab as u32);
        let events = swipe_shortcut(MOTION_VERTICAL, -0.4).unwrap();
        assert_eq!(keys(events), [(meta, 1), (tab, 1), (tab, 0), (meta, 0)]);
    }

    #[test]
    fn horizontal_swipe_switches_desktop() {
        let right = keys(swipe_shortcut(MOTION_HORIZONTAL, -0.5).unwrap());
        assert_eq!(right[2], (Linux::KeyRight as u32, 1));
        let left = keys(swipe_shortcut(MOTION_HORIZONTAL, 0.5).unwrap());
        assert_eq!(left[2], (Linux::KeyLeft as u32, 1));
        // a short swipe that went back is not a switch
        assert!(swipe_shortcut(MOTION_HORIZONTAL, 0.01).is_none());
    }
}
