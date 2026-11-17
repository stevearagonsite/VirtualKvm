//! (VirtualKvm) Mission Control gesture while the cursor is on a peer.
//!
//! The three-finger vertical swipe (Mission Control, Ctrl+Up on the Mac) reaches
//! the event tap as a private "dock control" event. While the cursor is on a
//! peer it must not act on the Mac: the peer gets Win+Tab (Task View) instead,
//! once per swipe, so swiping up opens it and swiping down closes it.
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

/// private event type of the Dock gestures (`kCGSEventDockControl`)
const DOCK_CONTROL: u32 = 30;
/// private fields of the gesture events, as macOS 26 sends them
const FIELD_HID_TYPE: u32 = 110;
const FIELD_SWIPE_MOTION: u32 = 123;
const FIELD_PHASE: u32 = 132;
/// `kIOHIDEventTypeDockSwipe`
const HID_DOCK_SWIPE: i64 = 23;
const MOTION_VERTICAL: i64 = 2;
const PHASE_BEGAN: i64 = 1;
const PHASE_ENDED: i64 = 4;

struct Context {
    state: Arc<Mutex<InputCaptureState>>,
    event_tx: Sender<(Position, CaptureEvent)>,
    port: Cell<*mut c_void>,
    /// whether the swipe in progress began with the cursor on a peer: the
    /// whole swipe goes to the same side, even if the cursor crosses midway
    on_peer: Cell<bool>,
}

/// Win+Tab, pressed and released
fn win_tab() -> impl Iterator<Item = CaptureEvent> {
    let meta = Linux::KeyLeftMeta as u32;
    let tab = Linux::KeyTab as u32;
    [(meta, 1), (tab, 1), (tab, 0), (meta, 0)]
        .into_iter()
        .map(|(key, state)| {
            CaptureEvent::Input(Event::Keyboard(KeyboardEvent::Key {
                time: 0,
                key,
                state,
            }))
        })
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
    if event_type != DOCK_CONTROL {
        return event;
    }
    let field = |f| unsafe { CGEventGetIntegerValueField(event, f) };
    if field(FIELD_HID_TYPE) != HID_DOCK_SWIPE || field(FIELD_SWIPE_MOTION) != MOTION_VERTICAL {
        return event;
    }
    let current_pos = ctx.state.blocking_lock().current_pos;
    let phase = field(FIELD_PHASE);
    if phase == PHASE_BEGAN {
        ctx.on_peer.set(current_pos.is_some());
    }
    if !ctx.on_peer.get() {
        return event;
    }
    if let (PHASE_ENDED, Some(pos)) = (phase, current_pos) {
        log::info!("Mission Control swipe on the peer: sending Win+Tab");
        for e in win_tab() {
            let _ = ctx.event_tx.blocking_send((pos, e));
        }
    }
    // the Mac must not open Mission Control
    std::ptr::null_mut()
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
                1 << DOCK_CONTROL,
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
