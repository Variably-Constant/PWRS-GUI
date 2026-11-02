//! The push, pop, read and write that [`SharedVec`](crate::shared_vec::SharedVec)
//! and [`RawVec`](crate::raw_vec::RawVec) share over one file layout: a top
//! word in the header and an array of slots, each of which claims and
//! publishes its element with one compare-and-swap, so no element is ever
//! visible half placed and no push or pop waits on another participant.
//!
//! # The top word
//!
//! The header's top word is 16 bytes, read whole. Its low half holds the
//! length in its low 32 bits, the mode bit above it, set while the vec runs
//! the multi-writer protocol, and a version above that; its high half holds
//! an epoch in its low 32 bits, then a 15-bit table generation, the
//! `SANITIZING` bit, and the count of live writers. A push raises the
//! length past the slot it claimed, and a pop lowers it past the slot it
//! took, each by an 8-byte swap or add on the low half; a lowering steps the
//! version, so no low half repeats and a swap aimed at an earlier one fails.
//! A clear swaps the whole word, setting the length to zero and stepping the
//! version and the epoch, so every claim made before it reads as one the
//! clear removed.
//!
//! # Writers
//!
//! Each thread that pushes, pops or sets through a handle registers once,
//! counted in the top word's writer count by one swap of the whole word, and
//! in the handle's entry in the writer table after the slots: its
//! process's holder word beside the handle's registrations. A handle takes
//! its entry on its first registration, stepping the table generation, and
//! frees it when it drops; a thread's exit and the handle's drop give its
//! registrations back.
//!
//! The registration that brings the count to two sets the mode bit in the
//! same swap. The one writer left clears it at the start of its next push or
//! pop, so no multi-writer push is ever in flight under the one-writer
//! protocol.
//!
//! A registration is counted in the top word before the entry, and given
//! back from the entry before the top word, so the top word never counts
//! fewer writers than the entries do. A writer that finds a holder gone, by
//! its pid and start, gives back all of that entry's registrations by one
//! swap of the entry and one lowering of the count. Each writer looks at
//! the table on its handle's first registration and again on a multi-writer
//! push or pop that finds the table generation, or the count of exits this
//! process's watches have been told of, moved since it last looked; it
//! watches each holder it finds alive for its exit. A one-writer push or
//! pop does none of this: the only writer counted is itself.
//!
//! # Slots
//!
//! An element of at most 8 bytes at an alignment of at most 8 lives in a
//! word slot: a 16-byte pair of the element's word (its bytes first, zero
//! after) and a state word, alone in its cache line. The state says whether
//! the slot holds an element ([`LIVE`](crate::vec_core::LIVE), or
//! [`LIVE_S`](crate::vec_core::LIVE_S) when one writer pushed it alone), held
//! one a pop took ([`TAKEN`](crate::vec_core::TAKEN)), or holds none
//! ([`FREE`](crate::vec_core::FREE), the
//! zeroed file), with the epoch of its claim in its high half and a
//! sequence every write steps below it. A larger element lives in a pair of
//! buffers (see [`slot_pair`](crate::slot_pair)) whose head carries the same
//! state beside its buffer bit.
//!
//! # One writer and several
//!
//! With one writer, a push claims slot `len` LIVE_S and raises the length,
//! and a pop lowers the length alone, leaving the element in place at or
//! past the length, where no writer counts it. With several, a push of a
//! word reserves slot `len` by one add to the length and then claims it
//! LIVE; near the capacity, or while a flip sanitizes, or for a pair of
//! buffers, it claims slot `len` LIVE first and raises the length after. A
//! writer that finds slot `len` LIVE in the current epoch counts that claim,
//! raising the length for it, before anything else: a claim whose pusher has
//! not raised the length yet, or has stopped. A claim left from an earlier
//! epoch is one a clear removed, and a push claims over it. A pop takes
//! slot `len - 1` by swapping it to TAKEN, the state half alone in a word
//! slot, then lowers the length.
//!
//! The flip to several writers sets `SANITIZING`, which no reservation passes:
//! the writer that flipped first makes FREE each element one-writer pops
//! left past the length and TAKEN the slot at the length, as
//! [`VecSlots::sanitize`](crate::vec_core::VecSlots::sanitize) says, so no
//! reservation uncovers a popped element.
//!
//! # What a reader sees
//!
//! Every slot below the length holds an element or is a hole: a pop whose
//! lowering loses to a push leaves the slot it took below the length, which
//! [`get`](crate::vec_core::VecSlots::get) answers None for and the next pop steps past. With
//! several writers, a reserved slot below the length holds what it held
//! until its claim lands, so a reader takes it for an element only when its
//! claim is of the current epoch. A writer that finds another writer's
//! process gone makes TAKEN every slot below the length with no claim of the
//! epoch before it gives that writer's registrations back, so with one
//! writer a reader judges a slot by its kind alone. A push whose raise finds
//! the length still at or below its slot takes its claim back and pushes
//! again; a claim it cannot take back was counted, or removed by a clear,
//! and the push is done.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{fence, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use crate::atomic_pair::AtomicPair;
use crate::process_identity;
use crate::slot_pair::{self, SlotGeometry};

/// The length's bits in the top word's low half, and a state's low half.
const LEN_MASK: u64 = 0xFFFF_FFFF;
/// Where the epoch starts in a state.
const HALF: u32 = 32;
/// The top word's mode bit, in its low half above the length: set while two
/// or more writers are registered, and compared by every swap of that half.
const MULTI: u64 = 1 << HALF;
/// Where the version starts in the top word's low half, above the mode bit.
const VERSION_SHIFT: u32 = HALF + 1;
/// What a pop adds to the top word's low half: the length down one and the
/// version up one, the mode bit as it is, the length being at least 1.
const LOWERING: u64 = (1 << VERSION_SHIFT) - 1;
/// Where the table generation starts in the top word's high half, above the
/// epoch: 15 bits a handle steps when it takes a writer-table entry.
const GENERATION_SHIFT: u32 = 32;
const GENERATION_MASK: u64 = 0x7FFF << GENERATION_SHIFT;
/// The top word's high half's bit set from the moment the vec turns to the
/// multi-writer protocol until no element a one-writer pop left in place
/// remains at or past the length: no push reserves a slot while it is set.
const SANITIZING: u64 = 1 << 47;
/// Where the live writer count starts in the top word's high half, above the
/// table generation.
const WRITERS_SHIFT: u32 = 48;

/// The longest a vec may be: the length's 32 bits.
pub(crate) const MAX_CAPACITY: usize = u32::MAX as usize;
/// The longest vec whose multi-writer pushes reserve their slots by adding
/// to the length: one add from each of fewer than 65535 writers past it
/// leaves the length below the mode bit.
const RESERVE_CAPACITY: u64 = (1 << HALF) - (1 << 16);

/// A slot that holds no element: never written, or taken back by its push.
pub(crate) const FREE: u64 = 0;
/// A slot that holds an element.
pub(crate) const LIVE: u64 = 1;
/// A slot whose element a pop took.
pub(crate) const TAKEN: u64 = 2;
/// A slot that holds an element a writer pushed while it was the vec's only
/// writer. At or above the length it is dead in both modes: a one-writer pop
/// leaves it there without taking it, and no writer counts it.
pub(crate) const LIVE_S: u64 = 3;
const KIND_MASK: u64 = 3;
/// A state's sequence: bits 2 to 30, below the epoch.
const SEQ_ONE: u64 = 4;
const SEQ_MASK: u64 = 0x7FFF_FFFC;
/// A state's low half as a pair's head carries it, shifted past the
/// buffer bit.
const LOW_31: u64 = 0x7FFF_FFFF;
const HIGH: u64 = !LEN_MASK;

/// Bytes from one word slot to the next: one cache line.
pub(crate) const WORD_STRIDE: usize = 64;

/// The length a top word's low half holds.
#[inline(always)]
pub(crate) fn len_of(low: u64) -> u64 {
    low & LEN_MASK
}

/// Whether the version of the top word at `top` is still the one in the low
/// half `low`, read after a slot: no pop lowered the length and no clear
/// emptied the vec since, so no slot below that length was taken and claimed
/// again by a push the length does not count yet. A raise leaves the
/// version.
#[inline(always)]
fn version_held_at(top: &AtomicPair, low: u64) -> bool {
    fence(Ordering::Acquire);
    (top.load_lo() ^ low) >> VERSION_SHIFT == 0
}

/// The epoch the top word `top` holds.
#[inline(always)]
fn epoch_of(top: u128) -> u32 {
    (top >> 64) as u32
}

/// The top word one push on: the length raised by one, by one add to the
/// low half, the length being below the capacity.
#[inline(always)]
fn raised(top: u128) -> u128 {
    (top & !u128::from(u64::MAX)) | u128::from((top as u64).wrapping_add(1))
}

/// The top word one pop on: the length lowered by one and the version
/// stepped, both by one add of [`LOWERING`] to the low half.
#[inline(always)]
fn lowered(top: u128) -> u128 {
    (top & !u128::from(u64::MAX)) | u128::from((top as u64).wrapping_add(LOWERING))
}

/// The top word a clear leaves: length zero, the version and the epoch
/// stepped, the mode, the table generation and the writer count as they
/// were.
#[inline(always)]
fn cleared(top: u128) -> u128 {
    let low = top as u64;
    let ver = (low >> VERSION_SHIFT).wrapping_add(1);
    let high = (top >> 64) as u64;
    let kept = high & !u64::from(u32::MAX);
    (u128::from(kept | u64::from(epoch_of(top).wrapping_add(1))) << 64)
        | u128::from((low & MULTI) | (ver << VERSION_SHIFT))
}

/// The table generation the top word `top` holds.
#[inline(always)]
fn generation_of(top: u128) -> u64 {
    ((top >> 64) as u64 & GENERATION_MASK) >> GENERATION_SHIFT
}

/// `top` with its table generation stepped, wrapping in its 15 bits.
#[inline(always)]
fn generation_stepped(top: u128) -> u128 {
    let high = (top >> 64) as u64;
    let stepped = (high & !GENERATION_MASK) | (high.wrapping_add(1 << GENERATION_SHIFT) & GENERATION_MASK);
    (u128::from(stepped) << 64) | u128::from(top as u64)
}

/// What a writer compares on each multi-writer push and pop: the table
/// generation of the top word `top` above the exits of watched holders this
/// process was told of. Any other value than the one it read at its last
/// look at the writer table sends it to the table again.
#[inline(always)]
fn heard(top: u128) -> u64 {
    (generation_of(top) << 48) | (crate::exit_watch::exits() & ((1 << 48) - 1))
}

/// Whether the top word's low half `low` says two or more writers.
#[inline(always)]
fn multi(low: u64) -> bool {
    low & MULTI != 0
}

/// The live writers the top word `top` counts.
#[inline(always)]
pub(crate) fn writers_of(top: u128) -> u64 {
    (top >> (64 + WRITERS_SHIFT)) as u64
}

/// `top` counting `writers` live writers, its mode bit set once there are two
/// or more. The bit stays set when the count falls: the one writer left
/// clears it at the start of its next push or pop, when it has nothing in
/// flight (see [`VecSlots::flip_back`]).
#[inline(always)]
fn with_writers(top: u128, writers: u64) -> u128 {
    let high = ((top >> 64) as u64 & ((1 << WRITERS_SHIFT) - 1)) | (writers << WRITERS_SHIFT);
    let low = if writers >= 2 { top as u64 | MULTI } else { top as u64 };
    (u128::from(high) << 64) | u128::from(low)
}

/// Whether the top word `top` says a flip to the multi-writer protocol has
/// not finished making TAKEN the elements one-writer pops left in place.
#[inline(always)]
fn sanitizing(top: u128) -> bool {
    (top >> 64) as u64 & SANITIZING != 0
}

/// Whether `state` says the slot holds an element claimed in `epoch`,
/// pushed by one writer or by several: LIVE and LIVE_S share the low kind
/// bit, compared with the epoch in one masked comparison. A reservation
/// raises the length over its slot before claiming it, so a slot below the
/// length can hold an element a clear removed, which reads as a hole.
#[inline(always)]
fn current(state: u64, epoch: u32) -> bool {
    state & (HIGH | LIVE) == (u64::from(epoch) << HALF) | LIVE
}

/// The kind a state says, in its low two bits.
#[inline(always)]
fn kind(state: u64) -> u64 {
    state & KIND_MASK
}

/// The epoch of the claim a state records.
#[inline(always)]
fn claim_epoch(state: u64) -> u32 {
    (state >> HALF) as u32
}

/// Whether `state` says LIVE for a claim in `epoch`: its kind and its epoch
/// compared at once, with bit 31, which no state sets.
#[inline(always)]
fn live_in(state: u64, epoch: u32) -> bool {
    state & !SEQ_MASK == (u64::from(epoch) << HALF) | LIVE
}

/// Whether `state` says LIVE_S for a claim in `epoch`, compared as
/// [`live_in`] compares.
#[inline(always)]
fn live_s_in(state: u64, epoch: u32) -> bool {
    state & !SEQ_MASK == (u64::from(epoch) << HALF) | LIVE_S
}

/// `state` one write on, saying `kind` for a claim in `epoch`: its
/// sequence stepped.
#[inline(always)]
fn stepped(state: u64, kind: u64, epoch: u32) -> u64 {
    (u64::from(epoch) << HALF) | (state.wrapping_add(SEQ_ONE) & SEQ_MASK) | kind
}

/// What a push that reserved its slot by adding to the length found.
enum Reserved {
    /// The element is in the vec at this index, or was and was taken or
    /// cleared since.
    Landed(usize),
    /// The add found the vec full.
    Full,
    /// The reservation went to another writer's claim, or came back: the
    /// push goes again.
    Again,
}

/// What a push found once its raise of the top word lost.
#[derive(Debug, PartialEq, Eq)]
enum Raise {
    /// The element is in the vec at this index, or was and was taken or
    /// cleared since: the push is done.
    Done(usize),
    /// The claim came back: the push goes again.
    Again,
}

/// Numbers every vec handle this process opens, for [`Writers`].
static HANDLE_IDS: AtomicU64 = AtomicU64::new(1);

/// The low bit of a handle's first-writer word: a thread that wrote through
/// the handle has exited, and its registration waits to be given back.
const POKE: usize = 1;

thread_local! {
    /// Its address names the thread among the ones running, 8-byte aligned,
    /// so a tag's low bit is free for [`POKE`].
    static THREAD_TAG: u64 = const { 0 };
    /// The id of the last handle this thread registered with or found itself
    /// registered with, zero before any: a thread registered with a handle
    /// stays registered until the handle drops, and no id is handed out
    /// twice.
    static LAST: Cell<u64> = const { Cell::new(0) };
    /// The handles this thread registered with as a writer. Its drop, at the
    /// thread's exit, counts each of those registrations dead.
    static REGISTERED: RefCell<Registrations> = const { RefCell::new(Registrations(Vec::new())) };
}

/// One thread's writer registrations: each handle's id beside what its
/// writers share, which a dropped handle no longer holds.
struct Registrations(Vec<(u64, Weak<WriterState>)>);

impl Drop for Registrations {
    fn drop(&mut self) {
        let me = THREAD_TAG.with(|t| t as *const u64 as usize);
        for (_, state) in self.0.drain(..) {
            if let Some(state) = state.upgrade() {
                state.exited(me);
            }
        }
    }
}

/// What one vec handle's writers share with the threads that registered
/// through it, which outlives the handle while such a thread runs.
struct WriterState {
    id: u64,
    /// The first writer thread's tag, zero before any and once it exited,
    /// with [`POKE`] set while exited writers' registrations wait.
    first: AtomicUsize,
    /// Registrations made through the handle and not given back.
    held: AtomicU64,
    /// Registrations of threads that exited, not given back yet.
    dead: AtomicU64,
}

impl WriterState {
    /// The thread tagged `me`, registered through this handle, has exited:
    /// its registration is dead, and the first-writer word is poked, its
    /// tag cleared when it named that thread. The vec's top word is left to
    /// the handle's next writer, so an exit never touches a mapping the
    /// handle may be unmapping.
    fn exited(&self, me: usize) {
        self.dead.fetch_add(1, Ordering::AcqRel);
        let mut f = self.first.load(Ordering::Acquire);
        loop {
            let poked = if f & !POKE == me { POKE } else { f | POKE };
            match self.first.compare_exchange(f, poked, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return,
                Err(now) => f = now,
            }
        }
    }
}

/// A writer-table entry: in its low half the holder word (pid and start
/// stamp) of the process whose handle took it, zero while it is free, and in
/// its high half that handle's live registrations.
pub(crate) type WriterEntry = AtomicPair;

/// [`Writers::entry`] before the handle's first registration.
const NO_ENTRY_YET: usize = 0;
/// [`Writers::entry`] once the handle found every entry held.
const NO_ENTRY: usize = usize::MAX;

/// The writers one vec handle has registered with its vec: each thread that
/// pushes, pops or sets through it, counted once in the vec's top word and
/// in the handle's entry in the vec's writer table. A thread's registration
/// is given back by the handle's next writer after the thread exits, and
/// every one left when the handle drops; a process's that is gone, by the
/// next writer of any process that finds it gone.
pub(crate) struct Writers {
    state: Arc<WriterState>,
    /// The writer table: `slots` entries from here, in the handle's mapping.
    table: *const WriterEntry,
    slots: usize,
    /// The handle's entry: [`NO_ENTRY_YET`], [`NO_ENTRY`], or its index plus
    /// one.
    entry: AtomicUsize,
    /// What [`heard`] read when this handle last looked at the table.
    seen: AtomicU64,
}

// SAFETY: the table lies in the mapping the handle holds and is read and
// written only through its atomic entries.
unsafe impl Send for Writers {}
// SAFETY: as for Send.
unsafe impl Sync for Writers {}

impl Writers {
    /// The writers of a handle whose vec's writer table is `slots` entries
    /// from `table`.
    ///
    /// # Safety
    /// `table` is 16-byte aligned and addresses `slots` entries, read and
    /// written only through this protocol, in a mapping that outlives every
    /// call on the writers but their drop; and the processor swaps 16 bytes.
    pub(crate) unsafe fn new(table: *const u8, slots: usize) -> Self {
        Self {
            state: Arc::new(WriterState {
                id: HANDLE_IDS.fetch_add(1, Ordering::Relaxed),
                first: AtomicUsize::new(0),
                held: AtomicU64::new(0),
                dead: AtomicU64::new(0),
            }),
            table: table as *const WriterEntry,
            slots,
            entry: AtomicUsize::new(NO_ENTRY_YET),
            seen: AtomicU64::new(0),
        }
    }

    /// The writer table.
    fn table(&self) -> &[WriterEntry] {
        // SAFETY: `new`'s contract.
        unsafe { std::slice::from_raw_parts(self.table, self.slots) }
    }

    /// Register the calling thread as a writer of the vec of `slots` unless
    /// it is already: one comparison for the handle's first writer, and one
    /// more for a thread that last registered with this handle, while no
    /// exited writer waits.
    #[inline(always)]
    fn enter(&self, slots: &VecSlots<'_>) {
        let me = THREAD_TAG.with(|t| t as *const u64 as usize);
        let f = self.state.first.load(Ordering::Relaxed);
        if f == me || (f & POKE == 0 && LAST.with(Cell::get) == self.state.id) {
            return;
        }
        self.enter_other(slots.top, slots.data, slots.capacity, me);
    }

    /// [`enter`](Self::enter) for any other thread, or once a writer has
    /// exited, into the vec whose top word is `top` and whose `capacity` word
    /// slots lie from `data`: the exited writers' registrations given back,
    /// the first-writer word taken when it names none, and the calling thread
    /// registered. It takes what it reads as arguments, so a write keeps its
    /// slots in registers.
    #[cold]
    #[inline(never)]
    fn enter_other(&self, top: &AtomicPair, data: *const u8, capacity: u64, me: usize) {
        let state = &*self.state;
        let slots = &VecSlots { top, data, capacity, shape: Shape::Word, writers: self };
        loop {
            let f = state.first.load(Ordering::Acquire);
            if f & POKE != 0 {
                if state.first.compare_exchange(f, f & !POKE, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    for _ in 0..state.dead.swap(0, Ordering::AcqRel) {
                        state.held.fetch_sub(1, Ordering::AcqRel);
                        self.release(top);
                    }
                }
                continue;
            }
            if f == me {
                return;
            }
            if f != 0 || state.first.compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                break;
            }
        }
        let id = state.id;
        let fresh = REGISTERED.try_with(|r| {
            let mut r = r.borrow_mut();
            if r.0.iter().any(|(h, _)| *h == id) {
                return false;
            }
            r.0.retain(|(_, s)| s.strong_count() > 0);
            r.0.push((id, Arc::downgrade(&self.state)));
            true
        });
        // A thread whose registrations already dropped, writing from another
        // thread-local's destructor, is counted as a fresh writer and given
        // back when the handle drops.
        if fresh.unwrap_or(true) {
            self.register(slots);
        }
        LAST.with(|last| last.set(id));
    }

    /// Count the calling thread as a writer: in the top word first, then in
    /// the handle's entry, so a crash between the two leaves the top word
    /// counting it, which keeps the vec in the multi-writer protocol. A
    /// registration that turns the vec to that protocol then sanitizes it.
    /// The handle's first registration takes its entry, steps the table
    /// generation when it found one, and looks at the table.
    fn register(&self, slots: &VecSlots<'_>) {
        let (entry, first) = self.entry_taken();
        self.state.held.fetch_add(1, Ordering::AcqRel);
        if let Some(flipped) = add_writer_to(slots.top, first && entry.is_some()) {
            slots.sanitize(flipped);
        }
        if let Some(entry) = entry {
            add_to_count(entry, 1);
        }
        if first {
            self.scan(slots);
        }
    }

    /// Give back one registration: from the handle's entry first, then from
    /// the top word, so a crash between the two leaves the top word high.
    fn release(&self, top: &AtomicPair) {
        if let Some(entry) = self.entry_at(self.entry.load(Ordering::Acquire)) {
            add_to_count(entry, u64::MAX);
        }
        remove_writers_from(top, 1);
    }

    /// The entry a mark in [`entry`](Self::entry) names.
    fn entry_at(&self, mark: usize) -> Option<&WriterEntry> {
        (mark != NO_ENTRY_YET && mark != NO_ENTRY).then(|| &self.table()[mark - 1])
    }

    /// The handle's entry, taken on its first call: the entry, or none when
    /// every entry was held then; and whether this call decided which.
    fn entry_taken(&self) -> (Option<&WriterEntry>, bool) {
        let found = self.entry.load(Ordering::Acquire);
        if found != NO_ENTRY_YET {
            return (self.entry_at(found), false);
        }
        let me = process_identity::current();
        let took = self.table().iter().position(|e| e.load_lo() == 0 && e.compare_exchange(0, u128::from(me)).is_ok());
        let mark = took.map_or(NO_ENTRY, |i| i + 1);
        match self.entry.compare_exchange(NO_ENTRY_YET, mark, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => (self.entry_at(mark), true),
            Err(found) => {
                // Another thread decided this handle's entry first.
                if let Some(i) = took {
                    free_entry(&self.table()[i], me);
                }
                (self.entry_at(found), false)
            }
        }
    }

    /// Whether a handle took an entry or a watched holder exited since this
    /// handle last looked at the writer table, by the top word `top`.
    #[inline(always)]
    fn news(&self, top: u128) -> bool {
        heard(top) != self.seen.load(Ordering::Relaxed)
    }

    /// [`scan`](Self::scan) for the vec whose top word is `top` and whose
    /// `capacity` word slots lie from `data`: for a push or pop, which keeps
    /// its slots in registers.
    #[cold]
    #[inline(never)]
    fn scan_at(&self, top: &AtomicPair, data: *const u8, capacity: u64) {
        self.scan(&VecSlots { top, data, capacity, shape: Shape::Word, writers: self });
    }

    /// Look at every entry of the writer table of the vec of `slots`: give
    /// back the registrations of each holder that is gone, by one swap of its
    /// entry and one lowering of the top word's count, and watch every other
    /// holder for its exit. A gone holder's registrations are given back only
    /// once no element of an earlier epoch is left below the length, which a
    /// reservation it made and never claimed could have uncovered. What
    /// [`heard`] read first is recorded last, so an entry taken or an exit
    /// told meanwhile sends the next multi-writer push or pop here again.
    #[cold]
    #[inline(never)]
    pub(crate) fn scan(&self, slots: &VecSlots<'_>) {
        let top = slots.top;
        let now = heard(load_top_word(top));
        let me = process_identity::current();
        let mut voided = false;
        for entry in self.table() {
            let held = entry.load();
            let holder = lo(held);
            if holder == 0 || holder == me {
                continue;
            }
            let gone = match crate::exit_watch::told(holder) {
                // Its watch waits, so its process runs.
                Some(false) => false,
                Some(true) => process_identity::holder_gone(holder),
                // Watched before it is checked again, so an exit after the
                // check is told.
                None => {
                    process_identity::holder_gone(holder) || {
                        crate::exit_watch::watch_holder(holder);
                        process_identity::holder_gone(holder)
                    }
                }
            };
            if !gone {
                continue;
            }
            if !voided {
                slots.void_stale_below_len();
                voided = true;
            }
            if entry.compare_exchange(held, 0).is_ok() {
                remove_writers_from(top, hi(held));
            }
        }
        self.seen.store(now, Ordering::Relaxed);
    }

    /// Give back every registration this handle made, to the vec whose top
    /// word is `top`, and then its entry.
    pub(crate) fn leave(&self, top: &AtomicPair) {
        self.state.dead.store(0, Ordering::Release);
        for _ in 0..self.state.held.swap(0, Ordering::AcqRel) {
            self.release(top);
        }
        if let Some(entry) = self.entry_at(self.entry.swap(NO_ENTRY, Ordering::AcqRel)) {
            free_entry(entry, process_identity::current());
        }
    }
}

/// Add `delta`, wrapping, to the registrations a writer-table entry counts.
fn add_to_count(entry: &WriterEntry, delta: u64) {
    let mut count = entry.load_hi();
    loop {
        match entry.compare_exchange_hi(count, count.wrapping_add(delta)) {
            Ok(_) => return,
            Err(now) => count = now,
        }
    }
}

/// Free the entry `me`'s handle holds, counting no registration.
fn free_entry(entry: &WriterEntry, me: u64) {
    let freed = entry.compare_exchange(u128::from(me), 0).is_ok();
    debug_assert!(freed, "a handle's entry holds this process and no registration when it is freed");
}

/// One vec's top word and slots, as a mapping holds them, and the writers of
/// the handle reaching them.
pub(crate) struct VecSlots<'a> {
    top: &'a AtomicPair,
    data: *const u8,
    capacity: u64,
    shape: Shape,
    writers: &'a Writers,
}

/// How each slot of a vec holds its element.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Shape {
    /// An element of at most 8 bytes, in a word slot.
    Word,
    /// A larger element, in a pair of buffers laid out by the geometry.
    Pair(SlotGeometry),
}

impl Shape {
    /// The slot shape for an element of `size` bytes aligned to `alignment`,
    /// with `pair` the buffer layout a larger one takes.
    pub(crate) fn for_element(size: usize, alignment: usize, pair: SlotGeometry) -> Self {
        if size <= 8 && alignment <= 8 { Shape::Word } else { Shape::Pair(pair) }
    }

    /// Bytes from one slot to the next.
    pub(crate) fn stride(&self) -> usize {
        match self {
            Shape::Word => WORD_STRIDE,
            Shape::Pair(g) => g.stride,
        }
    }

    /// Where an element's bytes start in its slot: the word in a word slot,
    /// the first buffer in a pair.
    pub(crate) fn payload_offset(&self) -> usize {
        match self {
            Shape::Word => 0,
            Shape::Pair(g) => g.buffers[0],
        }
    }

    /// Bytes from one buffer to the next, or the word's 8.
    pub(crate) fn payload_size(&self) -> usize {
        match self {
            Shape::Word => 8,
            Shape::Pair(g) => g.buffers[1] - g.buffers[0],
        }
    }
}

/// The element of `size` bytes, at most 8, at `src` as its slot's word: its
/// bytes first, in memory order, and zero after.
///
/// # Safety
/// `src` is readable for `size` bytes, and `size` is at most 8.
#[inline(always)]
pub(crate) unsafe fn word_from(src: *const u8, size: usize) -> u64 {
    debug_assert!(size <= 8);
    // SAFETY: the caller's contract. An element of 8 or 4 bytes is one read
    // of that width, held in a register; any other size is copied into a
    // zeroed word.
    unsafe {
        if size == 8 {
            return src.cast::<u64>().read_unaligned();
        }
        if size == 4 {
            let first = u64::from(src.cast::<u32>().read_unaligned());
            return if cfg!(target_endian = "little") { first } else { first << 32 };
        }
        let mut word = [0u8; 8];
        std::ptr::copy_nonoverlapping(src, word.as_mut_ptr(), size);
        u64::from_ne_bytes(word)
    }
}

/// Copy the first `size` bytes of a slot's word, at most 8, to `dst`.
///
/// # Safety
/// `dst` is writable for `size` bytes, and `size` is at most 8.
#[inline(always)]
pub(crate) unsafe fn word_into(word: u64, dst: *mut u8, size: usize) {
    debug_assert!(size <= 8);
    // SAFETY: the caller's contract. Widths as in `word_from`.
    unsafe {
        if size == 8 {
            dst.cast::<u64>().write_unaligned(word);
        } else if size == 4 {
            let first = if cfg!(target_endian = "little") { word as u32 } else { (word >> 32) as u32 };
            dst.cast::<u32>().write_unaligned(first);
        } else {
            let b = word.to_ne_bytes();
            std::ptr::copy_nonoverlapping(b.as_ptr(), dst, size);
        }
    }
}

/// Copy an element of `size` bytes out of `buffer`, 8-byte aligned and as
/// wide as the element rounded up to 8, into `out`, a whole word at a time.
///
/// # Safety
/// `buffer` is readable for `size` rounded up to 8 bytes, and `out` is
/// writable for `size` bytes and touched by nothing else during the copy.
#[inline(always)]
unsafe fn copy_out(out: *mut u8, buffer: *const u8, size: usize) {
    let whole = size & !7;
    // SAFETY: the caller's contract; `whole` is a multiple of 8.
    unsafe { crate::atomic_copy::load_bytes(out, buffer, whole) };
    if whole < size {
        // SAFETY: the buffer is as wide as the element rounded up to 8.
        let tail = unsafe { crate::atomic_copy::load_word(buffer.add(whole) as *const u64) }.to_ne_bytes();
        // SAFETY: `out` holds `size` bytes; the tail is `size - whole` of them.
        unsafe { std::ptr::copy_nonoverlapping(tail.as_ptr(), out.add(whole), size - whole) };
    }
}

/// The state a pair's head carries: its epoch in place, its kind and
/// sequence shifted down past the buffer bit.
#[inline(always)]
fn pair_state(head: u64) -> u64 {
    (head & HIGH) | ((head & LEN_MASK) >> 1)
}

/// The head naming `buffer` with `state` beside it.
#[inline(always)]
fn pair_head(buffer: u64, state: u64) -> u64 {
    (state & HIGH) | ((state & LOW_31) << 1) | buffer
}

/// The panic for slot index `i` at or past `capacity`, out of line and
/// given both by value.
#[cold]
#[inline(never)]
fn out_of_bounds(i: u64, capacity: u64) -> ! {
    panic!("slot index {i} out of bounds for capacity {capacity}")
}

/// The top word at `top` as two 8-byte reads, the low half first, as
/// [`VecSlots::load_top`] describes.
#[inline(always)]
fn load_top_word(top: &AtomicPair) -> u128 {
    let low = top.load_lo();
    fence(Ordering::Acquire);
    let high = top.load_hi();
    fence(Ordering::Acquire);
    pair(low, high)
}

/// Count one more live writer in the top word at `top`, the mode bit set
/// once there are two, in one swap of the whole word: a raise or lowering
/// racing it fails and reads the new mode. The count stops at its 16 bits'
/// top. With `took_entry`, for a handle's first registration once it has
/// taken a writer-table entry, the same swap steps the table generation.
/// The top word left when this add set the mode bit, which also sets
/// [`SANITIZING`]: the caller then runs [`VecSlots::sanitize`].
fn add_writer_to(top: &AtomicPair, took_entry: bool) -> Option<u128> {
    let mut word = load_top_word(top);
    loop {
        let writers = (writers_of(word) + 1).min(u64::from(u16::MAX));
        let counted = with_writers(word, writers);
        let turns = !multi(word as u64) && multi(counted as u64);
        let marked = if turns { counted | (u128::from(SANITIZING) << 64) } else { counted };
        let new = if took_entry { generation_stepped(marked) } else { marked };
        match top.compare_exchange(word, new) {
            Ok(_) => return turns.then_some(new),
            Err(now) => word = now,
        }
    }
}

/// Count `n` live writers fewer in the top word at `top`, the mode bit as it
/// stands (see [`with_writers`]). A count at its top stays there, since
/// writers past the top went uncounted, and the vec keeps the multi-writer
/// protocol.
fn remove_writers_from(top: &AtomicPair, n: u64) {
    let mut word = load_top_word(top);
    loop {
        let writers = writers_of(word);
        if writers == u64::from(u16::MAX) || n == 0 {
            return;
        }
        match top.compare_exchange(word, with_writers(word, writers.saturating_sub(n))) {
            Ok(_) => return,
            Err(now) => word = now,
        }
    }
}

/// Raise the length from `top` for the claim at its end, LIVE in `top`'s
/// epoch, that no writer has counted yet. A raise that loses leaves the
/// length to the writer that won.
#[cold]
#[inline(never)]
fn count_claim(top_word: &AtomicPair, top: u128) {
    if top_word.compare_exchange_lo(top as u64, raised(top) as u64).is_err() {
        std::hint::spin_loop();
    }
}

/// Take back the one-writer claim `mine` in `slot` once its raise lost: a
/// mode flip or another writer moved the length, and no writer counts a
/// LIVE_S claim, so its push goes again. A claim another writer has claimed
/// over needs no taking back.
#[cold]
#[inline(never)]
fn withdraw(slot: &AtomicPair, mine: u128) {
    let back = pair(lo(mine), stepped(hi(mine), FREE, claim_epoch(hi(mine))));
    if slot.compare_exchange(mine, back).is_err() {
        std::hint::spin_loop();
    }
}

/// A word push's claim `mine` in `slot`, slot `n`, once its raise lost to the
/// top word `now`.
#[cold]
#[inline(never)]
fn lost_raise_word(top_word: &AtomicPair, slot: &AtomicPair, mut now: u128, n: u64, mine: u128) -> Raise {
    let epoch = claim_epoch(hi(mine));
    loop {
        if epoch_of(now) == epoch {
            let len = len_of(now as u64);
            if len > n {
                // Counted by another writer.
                return Raise::Done(n as usize);
            }
            if len == n {
                // Uncounted at the length, where any writer may count it the
                // moment it finds it: a claim taken back then would leave a
                // hole, so the push counts its own claim.
                if slot.load() != mine {
                    return Raise::Done(n as usize);
                }
                match top_word.compare_exchange_lo(now as u64, raised(now) as u64) {
                    Ok(_) => return Raise::Done(n as usize),
                    Err(found) => {
                        fence(Ordering::Acquire);
                        now = pair(found, top_word.load_hi());
                        continue;
                    }
                }
            }
        }
        let back = pair(lo(mine), stepped(hi(mine), FREE, epoch));
        if slot.compare_exchange(mine, back).is_ok() {
            return Raise::Again;
        }
        // Counted and then taken or written, or removed by a clear.
        return Raise::Done(n as usize);
    }
}

/// A pair push's claim, the head `mine` of the pair at `slot`, slot `n`, once
/// its raise lost to the top word `now`.
///
/// # Safety
/// `slot` is a pair of buffers with its head first.
#[cold]
#[inline(never)]
unsafe fn lost_raise_pair(top_word: &AtomicPair, slot: *const u8, mut now: u128, n: u64, mine: u64) -> Raise {
    let state = pair_state(mine);
    let epoch = claim_epoch(state);
    loop {
        if epoch_of(now) == epoch {
            let len = len_of(now as u64);
            if len > n {
                return Raise::Done(n as usize);
            }
            if len == n {
                // As for a word slot: the push counts its own claim at the
                // length.
                // SAFETY: the caller's contract.
                if unsafe { slot_pair::load_head(slot) } != mine {
                    return Raise::Done(n as usize);
                }
                match top_word.compare_exchange_lo(now as u64, raised(now) as u64) {
                    Ok(_) => return Raise::Done(n as usize),
                    Err(found) => {
                        fence(Ordering::Acquire);
                        now = pair(found, top_word.load_hi());
                        continue;
                    }
                }
            }
        }
        let back = pair_head(mine & 1, stepped(state, FREE, epoch));
        // SAFETY: the caller's contract; the head names the same buffer.
        if unsafe { slot_pair::swap_head(slot, mine, back) }.is_ok() {
            return Raise::Again;
        }
        return Raise::Done(n as usize);
    }
}

impl<'a> VecSlots<'a> {
    /// The slots of a vec whose top word is `top`, `capacity` slots of
    /// `shape` from `data`, reached through a handle whose writers are
    /// `writers`.
    ///
    /// # Safety
    /// `data` addresses `capacity` slots of `shape`, 64-byte aligned, in a
    /// mapping that outlives `'a` and is accessed only through this
    /// protocol, `capacity` is at most [`MAX_CAPACITY`], and the processor
    /// swaps 16 bytes.
    #[inline(always)]
    pub(crate) unsafe fn new(top: &'a AtomicPair, data: *const u8, capacity: usize, shape: Shape, writers: &'a Writers) -> Self {
        Self { top, data, capacity: capacity as u64, shape, writers }
    }

    /// Register the calling thread as a writer through the handle these
    /// slots were reached by, as [`Writers::enter`] does.
    #[inline(always)]
    pub(crate) fn enter(&self) {
        self.writers.enter(self);
    }

    /// Look at the writer table, as [`Writers::scan`] does, for a handle
    /// that attaches.
    pub(crate) fn scan_writers(&self) {
        self.writers.scan(self);
    }

    /// Finish the flip to the multi-writer protocol whose swap left the top
    /// word `flipped`, then clear [`SANITIZING`] so pushes reserve again. A
    /// one-writer pop leaves the element it took in place, LIVE_S, at or past
    /// the length, where a reservation could uncover it before claiming it.
    /// The slot at the flip's length, the one a one-writer push that read the
    /// top word before the flip can still claim, is made TAKEN when it is
    /// FREE or LIVE_S, so that push claims it LIVE, never LIVE_S, and its
    /// raise fails on the flip. Each such element past it is made FREE, which
    /// a later one-writer push claims LIVE_S. The slots this epoch wrote are
    /// the ones below the first it did not, where the walk stops.
    #[cold]
    #[inline(never)]
    fn sanitize(&self, flipped: u128) {
        if matches!(self.shape, Shape::Word) {
            let epoch = epoch_of(flipped);
            let mut i = len_of(flipped as u64);
            if i < self.capacity {
                let slot = self.word_slot(i);
                loop {
                    let w = halves(slot);
                    let state = hi(w);
                    let claimable = kind(state) == FREE || (kind(state) == LIVE_S && claim_epoch(state) == epoch);
                    if !claimable || slot.compare_exchange(w, pair(lo(w), stepped(state, TAKEN, claim_epoch(state)))).is_ok() {
                        break;
                    }
                }
                i += 1;
            }
            while i < self.capacity {
                let slot = self.word_slot(i);
                let w = halves(slot);
                let state = hi(w);
                if state == 0 || claim_epoch(state) != epoch {
                    break;
                }
                // A swap that loses found the slot claimed since, LIVE.
                if kind(state) == LIVE_S && slot.compare_exchange(w, pair(lo(w), stepped(state, FREE, epoch))).is_err() {
                    std::hint::spin_loop();
                }
                i += 1;
            }
        }
        let mut word = load_top_word(self.top);
        while sanitizing(word) {
            match self.top.compare_exchange(word, word & !(u128::from(SANITIZING) << 64)) {
                Ok(_) => return,
                Err(now) => word = now,
            }
        }
    }

    /// Make TAKEN in this epoch every slot below the length that holds no
    /// claim of this epoch: what a reservation never claimed leaves, as a
    /// gone writer's can. Each slot is read before the epoch it is judged by;
    /// a swap that loses found the slot claimed since.
    #[cold]
    #[inline(never)]
    fn void_stale_below_len(&self) {
        if !matches!(self.shape, Shape::Word) {
            return;
        }
        let len = len_of(self.load_top() as u64).min(self.capacity);
        for i in 0..len {
            let slot = self.word_slot(i);
            let w = halves(slot);
            let state = hi(w);
            let epoch = epoch_of(self.load_top());
            if state != 0 && claim_epoch(state) == epoch {
                continue;
            }
            if slot.compare_exchange(w, pair(lo(w), stepped(state, TAKEN, epoch))).is_err() {
                std::hint::spin_loop();
            }
        }
    }

    /// Flip the vec whose top word at `top_word` reads `top`, of `capacity`
    /// slots, back to the one-writer protocol: `top` counts at most one
    /// writer, the calling thread, at the start of a push or pop, so no
    /// reservation it made waits to be claimed. A length that adds which
    /// found the vec full left past the capacity comes back to it, and the
    /// version steps. A swap that loses leaves the flip to the caller's next
    /// look at the top word.
    #[cold]
    #[inline(never)]
    fn flip_back(top_word: &AtomicPair, capacity: u64, top: u128) {
        let low = top as u64;
        let ver = (low >> VERSION_SHIFT).wrapping_add(1);
        let back = len_of(low).min(capacity) | (ver << VERSION_SHIFT);
        let high = (top >> 64) as u64 & !SANITIZING;
        if top_word.compare_exchange(top, pair(back, high)).is_err() {
            std::hint::spin_loop();
        }
    }

    /// Slot `i`'s start. Panics when `i` is not below the capacity.
    #[inline(always)]
    fn slot(&self, i: u64) -> *const u8 {
        if i >= self.capacity {
            out_of_bounds(i, self.capacity);
        }
        // SAFETY: `i` is below the capacity, and the slots lie from `data`.
        unsafe { self.data.add(i as usize * self.shape.stride()) }
    }

    /// Word slot `i`'s pair of words, in a vec of word slots. Panics when `i`
    /// is not below the capacity.
    #[inline(always)]
    fn word_slot(&self, i: u64) -> &'a AtomicPair {
        if i >= self.capacity {
            out_of_bounds(i, self.capacity);
        }
        // SAFETY: `i` is below the capacity, and the word slots lie from
        // `data`, each starting with its 16-byte pair, 64-byte aligned, per
        // `new`.
        unsafe { AtomicPair::at(self.data.add(i as usize * WORD_STRIDE) as *mut u8) }
    }

    /// The state slot `i` holds.
    #[inline(always)]
    fn state(&self, i: u64) -> u64 {
        match self.shape {
            Shape::Word => self.word_slot(i).load_hi(),
            // SAFETY: the slot is a pair of buffers with its head first.
            Shape::Pair(_) => pair_state(unsafe { slot_pair::load_head(self.slot(i)) }),
        }
    }

    /// The top word as two 8-byte reads, the low half first, each as wide
    /// as the swaps that raise and lower it. A clear swaps both halves at
    /// once, so a low half read after a clear's is followed by that clear's
    /// high half or a later one: a pair read across a clear holds an older
    /// length beside a newer epoch, and a raise or lowering from it fails on
    /// the low half.
    #[inline(always)]
    fn load_top(&self) -> u128 {
        load_top_word(self.top)
    }

    /// The length: slots holding an element or a hole, at most the
    /// capacity, which adds that found the vec full can pass. The handles
    /// read it from the header; a test reads it here.
    #[cfg(test)]
    pub(crate) fn len(&self) -> u64 {
        let low = self.top.load_lo();
        fence(Ordering::Acquire);
        len_of(low).min(self.capacity)
    }

    /// Swap the top word from `current` to `new`: whether it was `current`.
    #[inline(always)]
    fn swap_top(&self, current: u128, new: u128) -> Result<u128, u128> {
        self.top.compare_exchange(current, new)
    }

    /// Raise the length from `top` by one, swapping the low half alone:
    /// whether the low half was still `top`'s.
    #[inline(always)]
    fn raise(&self, top: u128) -> bool {
        self.top.compare_exchange_lo(top as u64, raised(top) as u64).is_ok()
    }

    /// Raise the length from `top` for the claim at its end, LIVE in
    /// `top`'s epoch, that no writer has counted yet.
    #[inline(always)]
    fn count(&self, top: u128) {
        count_claim(self.top, top);
    }

    /// Whether the slot at the length of `top` holds a claim of `top`'s
    /// epoch not counted yet, which a writer counts before it acts.
    #[inline(always)]
    fn uncounted_at(&self, top: u128) -> bool {
        let n = len_of(top as u64);
        if n >= self.capacity {
            return false;
        }
        let state = self.state(n);
        live_in(state, epoch_of(top))
    }

    /// Push the element of `size` bytes, the vec's element size, at `src`:
    /// the index it landed at, or None when the vec is full.
    ///
    /// # Safety
    /// `src` is readable for `size` bytes, and the vec is writable.
    #[inline]
    pub(crate) unsafe fn push(&self, src: *const u8, size: usize) -> Option<usize> {
        match self.shape {
            // SAFETY: the caller's contract; a word slot's element is at most 8.
            Shape::Word => self.push_word(unsafe { word_from(src, size) }),
            Shape::Pair(g) => self.push_pair(&g, src, size),
        }
    }

    /// Push the word `word` onto a word vec: the index it landed at, or None
    /// when the vec is full.
    #[inline(always)]
    pub(crate) fn push_word(&self, word: u64) -> Option<usize> {
        loop {
            let top = self.load_top();
            let n = len_of(top as u64);
            if n >= self.capacity {
                return None;
            }
            if multi(top as u64) {
                if writers_of(top) <= 1 {
                    Self::flip_back(self.top, self.capacity, top);
                    continue;
                }
                if self.writers.news(top) {
                    self.writers.scan_at(self.top, self.data, self.capacity);
                    continue;
                }
                if self.reserves(top) {
                    match self.push_reserved(word) {
                        Reserved::Landed(i) => return Some(i),
                        Reserved::Full => return None,
                        Reserved::Again => continue,
                    }
                }
            }
            let slot = self.word_slot(n);
            let w = halves(slot);
            let state = hi(w);
            let epoch = epoch_of(top);
            if live_in(state, epoch) {
                self.count(top);
                continue;
            }
            // A lone writer claims only a slot never written or one it popped
            // itself: no raise passes such a slot without first claiming it LIVE,
            // so the claim sits at or above the length even when the top word was
            // read before a second writer arrived, and no pop takes it. Any other
            // slot goes the multi-writer way.
            let alone = !multi(top as u64)
                && (kind(state) == FREE || (kind(state) == LIVE_S && claim_epoch(state) == epoch));
            let mine = pair(word, stepped(state, if alone { LIVE_S } else { LIVE }, epoch));
            if slot.compare_exchange(w, mine).is_err() {
                continue;
            }
            if self.raise(top) {
                return Some(n as usize);
            }
            if alone {
                withdraw(slot, mine);
                continue;
            }
            match self.after_lost_raise_word(self.load_top(), n, mine) {
                Raise::Done(i) => return Some(i),
                Raise::Again => continue,
            }
        }
    }

    /// A word push's claim `mine` at slot `n` once its raise lost to the top
    /// word `now`.
    #[inline(always)]
    fn after_lost_raise_word(&self, now: u128, n: u64, mine: u128) -> Raise {
        lost_raise_word(self.top, self.word_slot(n), now, n, mine)
    }

    /// Whether a push from the top word `top` reserves its slot by one add
    /// to the length: two or more writers, a count below its top, room for a
    /// reservation from every writer counted, a capacity no reservation past
    /// which reaches the mode bit, and no flip to the multi-writer protocol
    /// still sanitizing. Each writer holds one push at a time, so the counted
    /// writers bound the adds in flight.
    #[inline(always)]
    fn reserves(&self, top: u128) -> bool {
        let writers = writers_of(top);
        multi(top as u64)
            && writers < u64::from(u16::MAX)
            && self.capacity <= RESERVE_CAPACITY
            && len_of(top as u64) + writers < self.capacity
            && !sanitizing(top)
    }

    /// A multi-writer push of `word`: the slot at the length reserved by one
    /// add to it, which no other writer's add can take, then claimed LIVE in
    /// the epoch read after the add. The claim is in the vec while the
    /// length still covers it in that epoch; a pop that lowered past the
    /// slot before the claim landed, or a clear since, leaves the push to
    /// take its claim back and go again, as a lost raise does.
    #[inline(always)]
    fn push_reserved(&self, word: u64) -> Reserved {
        let before = self.top.fetch_add_lo(1);
        let n = len_of(before);
        if n >= self.capacity {
            // An add past the end, after another writer took the room this one
            // counted on: the length stands past the capacity until pops lower
            // past the slots that are not there.
            return Reserved::Full;
        }
        let epoch = self.top.load_hi() as u32;
        fence(Ordering::Acquire);
        let slot = self.word_slot(n);
        // The slot is this push's to fill unless another writer's claim sits
        // in it: a claim that sat at the length, which the add counted, or one
        // that lands in it meanwhile. Anything else that changes it, such as a
        // lone writer taking back the claim it made before a second writer
        // came, leaves it to claim again, or the reserved slot would stand
        // empty below the length.
        let mine = loop {
            let w = halves(slot);
            let state = hi(w);
            if live_in(state, epoch) {
                return Reserved::Again;
            }
            let mine = pair(word, stepped(state, LIVE, epoch));
            if slot.compare_exchange(w, mine).is_ok() {
                break mine;
            }
        };
        let now = self.load_top();
        if epoch_of(now) == epoch && len_of(now as u64) > n {
            return Reserved::Landed(n as usize);
        }
        match self.after_lost_raise_word(now, n, mine) {
            Raise::Done(i) => Reserved::Landed(i),
            Raise::Again => Reserved::Again,
        }
    }

    /// Pop the last element, `size` bytes, into `dst`: whether there was one.
    ///
    /// # Safety
    /// `dst` is writable for `size` bytes, the vec's element size, and the
    /// vec is writable.
    #[inline]
    pub(crate) unsafe fn pop(&self, dst: *mut u8, size: usize) -> bool {
        match self.shape {
            Shape::Word => match self.pop_word() {
                Some(word) => {
                    // SAFETY: the caller's contract.
                    unsafe { word_into(word, dst, size) };
                    true
                }
                None => false,
            },
            Shape::Pair(g) => self.pop_pair(&g, dst, size),
        }
    }

    /// Pop the last element of a word vec: its word, or None when the vec
    /// holds none. With one writer, whose elements are LIVE_S, the low half
    /// alone says the length and the mode, and the lowering alone pops; the
    /// lowering compares the mode bit, so a flip lands first or makes it fail
    /// and the pop goes the general way.
    #[inline(always)]
    pub(crate) fn pop_word(&self) -> Option<u64> {
        let low = self.top.load_lo();
        fence(Ordering::Acquire);
        if !multi(low) {
            let n = len_of(low);
            if n == 0 {
                return None;
            }
            if n <= self.capacity {
                let epoch = self.top.load_hi() as u32;
                let slot = self.word_slot(n - 1);
                let state = slot.load_hi();
                if live_s_in(state, epoch) {
                    fence(Ordering::Acquire);
                    let value = slot.load_lo();
                    if self.top.compare_exchange_lo(low, low.wrapping_add(LOWERING)).is_ok() {
                        return Some(value);
                    }
                }
            }
        }
        Self::pop_word_any(self.top, self.data, self.capacity, self.writers)
    }

    /// [`pop_word`](Self::pop_word) in either mode, from a fresh read of the
    /// top word, for the vec whose top word is `top` and whose `capacity`
    /// word slots lie from `data`, reached through a handle whose writers are
    /// `writers`. Out of line, taking what it reads as arguments, so the
    /// one-writer pop stays small and keeps its slots in registers.
    #[inline(never)]
    fn pop_word_any(top: &'a AtomicPair, data: *const u8, capacity: u64, writers: &'a Writers) -> Option<u64> {
        VecSlots { top, data, capacity, shape: Shape::Word, writers }.pop_word_loop()
    }

    /// The loop of [`pop_word_any`](Self::pop_word_any).
    #[inline(always)]
    fn pop_word_loop(&self) -> Option<u64> {
        loop {
            let top = self.load_top();
            let n = len_of(top as u64);
            let shared = multi(top as u64);
            if shared {
                if writers_of(top) <= 1 {
                    Self::flip_back(self.top, self.capacity, top);
                    continue;
                }
                if self.writers.news(top) {
                    self.writers.scan(self);
                    continue;
                }
                if n < self.capacity {
                    let at = self.word_slot(n).load_hi();
                    if live_in(at, epoch_of(top)) {
                        self.count(top);
                        continue;
                    }
                }
            }
            if n == 0 {
                return None;
            }
            if n > self.capacity {
                // Past the end by adds that found the vec full: no slot is
                // there.
                self.lower_past(top);
                continue;
            }
            let slot = self.word_slot(n - 1);
            let state = slot.load_hi();
            if !current(state, epoch_of(top)) {
                self.lower_past(top);
                continue;
            }
            // The value read after the state, and the take a swap of the state
            // half alone: every write steps the state, so the take lands only
            // while the slot holds the value read beside that state.
            fence(Ordering::Acquire);
            let value = slot.load_lo();
            if !shared && kind(state) == LIVE_S {
                // The only writer's own element: the lowering alone pops it, and
                // the slot it leaves LIVE_S above the length is dead in both
                // modes. The lowering compares the mode bit, so a flip lands
                // first or makes it fail.
                if self.top.compare_exchange_lo(top as u64, lowered(top) as u64).is_ok() {
                    return Some(value);
                }
                continue;
            }
            // With several writers the slot is left TAKEN, which a one-writer
            // push that read the top word before a flip claims LIVE, never
            // LIVE_S. The only writer leaves it FREE, which its own next push
            // claims LIVE_S, so the slot pops by the lowering alone again.
            let left = if shared { TAKEN } else { FREE };
            if slot.compare_exchange_hi(state, stepped(state, left, claim_epoch(state))).is_ok() {
                self.lower_past(top);
                return Some(value);
            }
        }
    }

    /// Lower the length from `top` past its last slot, which holds no
    /// element: this caller took it, or found it a hole. A lowering that
    /// loses leaves the length as it stands, and that slot a hole below it
    /// when a push raised the length meanwhile.
    #[inline(always)]
    fn lower_past(&self, top: u128) {
        if self.top.compare_exchange_lo(top as u64, lowered(top) as u64).is_err() {
            std::hint::spin_loop();
        }
    }

    /// Copy element `i`, `size` bytes, into `dst`: whether `i` is below the
    /// length and its slot holds an element.
    ///
    /// # Safety
    /// `dst` is writable for `size` bytes, the vec's element size.
    #[inline]
    pub(crate) unsafe fn get(&self, i: usize, dst: *mut u8, size: usize) -> bool {
        match self.shape {
            Shape::Word => match self.get_word(i) {
                Some(word) => {
                    // SAFETY: the caller's contract.
                    unsafe { word_into(word, dst, size) };
                    true
                }
                None => false,
            },
            Shape::Pair(g) => self.get_pair(&g, i, dst, size),
        }
    }

    /// Element `i` of a word vec: its word, or None when `i` is past the
    /// length or names a hole. With one writer no reservation is in flight,
    /// so every slot below the length holds a claim of the epoch or a hole,
    /// and the low half alone says the length; with two or more,
    /// [`get_word_shared`](Self::get_word_shared) reads. The slot's word and
    /// state are read as two 8-byte halves: while the version holds, the
    /// slot changes only by a set, which swaps both halves of an element
    /// for another, or by a pop's take, which leaves the word.
    #[inline(always)]
    pub(crate) fn get_word(&self, i: usize) -> Option<u64> {
        if i as u64 >= self.capacity {
            return None;
        }
        // SAFETY: `i` is below the capacity, and the word slots lie from
        // `data`, each starting with its 16-byte pair, 64-byte aligned, per
        // `new`.
        let slot = unsafe { AtomicPair::at(self.data.add(i * WORD_STRIDE) as *mut u8) };
        loop {
            let low = self.top.load_lo();
            fence(Ordering::Acquire);
            if multi(low) {
                return Self::get_word_shared(self.top, self.data, self.capacity, i);
            }
            // The capacity is below 2^32, so `i` is a 32-bit index here.
            if i as u32 >= low as u32 {
                return None;
            }
            #[cfg(test)]
            tests::between_get_reads();
            let word = slot.load_lo();
            let state = slot.load_hi();
            if self.version_held(low) {
                return (state & LIVE != 0).then_some(word);
            }
        }
    }

    /// [`get_word`](Self::get_word) while two or more writers write, for the
    /// vec whose top word is `top` and whose `capacity` word slots lie from
    /// `data`: a reservation raises the length over its slot before claiming
    /// it, so the slot's claim must be of the epoch, and adds that found the
    /// vec full can leave the length past the capacity. Out of line, taking
    /// what it reads as arguments, so a one-writer read keeps its slots in
    /// registers.
    #[inline(never)]
    fn get_word_shared(top: &AtomicPair, data: *const u8, capacity: u64, i: usize) -> Option<u64> {
        loop {
            let word = load_top_word(top);
            if i as u64 >= len_of(word as u64).min(capacity) {
                return None;
            }
            #[cfg(test)]
            tests::between_get_reads();
            // SAFETY: `i` is below the capacity, and the word slots lie from
            // `data`, each starting with its 16-byte pair, 64-byte aligned.
            let w = unsafe { AtomicPair::at(data.add(i * WORD_STRIDE) as *mut u8) }.load();
            if version_held_at(top, word as u64) {
                return current(hi(w), epoch_of(word)).then_some(lo(w));
            }
        }
    }

    /// Whether the top word's version is still the one in the low half
    /// `low`, as [`version_held_at`] says.
    #[inline(always)]
    fn version_held(&self, low: u64) -> bool {
        version_held_at(self.top, low)
    }

    /// Replace element `i` with the `size` bytes at `src`: false when `i` is
    /// past the length or names a hole.
    ///
    /// # Safety
    /// `src` is readable for `size` bytes, the vec's element size, and the
    /// vec is writable.
    #[inline]
    pub(crate) unsafe fn set(&self, i: usize, src: *const u8, size: usize) -> bool {
        match self.shape {
            // SAFETY: the caller's contract; a word slot's element is at most 8.
            Shape::Word => self.set_word(i, unsafe { word_from(src, size) }),
            Shape::Pair(g) => self.set_pair(&g, i, src, size),
        }
    }

    /// Replace element `i` of a word vec with `word`, as for
    /// [`set`](Self::set).
    #[inline]
    pub(crate) fn set_word(&self, i: usize, word: u64) -> bool {
        loop {
            let top = self.load_top();
            if i as u64 >= len_of(top as u64).min(self.capacity) {
                return false;
            }
            let slot = self.word_slot(i as u64);
            let w = halves(slot);
            if !self.version_held(top as u64) {
                continue;
            }
            let state = hi(w);
            if !current(state, epoch_of(top)) {
                return false;
            }
            if slot.compare_exchange(w, pair(word, stepped(state, kind(state), claim_epoch(state)))).is_ok() {
                return true;
            }
        }
    }

    /// Empty the vec: the length to zero, the version and the epoch
    /// stepped, so every claim made before reads as one the clear removed.
    pub(crate) fn clear(&self) {
        let mut top = self.load_top();
        loop {
            match self.swap_top(top, cleared(top)) {
                Ok(_) => return,
                Err(now) => top = now,
            }
        }
    }

    /// Count one more live writer, as [`add_writer_to`] does, for a test that
    /// registers a thread by hand.
    #[cfg(test)]
    pub(crate) fn add_writer(&self) {
        if let Some(flipped) = add_writer_to(self.top, false) {
            self.sanitize(flipped);
        }
    }

    /// Count one live writer fewer, as [`remove_writers_from`] does, for a
    /// test that registers a thread by hand.
    #[cfg(test)]
    pub(crate) fn remove_writer(&self) {
        remove_writers_from(self.top, 1);
    }

    /// Make slot `i` say `kind` in `epoch`, its element in place, as a
    /// writer that stopped there would leave it.
    #[cfg(test)]
    pub(crate) fn mark(&self, i: u64, kind: u64, epoch: u32) {
        match self.shape {
            Shape::Word => {
                let slot = self.word_slot(i);
                let w = slot.load();
                slot.store(pair(lo(w), stepped(hi(w), kind, epoch)));
            }
            Shape::Pair(_) => {
                let s = self.slot(i);
                // SAFETY: the slot is a pair with its head first.
                let head = unsafe { slot_pair::load_head(s) };
                let next = pair_head(head & 1, stepped(pair_state(head), kind, epoch));
                // SAFETY: as above, the same buffer named.
                assert!(unsafe { slot_pair::swap_head(s, head, next) }.is_ok());
            }
        }
    }

    /// Make slot `i` a hole of the current epoch, as a pop whose lowering
    /// of the length lost to a push leaves it.
    #[cfg(test)]
    pub(crate) fn make_hole(&self, i: u64) {
        self.mark(i, TAKEN, epoch_of(self.load_top()));
    }

    /// Count one more live writer and step the table generation, as a
    /// handle's first registration does once it has taken its entry, for a
    /// test that writes an entry by hand.
    #[cfg(test)]
    pub(crate) fn add_writer_with_entry(&self) {
        if let Some(flipped) = add_writer_to(self.top, true) {
            self.sanitize(flipped);
        }
    }

    // ---- pairs of buffers -------------------------------------------------

    fn push_pair(&self, g: &SlotGeometry, src: *const u8, size: usize) -> Option<usize> {
        loop {
            let top = self.load_top();
            let n = len_of(top as u64);
            if n >= self.capacity {
                return None;
            }
            let epoch = epoch_of(top);
            // SAFETY: the slot is a writable pair of buffers laid out by `g`,
            // and the fill writes the element's bytes from `src` into one.
            let written = unsafe {
                slot_pair::write_with(
                    self.slot(n),
                    g,
                    |buffer| crate::atomic_copy::store_bytes(buffer, src, size),
                    |head| {
                        let state = pair_state(head);
                        (!live_in(state, epoch))
                            .then(|| pair_head((head & 1) ^ 1, stepped(state, LIVE, epoch)))
                    },
                )
            };
            let mine = match written {
                Ok(mine) => mine,
                Err(found) => {
                    debug_assert_eq!(claim_epoch(pair_state(found)), epoch, "a write declines only a claim to count");
                    self.count(top);
                    continue;
                }
            };
            if self.raise(top) {
                return Some(n as usize);
            }
            match self.after_lost_raise_pair(self.load_top(), n, mine) {
                Raise::Done(i) => return Some(i),
                Raise::Again => continue,
            }
        }
    }

    /// A pair push's claim, the head `mine` at slot `n`, once its raise lost
    /// to the top word `now`.
    #[inline(always)]
    fn after_lost_raise_pair(&self, now: u128, n: u64, mine: u64) -> Raise {
        // SAFETY: the slot is a pair of buffers with its head first.
        unsafe { lost_raise_pair(self.top, self.slot(n), now, n, mine) }
    }

    fn pop_pair(&self, g: &SlotGeometry, dst: *mut u8, size: usize) -> bool {
        loop {
            let top = self.load_top();
            if self.uncounted_at(top) {
                self.count(top);
                continue;
            }
            let n = len_of(top as u64);
            if n == 0 {
                return false;
            }
            let slot = self.slot(n - 1);
            // SAFETY: the slot is a pair of buffers laid out by `g`; the copy
            // reads the element's bytes from the buffer the head names into
            // `dst`, writable for them per `pop`.
            let head = unsafe { slot_pair::read_head(slot, g, |buffer| copy_out(dst, buffer, size)) };
            let state = pair_state(head);
            if kind(state) != LIVE {
                self.lower_past(top);
                continue;
            }
            let taken = pair_head(head & 1, stepped(state, TAKEN, claim_epoch(state)));
            // SAFETY: as above; the head names the same buffer.
            if unsafe { slot_pair::swap_head(slot, head, taken) }.is_ok() {
                self.lower_past(top);
                return true;
            }
        }
    }

    fn get_pair(&self, g: &SlotGeometry, i: usize, dst: *mut u8, size: usize) -> bool {
        loop {
            let top = self.load_top();
            if i as u64 >= len_of(top as u64).min(self.capacity) {
                return false;
            }
            #[cfg(test)]
            tests::between_get_reads();
            // SAFETY: as in `pop_pair`.
            let head = unsafe { slot_pair::read_head(self.slot(i as u64), g, |buffer| copy_out(dst, buffer, size)) };
            if self.version_held(top as u64) {
                return kind(pair_state(head)) == LIVE;
            }
        }
    }

    fn set_pair(&self, g: &SlotGeometry, i: usize, src: *const u8, size: usize) -> bool {
        loop {
            let top = self.load_top();
            if i as u64 >= len_of(top as u64).min(self.capacity) {
                return false;
            }
            let moved = Cell::new(false);
            // SAFETY: as in `push_pair`.
            let written = unsafe {
                slot_pair::write_with(
                    self.slot(i as u64),
                    g,
                    |buffer| crate::atomic_copy::store_bytes(buffer, src, size),
                    |head| {
                        if !self.version_held(top as u64) {
                            moved.set(true);
                            return None;
                        }
                        let state = pair_state(head);
                        (kind(state) == LIVE).then(|| pair_head((head & 1) ^ 1, stepped(state, LIVE, claim_epoch(state))))
                    },
                )
            };
            if written.is_ok() {
                return true;
            }
            if !moved.get() {
                return false;
            }
        }
    }
}

#[inline(always)]
fn pair(lo: u64, hi: u64) -> u128 {
    u128::from(lo) | (u128::from(hi) << 64)
}

/// A word slot's pair read as its two halves, each whole and the two not
/// together: a value only a swap of the whole slot then acts on, the swap
/// failing on one the halves tore.
#[inline(always)]
fn halves(slot: &AtomicPair) -> u128 {
    pair(slot.load_lo(), slot.load_hi())
}

#[inline(always)]
fn lo(w: u128) -> u64 {
    w as u64
}

#[inline(always)]
fn hi(w: u128) -> u64 {
    (w >> 64) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    thread_local! {
        /// What a test runs, once, between a get's read of the top word and
        /// its read of the slot.
        static BETWEEN_GET_READS: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    }

    /// Run the step a test left for this thread between a get's two reads.
    pub(super) fn between_get_reads() {
        let step = BETWEEN_GET_READS.with(|s| s.borrow_mut().take());
        if let Some(step) = step {
            step();
        }
    }

    /// A get that read the length before a pop took slot 2 and a push
    /// claimed it, and reads the slot after, finds a claim the length does
    /// not count: it answers None, since the vec holds nothing at 2 once the
    /// pop has lowered the length past it.
    #[test]
    fn a_get_does_not_answer_a_claim_the_length_has_not_counted() {
        for shape in both_shapes() {
            let f = Fixture::new(8, shape);
            for v in 1..=3 {
                push(&f, v);
            }
            let g = f.clone();
            BETWEEN_GET_READS.with(|s| {
                *s.borrow_mut() = Some(Box::new(move || {
                    assert_eq!(pop(&g), Some(3));
                    mark(&g, 2, LIVE, epoch_of(g.slots().load_top()));
                }))
            });
            assert_eq!(get(&f, 2), None, "{shape:?}: slot 2 holds a claim the length does not count");
            assert_eq!(f.slots().len(), 2, "{shape:?}");
        }
    }

    /// A get never returns a value a pop took: a reservation that raised the
    /// length over a slot a one-writer pop left, its claim not landed yet,
    /// leaves that slot a hole to a reader.
    #[test]
    fn a_get_never_returns_a_value_a_one_writer_pop_took() {
        let f = Fixture::new(8, Shape::Word);
        push(&f, 1);
        push(&f, 2);
        assert_eq!(pop(&f), Some(2));
        f.slots().add_writer();
        f.slots().add_writer();
        assert!(multi(f.slots().load_top() as u64), "two writers");
        f.top().fetch_add_lo(1);
        assert_eq!(get(&f, 1), None, "slot 1 holds the value the pop took");
    }

    /// A clear steps the version and the epoch and keeps the mode, the table
    /// generation and the writer count; the generation wraps in its 16 bits,
    /// the epoch and the count beside it as they were.
    #[test]
    fn a_clear_keeps_the_table_generation_and_the_writer_count() {
        let top = with_writers(generation_stepped(pair(5 | (3 << VERSION_SHIFT), 7)), 2);
        let after = cleared(top);
        assert_eq!(len_of(after as u64), 0);
        assert_eq!((after as u64) >> VERSION_SHIFT, 4, "the version stepped");
        assert_eq!(epoch_of(after), 8, "the epoch stepped");
        assert_eq!(generation_of(after), 1);
        assert_eq!(writers_of(after), 2);
        assert!(multi(after as u64));

        let full = pair(0, 7 | GENERATION_MASK | (3 << WRITERS_SHIFT));
        let wrapped = generation_stepped(full);
        assert_eq!(generation_of(wrapped), 0);
        assert_eq!((epoch_of(wrapped), writers_of(wrapped)), (7, 3));
    }

    /// One cache line of slot memory.
    #[derive(Clone, Copy)]
    #[repr(C, align(64))]
    struct Line([u8; 64]);

    /// A vec's top word and slot memory on the heap, zeroed as a fresh file
    /// is.
    struct Fixture {
        top: Line,
        lines: Vec<Line>,
        capacity: usize,
        shape: Shape,
        writers: Writers,
    }

    impl Fixture {
        fn new(capacity: usize, shape: Shape) -> Arc<Self> {
            let lines = vec![Line([0; 64]); capacity * shape.stride() / 64];
            // SAFETY: an empty writer table, at an aligned address no access
            // reaches.
            let writers = unsafe { Writers::new(std::ptr::NonNull::<WriterEntry>::dangling().as_ptr() as *const u8, 0) };
            Arc::new(Self { top: Line([0; 64]), lines, capacity, shape, writers })
        }

        fn top(&self) -> &AtomicPair {
            // SAFETY: the line is 64-byte aligned and lives as long as the
            // fixture, accessed only as this pair.
            unsafe { AtomicPair::at(self.top.0.as_ptr() as *mut u8) }
        }

        /// Empty a fixture of word slots as a fresh file is, by atomic stores
        /// a reader may race: every slot's pair, then the top word.
        fn reset(&self) {
            for line in &self.lines {
                // SAFETY: as for `top`; a word slot's pair starts its line.
                unsafe { AtomicPair::at(line.0.as_ptr() as *mut u8) }.store(0);
            }
            self.top().store(0);
        }

        fn slots(&self) -> VecSlots<'_> {
            // SAFETY: the lines hold `capacity` slots of the shape, 64-byte
            // aligned, and live as long as the fixture.
            unsafe { VecSlots::new(self.top(), self.lines.as_ptr() as *const u8, self.capacity, self.shape, &self.writers) }
        }
    }

    /// The pair shape a 24-byte element takes in a vec.
    fn pair_shape() -> Shape {
        Shape::Pair(crate::shared_vec::vec_geometry(24))
    }

    fn both_shapes() -> [Shape; 2] {
        [Shape::Word, pair_shape()]
    }

    /// A 24-byte element carrying `v` in each of its words, each word
    /// differently masked, so a copy mixing two writes shows.
    fn element(v: u64) -> [u64; 3] {
        [v, v ^ 0x5555_5555_5555_5555, v ^ 0xAAAA_AAAA_AAAA_AAAA]
    }

    fn push(f: &Fixture, v: u64) -> Option<usize> {
        match f.shape {
            Shape::Word => f.slots().push_word(v),
            Shape::Pair(_) => {
                let e = element(v);
                // SAFETY: the element is 24 readable bytes, the vec's element.
                unsafe { f.slots().push(e.as_ptr() as *const u8, 24) }
            }
        }
    }

    fn pop(f: &Fixture) -> Option<u64> {
        match f.shape {
            Shape::Word => f.slots().pop_word(),
            Shape::Pair(_) => {
                let mut e = [0u64; 3];
                // SAFETY: as in `push`, writable.
                if !unsafe { f.slots().pop(e.as_mut_ptr() as *mut u8, 24) } {
                    return None;
                }
                assert_eq!(e, element(e[0]), "a torn element popped");
                Some(e[0])
            }
        }
    }

    fn get(f: &Fixture, i: usize) -> Option<u64> {
        match f.shape {
            Shape::Word => f.slots().get_word(i),
            Shape::Pair(_) => {
                let mut e = [0u64; 3];
                // SAFETY: as in `pop`.
                if !unsafe { f.slots().get(i, e.as_mut_ptr() as *mut u8, 24) } {
                    return None;
                }
                assert_eq!(e, element(e[0]), "a torn element read");
                Some(e[0])
            }
        }
    }

    fn set(f: &Fixture, i: usize, v: u64) -> bool {
        match f.shape {
            Shape::Word => f.slots().set_word(i, v),
            Shape::Pair(_) => {
                let e = element(v);
                // SAFETY: as in `push`.
                unsafe { f.slots().set(i, e.as_ptr() as *const u8, 24) }
            }
        }
    }

    /// Make slot `i` say `kind` in `epoch`, as [`VecSlots::mark`] does.
    fn mark(f: &Fixture, i: usize, kind: u64, epoch: u32) {
        f.slots().mark(i as u64, kind, epoch);
    }

    #[test]
    fn the_top_word_never_repeats_and_a_clear_steps_its_epoch() {
        let t = raised(raised(0));
        let l = lowered(t);
        assert_eq!((len_of(l as u64), epoch_of(l)), (1, 0));
        assert_ne!(raised(l), t, "a lowering and a raise do not bring a top word back");
        let c = cleared(raised(l));
        assert_eq!((len_of(c as u64), epoch_of(c)), (0, 1));
        assert_ne!(c & u128::from(u64::MAX), 0, "a clear steps the version too");
    }

    #[test]
    fn push_get_set_pop_and_clear_round_trip_in_both_shapes() {
        for shape in both_shapes() {
            let f = Fixture::new(4, shape);
            for v in 0..4u64 {
                assert_eq!(push(&f, 10 + v), Some(v as usize), "{shape:?}");
            }
            assert_eq!(push(&f, 99), None, "{shape:?}: a full vec refuses");
            assert_eq!(f.slots().len(), 4, "{shape:?}: a refusal moves nothing");
            assert_eq!(get(&f, 2), Some(12), "{shape:?}");
            assert_eq!(get(&f, 4), None, "{shape:?}");
            assert!(set(&f, 2, 42), "{shape:?}");
            assert_eq!(get(&f, 2), Some(42), "{shape:?}");
            assert_eq!(pop(&f), Some(13), "{shape:?}");
            assert_eq!(pop(&f), Some(42), "{shape:?}");
            assert_eq!(f.slots().len(), 2, "{shape:?}");
            f.slots().clear();
            assert_eq!(f.slots().len(), 0, "{shape:?}");
            assert_eq!(get(&f, 0), None, "{shape:?}");
            assert_eq!(pop(&f), None, "{shape:?}");
            assert_eq!(push(&f, 7), Some(0), "{shape:?}: a cleared vec takes a push at 0");
            assert_eq!((pop(&f), pop(&f)), (Some(7), None), "{shape:?}: the cleared elements stay gone");
        }
    }

    /// With one writer registered a pop is the lowering alone: the slot it
    /// popped still says LIVE_S above the length, and the next push claims
    /// over it.
    #[test]
    fn a_lone_writer_pops_by_the_lowering_alone() {
        let f = Fixture::new(8, Shape::Word);
        f.slots().add_writer();
        for v in 1..=3u64 {
            assert!(push(&f, v).is_some());
        }
        assert_eq!(pop(&f), Some(3));
        let top = f.slots().load_top();
        assert!(!multi(top as u64));
        assert_eq!(len_of(top as u64), 2);
        assert_eq!(kind(f.slots().word_slot(2).load_hi()), LIVE_S, "popped without a take");
        assert_eq!(push(&f, 9), Some(2));
        assert_eq!((pop(&f), pop(&f), pop(&f), pop(&f)), (Some(9), Some(2), Some(1), None));
    }

    /// A second writer registering sets the mode bit, and each element the
    /// first pushed alone pops once, taken as the multi-writer protocol takes.
    #[test]
    fn a_second_writer_flips_the_vec_and_its_elements_pop_once() {
        let f = Fixture::new(8, Shape::Word);
        f.slots().add_writer();
        for v in 1..=4u64 {
            assert!(push(&f, v).is_some());
        }
        f.slots().add_writer();
        assert!(multi(f.slots().load_top() as u64));
        assert_eq!(pop(&f), Some(4));
        assert_eq!(kind(f.slots().word_slot(3).load_hi()), TAKEN);
        assert_eq!((pop(&f), pop(&f), pop(&f), pop(&f)), (Some(3), Some(2), Some(1), None));
    }

    /// The second writer leaving brings the count back to one and leaves the
    /// mode bit for the writer left, whose next push clears it before
    /// pushing as the only writer.
    #[test]
    fn the_second_writer_leaving_returns_the_vec_to_one_writer() {
        let f = Fixture::new(8, Shape::Word);
        f.slots().add_writer();
        f.slots().add_writer();
        assert!(multi(f.slots().load_top() as u64));
        f.slots().remove_writer();
        let top = f.slots().load_top();
        assert!(multi(top as u64), "the bit waits for the writer left");
        assert_eq!(writers_of(top), 1);
        assert_eq!(push(&f, 7), Some(0));
        assert!(!multi(f.slots().load_top() as u64), "its push cleared it");
        assert_eq!(push(&f, 8), Some(1));
        assert_eq!(kind(f.slots().word_slot(1).load_hi()), LIVE_S, "the next push claimed as the only writer");
        assert_eq!((pop(&f), pop(&f), pop(&f)), (Some(8), Some(7), None));
    }

    /// A get never returns a value a pop returned before the get began, while
    /// one writer pushes and pops alone and a second comes and goes: each
    /// arrival flips the vec to the multi-writer protocol, whose reservations
    /// raise the length over slots before claiming them, and each departure
    /// flips it back. Every value is pushed once.
    #[test]
    fn a_get_never_returns_a_value_popped_before_it_while_writers_come_and_go() {
        const VALUES: u64 = 1 << 19;
        const ROUNDS: u64 = 20_000;
        let f = Fixture::new(64, Shape::Word);
        let popped: Vec<AtomicU64> = (0..VALUES).map(|_| AtomicU64::new(u64::MAX)).collect();
        let pops = AtomicU64::new(0);
        let done = AtomicBool::new(false);
        let record = |v: u64| {
            popped[v as usize].store(pops.load(Ordering::Acquire), Ordering::Relaxed);
            pops.fetch_add(1, Ordering::AcqRel);
        };
        // Eight elements no one pops, so the reader always finds some.
        for v in 0..8 {
            assert_eq!(push(&f, v), Some(v as usize));
        }
        let start = std::sync::Barrier::new(3);
        std::thread::scope(|s| {
            let alone = s.spawn(|| {
                let mut next = 8;
                start.wait();
                for round in 0..ROUNDS {
                    let k = 1 + round % 7;
                    let pushed = (0..k).filter(|_| push(&f, next).inspect(|_| next += 1).is_some()).count();
                    for _ in 0..pushed {
                        match pop(&f) {
                            Some(v) => record(v),
                            None => std::thread::yield_now(),
                        }
                    }
                    std::thread::yield_now();
                }
            });
            let visitor = s.spawn(|| {
                let mut next = VALUES / 2;
                start.wait();
                for round in 0..ROUNDS / 4 {
                    f.slots().add_writer();
                    f.slots().add_writer();
                    let m = 1 + round % 5;
                    for _ in 0..m {
                        if push(&f, next).is_some() {
                            next += 1;
                        }
                    }
                    for _ in 0..m {
                        if let Some(v) = pop(&f) {
                            record(v);
                        }
                    }
                    f.slots().remove_writer();
                    f.slots().remove_writer();
                    std::thread::yield_now();
                }
            });
            // The reader reads the last slots below the length, where a
            // reservation's slot waits for its claim.
            let reader = s.spawn(|| {
                let mut seen = 0u64;
                let mut reads = 0u64;
                start.wait();
                while !done.load(Ordering::Acquire) {
                    let before = pops.load(Ordering::Acquire);
                    let len = f.slots().len();
                    for i in len.saturating_sub(2)..len {
                        if let Some(v) = get(&f, i as usize) {
                            let at = popped[v as usize].load(Ordering::Relaxed);
                            assert!(at == u64::MAX || at >= before, "get({i}) returned {v}, popped at {at}, before the get began at {before}");
                            seen += 1;
                        }
                    }
                    reads += 1;
                    if reads.is_multiple_of(64) {
                        std::thread::yield_now();
                    }
                }
                seen
            });
            alone.join().unwrap();
            visitor.join().unwrap();
            done.store(true, Ordering::Release);
            assert!(reader.join().unwrap() > 0, "the reader read elements");
        });
    }

    /// A get never returns a value a pop returned before the get began, when
    /// one-writer pops have left their elements in place and a second
    /// writer's arrival flips the vec: the pushes after the flip reserve
    /// those slots, raising the length over each before claiming it. Each
    /// round starts from an empty vec, and a reader reads the last slots
    /// below the length throughout.
    #[test]
    fn a_get_never_returns_a_value_popped_before_it_across_a_flip() {
        const ROUNDS: u64 = 4000;
        let f = Fixture::new(16, Shape::Word);
        let popped: Vec<AtomicU64> = (0..ROUNDS * 16).map(|_| AtomicU64::new(u64::MAX)).collect();
        let pops = AtomicU64::new(0);
        let done = AtomicBool::new(false);
        let start = std::sync::Barrier::new(2);
        std::thread::scope(|s| {
            let reader = s.spawn(|| {
                let mut seen = 0u64;
                start.wait();
                while !done.load(Ordering::Acquire) {
                    let before = pops.load(Ordering::Acquire);
                    let len = f.slots().len();
                    for i in len.saturating_sub(2)..len {
                        if let Some(v) = get(&f, i as usize) {
                            let at = popped[v as usize].load(Ordering::Relaxed);
                            assert!(at == u64::MAX || at >= before, "get({i}) returned {v}, popped at {at}, before the get began at {before}");
                            seen += 1;
                        }
                    }
                }
                seen
            });
            start.wait();
            for round in 0..ROUNDS {
                f.reset();
                let base = round * 16;
                for j in 0..8 {
                    assert_eq!(push(&f, base + j), Some(j as usize));
                }
                for _ in 0..4 {
                    let v = pop(&f).expect("an element to pop");
                    popped[v as usize].store(pops.load(Ordering::Acquire), Ordering::Relaxed);
                    pops.fetch_add(1, Ordering::AcqRel);
                }
                f.slots().add_writer();
                f.slots().add_writer();
                for j in 8..12 {
                    assert!(push(&f, base + j).is_some());
                }
                std::thread::yield_now();
            }
            done.store(true, Ordering::Release);
            assert!(reader.join().unwrap() > 0, "the reader read elements");
        });
    }

    /// After two writers push and pop, leaving their slots TAKEN, the one
    /// writer left pushes over such a slot LIVE, pops it leaving it FREE,
    /// and from then pushes it LIVE_S and pops it by the lowering alone.
    #[test]
    fn the_one_writer_left_wins_back_its_pop_after_a_multi_writer_phase() {
        let f = Fixture::new(8, Shape::Word);
        f.slots().add_writer();
        f.slots().add_writer();
        assert_eq!((push(&f, 1), push(&f, 2)), (Some(0), Some(1)));
        assert_eq!((pop(&f), pop(&f)), (Some(2), Some(1)));
        let state = |i: u64| f.slots().word_slot(i).load_hi();
        assert_eq!(kind(state(0)), TAKEN);
        f.slots().remove_writer();
        f.slots().remove_writer();
        assert_eq!(push(&f, 3), Some(0));
        assert_eq!(kind(state(0)), LIVE, "a push over a TAKEN slot claims it LIVE");
        assert_eq!(pop(&f), Some(3));
        assert_eq!(kind(state(0)), FREE, "the only writer's pop left it FREE");
        assert_eq!(push(&f, 4), Some(0));
        assert_eq!(kind(state(0)), LIVE_S, "and its next push claimed it as the only writer");
        let before = f.slots().word_slot(0).load();
        assert_eq!(pop(&f), Some(4));
        assert_eq!(f.slots().word_slot(0).load(), before, "a pop by the lowering alone leaves the slot as it was");
    }

    /// A flip to the multi-writer protocol makes the slot at its length
    /// TAKEN, which a one-writer push that read the top word before the flip
    /// can only claim LIVE, and makes FREE each element one-writer pops left
    /// in place past it, so no reservation uncovers a popped element; the
    /// elements below the length stand.
    #[test]
    fn a_flip_clears_what_one_writer_pops_left_in_place() {
        let f = Fixture::new(8, Shape::Word);
        for v in 1..=5 {
            push(&f, v);
        }
        assert_eq!((pop(&f), pop(&f), pop(&f)), (Some(5), Some(4), Some(3)));
        let state = |i: u64| f.slots().word_slot(i).load_hi();
        assert_eq!((kind(state(2)), kind(state(3)), kind(state(4))), (LIVE_S, LIVE_S, LIVE_S), "the pops left them in place");
        f.slots().add_writer();
        f.slots().add_writer();
        let top = f.slots().load_top();
        assert!(multi(top as u64) && !sanitizing(top), "the flip finished");
        assert_eq!((kind(state(2)), kind(state(3)), kind(state(4))), (TAKEN, FREE, FREE));
        assert_eq!(kind(state(5)), FREE, "a slot never written stays as it is");
        assert_eq!((get(&f, 0), get(&f, 1), get(&f, 2)), (Some(1), Some(2), None));
        for at in 2..5 {
            f.top().fetch_add_lo(1);
            assert_eq!(get(&f, at), None, "a reservation over slot {at} uncovers nothing");
        }
    }

    /// Slot `len`, LIVE in the current epoch, is a claim no writer counted,
    /// its pusher stopped before its raise: with two writers registered, the
    /// next push counts it and lands after it, and a pop counts it and takes
    /// it.
    #[test]
    fn a_claim_not_yet_counted_is_counted_by_the_next_writer() {
        for shape in both_shapes() {
            let f = Fixture::new(4, shape);
            f.slots().add_writer();
            f.slots().add_writer();
            assert_eq!(push(&f, 55), Some(0));
            assert_eq!(pop(&f), Some(55));
            mark(&f, 0, LIVE, 0);
            assert_eq!(push(&f, 66), Some(1), "{shape:?}: the push counted the claim first");
            assert_eq!((get(&f, 0), get(&f, 1)), (Some(55), Some(66)), "{shape:?}");
            assert_eq!(pop(&f), Some(66), "{shape:?}");
            assert_eq!(pop(&f), Some(55), "{shape:?}");
            mark(&f, 0, LIVE, 0);
            assert_eq!(pop(&f), Some(55), "{shape:?}: a pop on an empty vec counts and takes the claim");
            assert_eq!(pop(&f), None, "{shape:?}");
        }
    }

    /// A claim a clear left, LIVE in an earlier epoch, is claimed over by
    /// the next push at its slot, and no pop or read sees it.
    #[test]
    fn a_claim_a_clear_left_is_claimed_over_and_never_seen() {
        for shape in both_shapes() {
            let f = Fixture::new(4, shape);
            assert_eq!(push(&f, 1), Some(0));
            f.slots().clear();
            assert_eq!(pop(&f), None, "{shape:?}: the cleared claim is not counted");
            assert_eq!(push(&f, 2), Some(0), "{shape:?}: claimed over");
            assert_eq!((get(&f, 0), get(&f, 1), f.slots().len()), (Some(2), None, 1), "{shape:?}");
        }
    }

    /// A push whose raise lost: counted by another writer, it is done; its
    /// claim still its own and uncounted at the length, it counts the claim
    /// itself and is done; the length below its slot, it takes the claim back
    /// and goes again; its claim taken by a pop since, it is done; a clear
    /// since, it takes the claim back and goes again.
    #[test]
    fn a_push_whose_raise_lost_is_done_or_goes_again() {
        let mine = pair(9, stepped(FREE, LIVE, 0));
        let f = Fixture::new(4, Shape::Word);
        f.slots().word_slot(0).store(mine);
        assert_eq!(f.slots().after_lost_raise_word(raised(0), 0, mine), Raise::Done(0), "counted by another writer");

        let f = Fixture::new(4, Shape::Word);
        f.slots().word_slot(0).store(mine);
        assert_eq!(f.slots().after_lost_raise_word(0, 0, mine), Raise::Done(0), "uncounted at the length: counted by the push");
        assert_eq!((f.slots().len(), get(&f, 0)), (1, Some(9)), "the claim stands, counted");

        let f = Fixture::new(4, Shape::Word);
        f.slots().word_slot(1).store(mine);
        assert_eq!(f.slots().after_lost_raise_word(0, 1, mine), Raise::Again, "the length below it: taken back");
        assert_eq!(kind(f.slots().state(1)), FREE, "the claim came back");

        let f = Fixture::new(4, Shape::Word);
        f.slots().word_slot(0).store(pair(9, stepped(hi(mine), TAKEN, 0)));
        assert_eq!(f.slots().after_lost_raise_word(0, 0, mine), Raise::Done(0), "counted, then taken");

        let f = Fixture::new(4, Shape::Word);
        f.slots().word_slot(0).store(mine);
        assert_eq!(f.slots().after_lost_raise_word(cleared(0), 0, mine), Raise::Again, "a clear since: taken back");
    }

    /// A slot below the length that holds no element is a hole: a read
    /// answers None for it, a set refuses it, and a pop steps past it to the
    /// element below.
    #[test]
    fn a_hole_reads_as_none_and_a_pop_steps_past_it() {
        for shape in both_shapes() {
            let f = Fixture::new(4, shape);
            for v in 1..=3 {
                push(&f, v);
            }
            mark(&f, 1, TAKEN, 0);
            assert_eq!(get(&f, 1), None, "{shape:?}: a hole reads as None");
            assert!(!set(&f, 1, 9), "{shape:?}: a set refuses a hole");
            assert_eq!(f.slots().len(), 3, "{shape:?}");
            assert_eq!(pop(&f), Some(3), "{shape:?}");
            assert_eq!(pop(&f), Some(1), "{shape:?}: the pop stepped past the hole");
            assert_eq!((f.slots().len(), pop(&f)), (0, None), "{shape:?}");
        }
    }

    /// A set refuses an index past the length.
    #[test]
    fn a_set_past_the_length_is_refused() {
        for shape in both_shapes() {
            let f = Fixture::new(4, shape);
            push(&f, 1);
            push(&f, 2);
            assert_eq!(pop(&f), Some(2));
            assert!(!set(&f, 1, 9), "{shape:?}");
            assert!(set(&f, 0, 9), "{shape:?}");
        }
    }

    /// `pushers` pushers push distinct values, and `poppers` poppers pop
    /// until `pops` values have come out, on a vec of `capacity` slots of
    /// `shape`; then the vec is drained. Every value a push answered for comes
    /// out exactly once, and none comes out that no push answered for. A
    /// push the full vec refuses, or a pop that finds it empty, yields its
    /// thread's turn, so the workers keep the values moving when they
    /// outnumber the CPUs. A watchdog fails the test, rather than hanging it,
    /// when the pops stop coming for 30 s.
    fn race(shape: Shape, capacity: usize, pushers: u64, poppers: usize, pops: u64) {
        let f = Fixture::new(capacity, shape);
        let popped = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let push_handles: Vec<_> = (0..pushers)
            .map(|t| {
                let (f, stop, done) = (f.clone(), stop.clone(), done_tx.clone());
                std::thread::spawn(move || {
                    f.slots().add_writer();
                    let mut mine = Vec::new();
                    let mut i = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        let v = ((t + 1) << 40) | i;
                        if push(&f, v).is_some() {
                            mine.push(v);
                        } else {
                            std::thread::yield_now();
                        }
                        i += 1;
                    }
                    done.send(()).unwrap();
                    mine
                })
            })
            .collect();
        let pop_handles: Vec<_> = (0..poppers)
            .map(|_| {
                let (f, popped, done) = (f.clone(), popped.clone(), done_tx.clone());
                std::thread::spawn(move || {
                    f.slots().add_writer();
                    let mut got = Vec::new();
                    while popped.load(Ordering::Relaxed) < pops {
                        if let Some(v) = pop(&f) {
                            got.push(v);
                            popped.fetch_add(1, Ordering::Relaxed);
                        } else {
                            std::thread::yield_now();
                        }
                    }
                    done.send(()).unwrap();
                    got
                })
            })
            .collect();
        let mut seen = 0;
        let mut since = Instant::now();
        while popped.load(Ordering::Relaxed) < pops {
            std::thread::sleep(Duration::from_millis(20));
            let now = popped.load(Ordering::Relaxed);
            if now != seen {
                (seen, since) = (now, Instant::now());
            }
            assert!(
                since.elapsed() < Duration::from_secs(30),
                "{shape:?}, {poppers} poppers: pops stopped at {now} of {pops}, top word {:#x}",
                f.top().load(),
            );
        }
        stop.store(true, Ordering::Relaxed);
        for _ in 0..(pushers as usize + poppers) {
            done_rx.recv_timeout(Duration::from_secs(30)).expect("a worker did not finish within 30 s of the stop");
        }
        let mut pushed: Vec<u64> = push_handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
        let mut out: Vec<u64> = pop_handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
        while let Some(v) = pop(&f) {
            out.push(v);
        }
        assert_eq!(f.slots().len(), 0, "{shape:?}: a drained vec has no length left");
        compare(shape, poppers, &mut pushed, &mut out);
    }

    /// The values out against the values pushed, sorted: the counts of any
    /// repeated, lost or invented, and the first few of each.
    fn compare(shape: Shape, poppers: usize, pushed: &mut [u64], out: &mut [u64]) {
        pushed.sort_unstable();
        out.sort_unstable();
        let repeated: Vec<u64> = out.windows(2).filter(|w| w[0] == w[1]).map(|w| w[0]).collect();
        let lost: Vec<u64> = pushed.iter().copied().filter(|v| out.binary_search(v).is_err()).collect();
        let invented: Vec<u64> = out.iter().copied().filter(|v| pushed.binary_search(v).is_err()).collect();
        assert!(
            repeated.is_empty() && lost.is_empty() && invented.is_empty(),
            "{shape:?}, {poppers} poppers: {} repeated {:x?}, {} lost {:x?}, {} invented {:x?}",
            repeated.len(),
            &repeated[..repeated.len().min(8)],
            lost.len(),
            &lost[..lost.len().min(8)],
            invented.len(),
            &invented[..invented.len().min(8)],
        );
    }

    #[test]
    fn pushers_racing_one_popper_lose_and_repeat_nothing() {
        for shape in both_shapes() {
            race(shape, 8, 3, 1, 20_000);
        }
    }

    #[test]
    fn pushers_racing_two_and_three_poppers_lose_and_repeat_nothing() {
        for shape in both_shapes() {
            race(shape, 8, 3, 2, 20_000);
            race(shape, 8, 2, 3, 20_000);
        }
    }

    /// With two writers and room for a reservation from each, a word push
    /// reserves its slot by one add to the length and claims it: the indices
    /// come in order and every element reads back.
    #[test]
    fn a_multi_writer_push_reserves_its_slot_by_an_add() {
        let f = Fixture::new(64, Shape::Word);
        f.slots().add_writer();
        f.slots().add_writer();
        assert!(f.slots().reserves(f.slots().load_top()));
        for v in 0..10u64 {
            assert_eq!(push(&f, 100 + v), Some(v as usize));
        }
        let read: Vec<_> = (0..10).map(|i| get(&f, i)).collect();
        assert_eq!(read, (0..10u64).map(|v| Some(100 + v)).collect::<Vec<_>>());
        assert_eq!(pop(&f), Some(109));
        assert_eq!(f.slots().len(), 9);
    }

    /// A reservation raises the length over its slot before claiming it, so
    /// a slot below the length can hold an element a clear removed: a read
    /// answers None for it, a set refuses it, and a pop steps past it.
    #[test]
    fn an_element_a_clear_removed_stays_removed_under_a_reservation() {
        let f = Fixture::new(8, Shape::Word);
        f.slots().add_writer();
        f.slots().add_writer();
        for v in 1..=3 {
            push(&f, v);
        }
        f.slots().clear();
        f.top().fetch_add_lo(2);
        assert_eq!((get(&f, 0), get(&f, 1), f.slots().len()), (None, None, 2));
        assert!(!set(&f, 0, 9), "a set refuses an element the clear removed");
        assert_eq!(pop(&f), None, "both slots under the length hold elements the clear removed");
        assert_eq!(f.slots().len(), 0);
        assert_eq!(push(&f, 7), Some(0));
        assert_eq!(get(&f, 0), Some(7));
    }

    /// Adds that found the vec full can leave the length past the capacity:
    /// the length reads as the capacity, a push is refused, and pops step
    /// past the slots that are not there, and the hole, to the element below.
    #[test]
    fn a_length_past_the_capacity_reads_as_full_and_pops_step_back_under_it() {
        let f = Fixture::new(4, Shape::Word);
        f.slots().add_writer();
        f.slots().add_writer();
        for v in 1..=3 {
            push(&f, v);
        }
        f.top().fetch_add_lo(3);
        assert_eq!((f.slots().len(), get(&f, 3)), (4, None));
        assert_eq!(push(&f, 9), None, "a push past the capacity is refused");
        assert_eq!(pop(&f), Some(3));
        assert_eq!(f.slots().len(), 2);
    }

    /// Pushers racing poppers on a vec with room for every writer's
    /// reservation, so pushes reserve by adding to the length: every value
    /// still comes out once.
    #[test]
    fn reserving_pushers_racing_poppers_lose_and_repeat_nothing() {
        race(Shape::Word, 1 << 16, 3, 1, 20_000);
        race(Shape::Word, 1 << 16, 3, 2, 20_000);
        race(Shape::Word, 1 << 16, 2, 3, 20_000);
    }

    /// Reserving pushers and a popper racing a thread that clears the vec:
    /// a clear removes values, but no value comes out twice and none comes
    /// out that no push answered for. A pusher yields its thread's turn when
    /// refused and every 32 pushes, so the clearing thread wakes on time when
    /// the workers outnumber the CPUs.
    #[test]
    fn reserving_pushers_racing_clears_repeat_and_invent_nothing() {
        let f = Fixture::new(1 << 12, Shape::Word);
        let stop = Arc::new(AtomicBool::new(false));
        let pushers: Vec<_> = (0..3u64)
            .map(|t| {
                let (f, stop) = (f.clone(), stop.clone());
                std::thread::spawn(move || {
                    f.slots().add_writer();
                    let mut mine = Vec::new();
                    let mut i = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        let v = ((t + 1) << 40) | i;
                        let landed = push(&f, v).is_some();
                        if landed {
                            mine.push(v);
                        }
                        if !landed || i % 32 == 31 {
                            std::thread::yield_now();
                        }
                        i += 1;
                    }
                    mine
                })
            })
            .collect();
        let popper = {
            let (f, stop) = (f.clone(), stop.clone());
            std::thread::spawn(move || {
                f.slots().add_writer();
                let mut got = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    match pop(&f) {
                        Some(v) => {
                            got.push(v);
                            if got.len() % 32 == 0 {
                                std::thread::yield_now();
                            }
                        }
                        None => std::thread::yield_now(),
                    }
                }
                got
            })
        };
        for _ in 0..200 {
            std::thread::sleep(Duration::from_micros(500));
            f.slots().clear();
        }
        stop.store(true, Ordering::Relaxed);
        let mut pushed: Vec<u64> = pushers.into_iter().flat_map(|h| h.join().unwrap()).collect();
        let mut out = popper.join().unwrap();
        while let Some(v) = pop(&f) {
            out.push(v);
        }
        pushed.sort_unstable();
        out.sort_unstable();
        let repeated: Vec<u64> = out.windows(2).filter(|w| w[0] == w[1]).map(|w| w[0]).collect();
        let invented: Vec<u64> = out.iter().copied().filter(|v| pushed.binary_search(v).is_err()).collect();
        assert!(
            repeated.is_empty() && invented.is_empty(),
            "{} repeated {:x?}, {} invented {:x?}",
            repeated.len(),
            &repeated[..repeated.len().min(8)],
            invented.len(),
            &invented[..invented.len().min(8)],
        );
    }

    /// Every thread both pushes and pops, a pop after every third push: every
    /// value a push answered for comes out exactly once.
    #[test]
    fn threads_that_push_and_pop_both_lose_and_repeat_nothing() {
        for shape in both_shapes() {
            let f = Fixture::new(8, shape);
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let handles: Vec<_> = (0..4u64)
                .map(|t| {
                    let (f, done) = (f.clone(), done_tx.clone());
                    std::thread::spawn(move || {
                        f.slots().add_writer();
                        let (mut pushed, mut popped) = (Vec::new(), Vec::new());
                        for i in 0..20_000u64 {
                            let v = ((t + 1) << 40) | i;
                            if push(&f, v).is_some() {
                                pushed.push(v);
                            }
                            if i % 3 == 0
                                && let Some(v) = pop(&f)
                            {
                                popped.push(v);
                            }
                        }
                        done.send(()).unwrap();
                        (pushed, popped)
                    })
                })
                .collect();
            for _ in 0..4 {
                done_rx.recv_timeout(Duration::from_secs(60)).expect("a thread did not finish in 60 s");
            }
            let (mut pushed, mut out) = (Vec::new(), Vec::new());
            for h in handles {
                let (p, o) = h.join().unwrap();
                pushed.extend(p);
                out.extend(o);
            }
            while let Some(v) = pop(&f) {
                out.push(v);
            }
            compare(shape, 4, &mut pushed, &mut out);
        }
    }
}
