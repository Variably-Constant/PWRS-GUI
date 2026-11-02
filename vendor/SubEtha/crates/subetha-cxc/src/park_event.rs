//! The kernel park of a Windows waiter on a
//! [`CrossProcessWaker`](crate::cross_process_waker::CrossProcessWaker):
//! every waiter on a file or shared-memory backing, and a timed waiter on
//! an anonymous one.
//!
//! `WaitOnAddress` is never woken from another process, so past its
//! monitor budget a waiter on a file or shared-memory backing sleeps on a
//! named auto-reset event, one per waker instance and slot, with the
//! event's id in the slot's `park_event` word. `WaitOnAddress` also ends a
//! timeout on the system timer's tick rather than at its deadline, so a
//! timed waiter on an anonymous backing sleeps the same way on an unnamed
//! event. A waker that wins the slot's `PARKED` to `WOKEN` exchange loads
//! the word and sets that event. The waiter's store and re-check and the
//! waker's exchange and load are all SeqCst, so either the waiter sees the
//! slot woken before it sleeps or the waker sees the id. Names follow
//! `cross_process_notifier`: `Local\subetha_park_<id>` beside a file, the
//! region's object directory and descriptor beside a shared-memory region,
//! where the id is the process id above a process-wide counter. A timed
//! sleep waits on the event and the thread's high-resolution waitable
//! timer together, so it ends at its deadline rather than at a tick. A
//! sleep on a named event re-checks the slot at least every
//! [`PARK_HEAL`], which bounds a wake that sets no event.

use std::io;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    CreateWaitableTimerExW, OpenEventW, SetEvent, SetWaitableTimer, WaitForMultipleObjects,
    WaitForSingleObject, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CREATE_WAITABLE_TIMER_MANUAL_RESET,
    EVENT_MODIFY_STATE, INFINITE, SYNCHRONIZATION_SYNCHRONIZE, TIMER_MODIFY_STATE,
};

use crate::cross_process_waker::WakerError;

/// The longest a parked waiter sleeps before it checks its slot again.
pub(crate) const PARK_HEAL: Duration = Duration::from_millis(20);

/// The low half of the next event id this process makes.
static NEXT_SEQ: AtomicU32 = AtomicU32::new(1);

/// An open event handle and the id its name carries, closed on drop.
struct Event {
    handle: HANDLE,
    id: u64,
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the handle is open and owned by this value.
        unsafe { CloseHandle(self.handle) };
    }
}

/// One waker instance's park events, per slot: the event its own waiters
/// sleep on, and the first event of a peer its wakers opened.
pub(crate) struct ParkEvents {
    /// The object directory the event names are made in; `None` beside an
    /// anonymous waker, whose events have no name.
    prefix: Option<String>,
    sddl: Option<String>,
    /// Set once per slot and freed only on drop, so a reference taken from
    /// one stays valid for the instance's life.
    own: Box<[AtomicPtr<Event>]>,
    /// Set once per slot, like `own`. A wake for any other id opens, sets
    /// and closes, since replacing a handle another thread may be setting
    /// would take a lock.
    peer: Box<[AtomicPtr<Event>]>,
    /// Per slot, whether a failure to set that slot's event was reported.
    set_reported: Box<[AtomicBool]>,
    /// Whether a failure to create an own event was reported.
    create_reported: AtomicBool,
    #[cfg(test)]
    heal_off: AtomicBool,
}

impl ParkEvents {
    /// The events beside a file-backed waker: `Local` names and the
    /// creator's default descriptor.
    pub(crate) fn file(capacity: usize) -> Self {
        Self::new(Some("Local".to_owned()), None, capacity)
    }

    /// The events beside a waker on a shared-memory region, in the
    /// object directory the region's name was resolved in and with its
    /// descriptor.
    pub(crate) fn shm(directory: &str, sddl: Option<&str>, capacity: usize) -> Self {
        Self::new(Some(directory.to_owned()), sddl.map(str::to_owned), capacity)
    }

    /// The events beside an anonymous waker: unnamed, so only this
    /// instance sets them.
    pub(crate) fn anon(capacity: usize) -> Self {
        Self::new(None, None, capacity)
    }

    fn new(prefix: Option<String>, sddl: Option<String>, capacity: usize) -> Self {
        Self {
            prefix,
            sddl,
            own: (0..capacity).map(|_| AtomicPtr::new(null_mut())).collect(),
            peer: (0..capacity).map(|_| AtomicPtr::new(null_mut())).collect(),
            set_reported: (0..capacity).map(|_| AtomicBool::new(false)).collect(),
            create_reported: AtomicBool::new(false),
            #[cfg(test)]
            heal_off: AtomicBool::new(false),
        }
    }

    /// The name of event `id`; `None` for an unnamed one.
    fn name(&self, id: u64) -> Option<String> {
        self.prefix.as_ref().map(|prefix| format!("{prefix}\\subetha_park_{id:016x}"))
    }

    /// How a report names event `id`: its name, or its id where it has none.
    fn label(&self, id: u64) -> String {
        self.name(id).unwrap_or_else(|| format!("{id:016x}"))
    }

    /// Sleep on slot `idx`'s event until `state` leaves `parked` or
    /// `deadline` passes, with the event's id in `word` meanwhile. With a
    /// deadline each sleep runs beside this thread's high-resolution timer.
    ///
    /// `None` when the event cannot be created, which is reported once per
    /// instance and leaves the wait to the caller. A failed sleep is an
    /// `IoError`.
    pub(crate) fn park(
        &self,
        idx: usize,
        state: &AtomicU32,
        parked: u32,
        word: &AtomicU64,
        deadline: Option<Instant>,
    ) -> Option<Result<(), WakerError>> {
        let event = match self.own_event(idx) {
            Ok(event) => event,
            Err((name, e)) => {
                if !self.create_reported.swap(true, Ordering::Relaxed) {
                    let fallback = if self.prefix.is_some() {
                        "cross-process waits on this waker fall back to the monitor and their timeouts"
                    } else {
                        "timed waits on this waker end on the system timer's tick"
                    };
                    eprintln!("subetha: park event {name} not created, so {fallback}: {e}");
                }
                return None;
            }
        };
        let heal = self.heal();
        word.store(event.id, Ordering::SeqCst);
        let result = loop {
            if state.load(Ordering::SeqCst) != parked {
                break Ok(());
            }
            let sleep = match deadline {
                None => heal,
                Some(deadline) => match deadline.checked_duration_since(Instant::now()) {
                    Some(rest) if !rest.is_zero() => Some(heal.map_or(rest, |h| rest.min(h))),
                    // Deadline reached; one last check catches a wake that
                    // landed at the wire.
                    _ => {
                        break if state.load(Ordering::SeqCst) != parked {
                            Ok(())
                        } else {
                            Err(WakerError::Timeout)
                        };
                    }
                },
            };
            let woke = match (sleep, deadline) {
                (Some(sleep), Some(_)) => timed_sleep(event, sleep),
                // SAFETY: the handle is this instance's and stays open until
                // it drops, which cannot happen while `self` is borrowed here.
                _ => unsafe { WaitForSingleObject(event.handle, sleep.map_or(INFINITE, whole_ms)) },
            };
            match woke {
                WAIT_OBJECT_0 | WAIT_TIMEOUT => {}
                _ => break Err(WakerError::IoError(io::Error::last_os_error())),
            }
        };
        word.store(0, Ordering::SeqCst);
        Some(result)
    }

    /// Set the event whose id slot `idx` held when this thread woke it:
    /// this instance's own event when the id is its, else a peer's, opened
    /// by name.
    pub(crate) fn signal(&self, idx: usize, id: u64) {
        // SAFETY: a published event is freed only when `self` drops.
        if let Some(event) = unsafe { self.own[idx].load(Ordering::Acquire).as_ref() }
            && event.id == id
        {
            self.set(idx, event);
            return;
        }
        let cell = &self.peer[idx];
        let cached = cell.load(Ordering::Acquire);
        // SAFETY: a published event is freed only when `self` drops.
        if let Some(event) = unsafe { cached.as_ref() }
            && event.id == id
        {
            self.set(idx, event);
            return;
        }
        let Some(name) = self.name(id) else {
            // An unnamed event is only ever its own instance's.
            self.report_set(idx, &self.label(id), &io::Error::from(io::ErrorKind::NotFound));
            return;
        };
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: wide is a NUL-terminated name.
        let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, wide.as_ptr()) };
        if handle.is_null() {
            let e = io::Error::last_os_error();
            // Not found is a waiter that has gone, and nothing is owed to it.
            if e.kind() != io::ErrorKind::NotFound {
                self.report_set(idx, &name, &e);
            }
            return;
        }
        let opened = Event { handle, id };
        self.set(idx, &opened);
        if cached.is_null() {
            let fresh = Box::into_raw(Box::new(opened));
            if cell
                .compare_exchange(null_mut(), fresh, Ordering::AcqRel, Ordering::Relaxed)
                .is_err()
            {
                // SAFETY: fresh came from Box::into_raw and was never published.
                drop(unsafe { Box::from_raw(fresh) });
            }
        }
    }

    /// Slot `idx`'s own event, created on first use. The error carries the
    /// label of the event the create was refused for.
    fn own_event(&self, idx: usize) -> Result<&Event, (String, io::Error)> {
        let cell = &self.own[idx];
        let current = cell.load(Ordering::Acquire);
        // SAFETY: a published event is freed only when `self` drops.
        if let Some(event) = unsafe { current.as_ref() } {
            return Ok(event);
        }
        let id = (u64::from(std::process::id()) << 32)
            | u64::from(NEXT_SEQ.fetch_add(1, Ordering::Relaxed));
        let handle = match create(self.name(id).as_deref(), self.sddl.as_deref()) {
            Ok(handle) => handle,
            Err(e) => return Err((self.label(id), e)),
        };
        let fresh = Box::into_raw(Box::new(Event { handle, id }));
        match cell.compare_exchange(null_mut(), fresh, Ordering::AcqRel, Ordering::Acquire) {
            // SAFETY: fresh is now the published event, freed only on drop.
            Ok(_) => Ok(unsafe { &*fresh }),
            Err(existing) => {
                // SAFETY: fresh came from Box::into_raw and was never
                // published; existing is published and outlives the borrow.
                drop(unsafe { Box::from_raw(fresh) });
                Ok(unsafe { &*existing })
            }
        }
    }

    fn set(&self, idx: usize, event: &Event) {
        // SAFETY: the handle is open and carries EVENT_MODIFY_STATE.
        if unsafe { SetEvent(event.handle) } == 0 {
            let e = io::Error::last_os_error();
            self.report_set(idx, &self.label(event.id), &e);
        }
    }

    fn report_set(&self, idx: usize, name: &str, e: &io::Error) {
        if !self.set_reported[idx].swap(true, Ordering::Relaxed) {
            eprintln!(
                "subetha: park event {name} not set, so its waiter wakes at its next check of the slot: {e}"
            );
        }
    }

    /// The longest one sleep lasts: [`PARK_HEAL`] on a named event, and no
    /// bound on an unnamed one, whose wakers are this process's own threads,
    /// or in a test that has turned the heal off so a lost wake shows as a
    /// timeout.
    fn heal(&self) -> Option<Duration> {
        #[cfg(test)]
        if self.heal_off.load(Ordering::Relaxed) {
            return None;
        }
        self.prefix.as_ref().map(|_| PARK_HEAL)
    }

    #[cfg(test)]
    pub(crate) fn turn_heal_off(&self) {
        self.heal_off.store(true, Ordering::Relaxed);
    }

    /// The id of slot `idx`'s own event, once a park has made it.
    #[cfg(test)]
    pub(crate) fn own_id(&self, idx: usize) -> Option<u64> {
        // SAFETY: a published event is freed only when `self` drops.
        unsafe { self.own[idx].load(Ordering::Acquire).as_ref() }.map(|event| event.id)
    }

    /// Whether slot `idx`'s own event holds a set, consuming it.
    #[cfg(test)]
    pub(crate) fn take_set(&self, idx: usize) -> bool {
        // SAFETY: a published event is freed only when `self` drops.
        let event = unsafe { self.own[idx].load(Ordering::Acquire).as_ref() }
            .expect("a park has made this slot's event");
        // SAFETY: the handle is open for the instance's life.
        unsafe { WaitForSingleObject(event.handle, 0) == WAIT_OBJECT_0 }
    }
}

impl Drop for ParkEvents {
    fn drop(&mut self) {
        for cell in self.own.iter_mut().chain(self.peer.iter_mut()) {
            let ptr = *cell.get_mut();
            if !ptr.is_null() {
                // SAFETY: every published pointer came from Box::into_raw and
                // is freed here, once.
                drop(unsafe { Box::from_raw(ptr) });
            }
        }
    }
}

/// An auto-reset event, named when `name` is given: one set releases one
/// wait and then clears, so a set is one wake, and a late one costs a
/// single extra check.
fn create(name: Option<&str>, sddl: Option<&str>) -> io::Result<HANDLE> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Threading::CreateEventW;

    let mut sd = null_mut();
    if let Some(s) = sddl {
        let wide_sddl: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
        // SAFETY: wide_sddl is NUL-terminated; sd receives a descriptor
        // LocalFree releases below.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(wide_sddl.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut())
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    let sa_ptr: *const SECURITY_ATTRIBUTES = if sddl.is_some() { &sa } else { std::ptr::null() };
    let wide: Option<Vec<u16>> = name.map(|name| name.encode_utf16().chain(Some(0)).collect());
    let name_ptr = wide.as_ref().map_or(std::ptr::null(), |wide| wide.as_ptr());
    // SAFETY: the attributes and the name, when given, are valid for the call.
    let handle = unsafe { CreateEventW(sa_ptr, 0, 0, name_ptr) };
    let err = io::Error::last_os_error();
    if !sd.is_null() {
        // SAFETY: sd was allocated by the conversion above.
        unsafe { LocalFree(sd as _) };
    }
    if handle.is_null() {
        return Err(err);
    }
    Ok(handle)
}

/// `d` in whole milliseconds, rounded up so a remainder under a millisecond
/// sleeps rather than spins, and kept below `INFINITE`.
pub(crate) fn whole_ms(d: Duration) -> u32 {
    d.as_nanos().div_ceil(1_000_000).min(u128::from(INFINITE - 1)) as u32
}

/// A waitable timer, closed on drop.
struct Timer(HANDLE);

impl Drop for Timer {
    fn drop(&mut self) {
        // SAFETY: the handle is open and owned by this value.
        unsafe { CloseHandle(self.0) };
    }
}

thread_local! {
    /// This thread's timer for timed sleeps, made on its first one.
    static TIMER: Option<Timer> = high_resolution_timer();
}

/// Whether a thread without a high-resolution timer was reported.
static TIMER_REPORTED: AtomicBool = AtomicBool::new(false);

/// Report, once per process, that timed sleeps run without a
/// high-resolution timer, and why.
fn report_no_timer(why: &dyn std::fmt::Display) {
    if !TIMER_REPORTED.swap(true, Ordering::Relaxed) {
        eprintln!("subetha: no high-resolution timer, so timed waits end on the system timer's tick: {why}");
    }
}

/// A manual-reset high-resolution timer. Every timed sleep sets it, which
/// also clears a signal an earlier sleep left on it. `None` where the
/// system refuses one (before Windows 10 1803).
fn high_resolution_timer() -> Option<Timer> {
    // SAFETY: no attributes and no name; a handle made here is owned by the
    // value returned.
    let handle = unsafe {
        CreateWaitableTimerExW(
            std::ptr::null(),
            std::ptr::null(),
            CREATE_WAITABLE_TIMER_MANUAL_RESET | CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
            TIMER_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
        )
    };
    if handle.is_null() {
        report_no_timer(&io::Error::last_os_error());
        return None;
    }
    Some(Timer(handle))
}

/// Sleep on `event` until it is set or `sleep` passes: `WAIT_OBJECT_0` for a
/// set, `WAIT_TIMEOUT` for the sleep passing, and `WAIT_FAILED`, with the
/// error left as the thread's last, for a failure. The sleep runs on this
/// thread's high-resolution timer, or where it has none on the event's own
/// timeout in whole milliseconds.
fn timed_sleep(event: &Event, sleep: Duration) -> u32 {
    let timer = match TIMER.try_with(|timer| timer.as_ref().map(|timer| timer.0)) {
        Ok(timer) => timer,
        Err(gone) => {
            report_no_timer(&gone);
            None
        }
    };
    #[cfg(test)]
    let timer = timer.filter(|_| !TEST_TIMER_OFF.get());
    let Some(timer) = timer else {
        // SAFETY: the event's handle is open while it is borrowed.
        return unsafe { WaitForSingleObject(event.handle, whole_ms(sleep)) };
    };
    // Relative, in hundreds of nanoseconds, rounded up and held within an i64.
    let due = -(sleep.as_nanos().div_ceil(100).min(i64::MAX as u128) as i64);
    // SAFETY: the timer is this thread's and open for the thread's life.
    if unsafe { SetWaitableTimer(timer, &due, 0, None, std::ptr::null(), 0) } == 0 {
        return WAIT_FAILED;
    }
    let handles = [event.handle, timer];
    // SAFETY: both handles are open for the call.
    match unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) } {
        ended if ended == WAIT_OBJECT_0 + 1 => {
            #[cfg(test)]
            TEST_TIMER_ENDS.set(TEST_TIMER_ENDS.get() + 1);
            WAIT_TIMEOUT
        }
        woke => woke,
    }
}

#[cfg(test)]
thread_local! {
    /// Whether this thread's timed sleeps run as on a system without a
    /// high-resolution timer.
    static TEST_TIMER_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// How many of this thread's timed sleeps its timer ended.
    static TEST_TIMER_ENDS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Run `f` with this thread's timed sleeps on the event's own timeout, as on
/// a system without a high-resolution timer.
#[cfg(test)]
pub(crate) fn without_timer<R>(f: impl FnOnce() -> R) -> R {
    TEST_TIMER_OFF.set(true);
    let result = f();
    TEST_TIMER_OFF.set(false);
    result
}

/// How many of this thread's timed sleeps its timer has ended.
#[cfg(test)]
pub(crate) fn timer_ends() -> u32 {
    TEST_TIMER_ENDS.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARKED: u32 = 2;
    const WOKEN: u32 = 3;

    fn events(prefix: &str, sddl: Option<&str>) -> ParkEvents {
        ParkEvents::new(Some(prefix.to_owned()), sddl.map(str::to_owned), 2)
    }

    /// Slot 0's own event of `park`, made by a park on a slot that is not
    /// parked.
    fn made_event(park: &ParkEvents) -> &Event {
        let state = AtomicU32::new(WOKEN);
        let word = AtomicU64::new(0);
        assert!(matches!(park.park(0, &state, PARKED, &word, None), Some(Ok(()))));
        // SAFETY: a published event is freed only when `park` drops.
        unsafe { &*park.own[0].load(Ordering::Acquire) }
    }

    #[test]
    fn whole_ms_rounds_up_and_stays_below_infinite() {
        assert_eq!(whole_ms(Duration::ZERO), 0);
        assert_eq!(whole_ms(Duration::from_nanos(1)), 1);
        assert_eq!(whole_ms(Duration::from_millis(20)), 20);
        assert_eq!(whole_ms(Duration::from_micros(20_001)), 21);
        assert_eq!(whole_ms(Duration::MAX), INFINITE - 1);
    }

    #[test]
    fn a_slot_no_longer_parked_returns_at_once_and_clears_its_word() {
        let park = events("Local", None);
        let state = AtomicU32::new(WOKEN);
        let word = AtomicU64::new(0);
        assert!(matches!(park.park(0, &state, PARKED, &word, None), Some(Ok(()))));
        assert_eq!(word.load(Ordering::SeqCst), 0);
        assert!(park.own_id(0).is_some(), "the event is made on the first park and kept");
        assert!(park.own_id(1).is_none());
    }

    /// Only the `Local` and `Global` prefixes name a namespace, so any other
    /// is refused at the create. A deadline already passed turns a create
    /// that unexpectedly succeeds into a timeout rather than a hang.
    #[test]
    fn an_event_that_cannot_be_created_leaves_the_wait_to_the_caller() {
        let park = events("NoSuchNamespace", None);
        let state = AtomicU32::new(PARKED);
        let word = AtomicU64::new(0);
        assert!(park.park(0, &state, PARKED, &word, Some(Instant::now())).is_none());
        assert_eq!(word.load(Ordering::SeqCst), 0, "a refused park publishes no id");
        assert!(park.create_reported.load(Ordering::Relaxed));
    }

    #[test]
    fn a_wake_for_a_waiter_that_has_gone_is_not_reported() {
        let park = events("Local", None);
        // The counter starts at 1, so no event of this process has a low
        // half of zero, and no other process has this process's id.
        park.signal(0, u64::from(std::process::id()) << 32);
        assert!(!park.set_reported[0].load(Ordering::Relaxed));
        assert!(park.peer[0].load(Ordering::Acquire).is_null());
    }

    /// An empty DACL admits no account, so a second instance is refused the
    /// right to set the event while the creator's own handle keeps it.
    #[test]
    fn a_waker_refused_the_right_to_set_reports_it_for_that_slot() {
        let waiter = events("Local", Some("D:"));
        let state = AtomicU32::new(WOKEN);
        let word = AtomicU64::new(0);
        assert!(matches!(waiter.park(0, &state, PARKED, &word, None), Some(Ok(()))));
        let id = waiter.own_id(0).expect("the park made the event");
        let waker = events("Local", None);
        waker.signal(0, id);
        assert!(waker.set_reported[0].load(Ordering::Relaxed), "a refused open is reported");
        assert!(!waker.set_reported[1].load(Ordering::Relaxed), "and only for its own slot");
        assert!(waker.peer[0].load(Ordering::Acquire).is_null(), "a refused event is not kept");
    }

    #[test]
    fn a_set_from_a_second_instance_is_seen_by_the_next_sleep() {
        let waiter = events("Local", None);
        let state = AtomicU32::new(WOKEN);
        let word = AtomicU64::new(0);
        assert!(matches!(waiter.park(0, &state, PARKED, &word, None), Some(Ok(()))));
        // SAFETY: as above.
        let event = unsafe { &*waiter.own[0].load(Ordering::Acquire) };
        let waker = events("Local", None);
        waker.signal(0, event.id);
        assert!(!waker.set_reported[0].load(Ordering::Relaxed));
        assert!(!waker.peer[0].load(Ordering::Acquire).is_null(), "the first opened event is kept");
        // SAFETY: the waiter's handle is open.
        assert_eq!(unsafe { WaitForSingleObject(event.handle, 0) }, WAIT_OBJECT_0, "the set is on the event");
        assert_eq!(unsafe { WaitForSingleObject(event.handle, 0) }, WAIT_TIMEOUT, "and one wait consumed it");
        waker.signal(0, event.id);
        assert_eq!(unsafe { WaitForSingleObject(event.handle, 0) }, WAIT_OBJECT_0, "the kept handle sets it again");
    }

    /// An anonymous instance's events have no name, and a wake for its own
    /// event sets it through the handle the instance holds.
    #[test]
    fn an_anonymous_instance_sets_its_own_unnamed_event() {
        let park = ParkEvents::anon(2);
        let event = made_event(&park);
        assert!(park.name(event.id).is_none());
        park.signal(0, event.id);
        assert!(!park.set_reported[0].load(Ordering::Relaxed));
        assert!(park.take_set(0), "the set is on the event");
        assert!(park.peer[0].load(Ordering::Acquire).is_null(), "no peer is opened for an own event");
    }

    /// A timed sleep on an event nobody sets ends on the thread's timer, one
    /// on a set event ends on the set, and without a timer the sleep is the
    /// event's own timeout.
    #[test]
    fn a_timed_sleep_ends_on_the_timer_or_on_a_set() {
        let park = ParkEvents::anon(1);
        let event = made_event(&park);
        let ends = timer_ends();
        assert_eq!(timed_sleep(event, Duration::from_millis(2)), WAIT_TIMEOUT);
        assert_eq!(timer_ends(), ends + 1, "the timer ended the sleep");
        park.signal(0, event.id);
        assert_eq!(timed_sleep(event, Duration::from_secs(30)), WAIT_OBJECT_0, "the set ends the sleep");
        assert_eq!(timer_ends(), ends + 1);
        assert_eq!(without_timer(|| timed_sleep(event, Duration::from_millis(2))), WAIT_TIMEOUT);
        assert_eq!(timer_ends(), ends + 1, "without a timer the event's own timeout ended the sleep");
    }

    /// A signal an earlier sleep left on the timer does not end the next
    /// sleep, since setting the timer clears it.
    #[test]
    fn a_signal_left_on_the_timer_does_not_end_the_next_sleep() {
        let park = ParkEvents::anon(1);
        let event = made_event(&park);
        park.signal(0, event.id);
        assert_eq!(timed_sleep(event, Duration::from_millis(1)), WAIT_OBJECT_0);
        // The timer comes due with nobody waiting on it and keeps its signal.
        std::thread::sleep(Duration::from_millis(10));
        let sleep = Duration::from_millis(50);
        let start = Instant::now();
        assert_eq!(timed_sleep(event, sleep), WAIT_TIMEOUT);
        let took = start.elapsed();
        assert!(took >= sleep / 2, "the signal left on the timer ended the sleep after {took:?}");
    }
}
