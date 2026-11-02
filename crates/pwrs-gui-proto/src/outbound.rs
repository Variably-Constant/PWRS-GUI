//! Sending to rings from a thread that must never wait: a renderer's.
//!
//! A send goes straight through the ring's sender while the ring has room.
//! When it is full, the sender moves to a flusher thread with everything
//! behind it queued, and moves back once the queue is empty, so the
//! sending thread never waits and no message overtakes another.

use crate::FromHelper;
use crate::transport::{self, Ring, Role, Sender, TransportError};
use crossbeam_queue::SegQueue;
use std::collections::HashMap;
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering, fence};

/// A slot a value can be put in and taken from, any number of times, by
/// any thread, with no lock.
struct Slot<T>(AtomicPtr<T>);

impl<T> Slot<T> {
    fn empty() -> Slot<T> {
        Slot(AtomicPtr::new(null_mut()))
    }

    fn put(&self, value: T) {
        let old = self.0.swap(Box::into_raw(Box::new(value)), Ordering::AcqRel);
        if !old.is_null() {
            // SAFETY: a pointer in the slot came from Box::into_raw here.
            drop(unsafe { Box::from_raw(old) });
            eprintln!("PWRS-GUI: a sender was put back into a slot that already had one");
        }
    }

    fn take(&self) -> Option<T> {
        let taken = self.0.swap(null_mut(), Ordering::AcqRel);
        if taken.is_null() {
            None
        } else {
            // SAFETY: a pointer in the slot came from Box::into_raw in `put`,
            // and the swap gave it to this caller alone.
            Some(*unsafe { Box::from_raw(taken) })
        }
    }
}

impl<T> Drop for Slot<T> {
    fn drop(&mut self) {
        drop(self.take());
    }
}

// SAFETY: a value in the slot moves to exactly one taker.
unsafe impl<T: Send> Send for Slot<T> {}
unsafe impl<T: Send> Sync for Slot<T> {}

/// One ring a renderer sends to.
pub struct Outbound {
    ring: Arc<Ring>,
    local: Option<Sender>,
    queue: Arc<SegQueue<Vec<u8>>>,
    returned: Arc<Slot<Sender>>,
}

impl Outbound {
    /// Opens the ring `name` another handle created.
    pub fn open(name: &str, role: Role) -> Result<Outbound, TransportError> {
        let ring = Ring::open(name, role)?;
        let local = ring.sender()?;
        Ok(Outbound { ring, local: Some(local), queue: Arc::new(SegQueue::new()), returned: Arc::new(Slot::empty()) })
    }

    pub fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    /// Sends `msg` without waiting. A message the ring has no room for
    /// goes to a flusher, which reports on standard error if the ring
    /// stops taking messages before its queue is empty.
    pub fn send(&mut self, msg: &FromHelper) -> Result<(), TransportError> {
        let bytes = transport::encode(&self.ring, msg)?;
        if self.local.is_none() {
            self.local = self.returned.take();
        }
        match self.local.take() {
            Some(sender) if self.queue.is_empty() => match sender.try_send_encoded(&bytes) {
                Ok(true) => {
                    self.local = Some(sender);
                    Ok(())
                }
                Ok(false) => {
                    self.queue.push(bytes);
                    self.flush(sender);
                    Ok(())
                }
                Err(e) => {
                    self.local = Some(sender);
                    Err(e)
                }
            },
            Some(sender) => {
                self.queue.push(bytes);
                self.flush(sender);
                Ok(())
            }
            None => {
                self.queue.push(bytes);
                fence(Ordering::SeqCst);
                if let Some(sender) = self.returned.take() {
                    self.flush(sender);
                }
                Ok(())
            }
        }
    }

    /// Starts a flusher with `sender`, which sends the queue in order,
    /// waiting while the ring is full, and puts the sender back when the
    /// queue is empty.
    fn flush(&self, sender: Sender) {
        let queue = Arc::clone(&self.queue);
        let returned = Arc::clone(&self.returned);
        let name = self.ring.name().to_string();
        let spawned = std::thread::Builder::new().name(String::from("pwrs-gui-flush")).spawn(move || {
            let mut sender = sender;
            loop {
                while let Some(bytes) = queue.pop() {
                    if let Err(e) = sender.send_encoded(&bytes) {
                        eprintln!("PWRS-GUI: {name} stopped taking messages, so {} queued were dropped: {e}", queue.len() + 1);
                        return;
                    }
                }
                returned.put(sender);
                fence(Ordering::SeqCst);
                if queue.is_empty() {
                    return;
                }
                match returned.take() {
                    Some(s) => sender = s,
                    None => return,
                }
            }
        });
        if let Err(e) = spawned {
            eprintln!("PWRS-GUI: no thread could flush {}: {e}; its queued messages were dropped", self.ring.name());
        }
    }
}

/// The rings a renderer sends to, opened by name on first use and let go
/// when the session says they are gone.
#[derive(Default)]
pub struct RingOutlets {
    outlets: HashMap<String, Outbound>,
}

impl RingOutlets {
    pub fn new() -> RingOutlets {
        RingOutlets { outlets: HashMap::new() }
    }

    /// Sends `msg` to the ring `name`, opening it on first use, without
    /// waiting.
    pub fn send(&mut self, name: &str, role: Role, msg: &FromHelper) -> Result<(), TransportError> {
        if let Some(o) = self.outlets.get_mut(name) {
            return o.send(msg);
        }
        let mut o = Outbound::open(name, role)?;
        let sent = o.send(msg);
        self.outlets.insert(name.to_string(), o);
        sent
    }

    /// Lets go of the ring `name`, closing this handle on it.
    pub fn forget(&mut self, name: &str) {
        if let Some(o) = self.outlets.remove(name) {
            o.ring.close();
        }
    }
}
