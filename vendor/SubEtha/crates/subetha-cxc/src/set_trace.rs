//! A test-build trace of what writers, carriers and passes did to the slots
//! of the set tables a test names, so a failure can say what happened to a
//! key and in which order.
//!
//! An event is six words a thread fills after taking its place with one
//! `fetch_add`, so recording takes no lock and waits on no thread: the
//! windows this records are a few instructions wide, and an instrument that
//! synchronized threads would close them. A table records only once a test
//! has named it, so the other tests running beside it add nothing.

#![cfg(test)]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Events kept. A move of a few hundred keys by four carriers emits a few
/// thousand, so this covers a whole test of twenty rounds.
const CAPACITY: usize = 1 << 20;
const WORDS: usize = 6;

static EVENTS: OnceLock<Box<[AtomicU64]>> = OnceLock::new();
static NEXT: AtomicUsize = AtomicUsize::new(0);
static TABLES: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static THREADS: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static THREAD: u64 = THREADS.fetch_add(1, Ordering::Relaxed);
}

/// What a recorded event was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum What {
    /// A writer's probe ended: `word` is the slot it would take or revive,
    /// `u64::MAX` for none, and `extra` is 1 when it found the key.
    Probed,
    /// A writer claimed the slot; `word` is what it held before.
    Claimed,
    /// A writer's claim of the slot lost to another writer.
    ClaimLost,
    /// A writer took back the slot's stale copy of its key.
    Revived,
    /// A writer's take-back lost to another writer.
    ReviveLost,
    /// A writer published its claim.
    Published,
    /// A writer's publish failed: its claim had been given up.
    PublishLost,
    /// A writer gave up another's claim of its fingerprint.
    GaveUp,
    /// A writer found a published copy where it went to give a claim up;
    /// `extra` is 1 when the copy is its key.
    FoundCopy,
    /// A writer freed its own claim.
    Freed,
    /// A writer's published copy stood down for an earlier one.
    StoodDown,
    /// A writer counted its key added.
    Counted,
    /// A writer found its key present and added nothing.
    Present,
    /// A writer found the set moved and added nothing.
    Moved,
    /// A carrier took the slot at its cursor; `word` is what it held.
    CursorClaimed,
    /// A carrier stopped taking slots at its cursor; `word` is what the slot
    /// held, `u64::MAX` when the cursor had already stopped.
    CursorStopped,
    /// A pass gave up a claim of the sealed round.
    PassGaveUp,
    /// A pass retired a stale published key.
    PassRetired,
    /// A pass handed a key published in the sealed round to its caller.
    PassCopied,
    /// A writer raised the hint; `word` is the slot it raised it to.
    HintRaised,
    /// A cache put looked for its key's committed entry; `slot` is the
    /// bucket word holding it, `usize::MAX` for none.
    CacheLooked,
    /// A cache put published its pending entry (`word`) in bucket word
    /// `slot`.
    CacheEntered,
    /// A cache put's entry in bucket word `slot` is committed: an insert;
    /// `extra` is 1 when another writer committed it.
    CacheCommitted,
    /// A cache put found a committed entry of its key in bucket word `slot`.
    CacheBeaten,
    /// A cache put gave up its own pending entry in bucket word `slot`.
    CacheGaveOwnUp,
    /// A cache put gave up another writer's pending entry of its key in
    /// bucket word `slot`.
    CacheGaveUp,
    /// A cache put committed another writer's pending entry of its key in
    /// bucket word `slot`, which won.
    CacheHelped,
    /// A cache put took its own entry out of bucket word `slot`.
    CacheWithdrew,
    /// A cache put replaced the entry in bucket word `slot`, `word` the
    /// entry it replaced: an update.
    CacheReplaced,
    /// A cache put whose entry lost moved its frame on; `word` is the frame's
    /// new word.
    CacheStepped,
    /// A cache put making room took back bucket word `slot`, which held
    /// `word`; `extra` is 1 for a given-up entry whose frame it marked, 0 for
    /// an entry whose frame had moved on.
    CacheTookBack,
    /// A cache put making room evicted the committed entry `word` in bucket
    /// word `slot`.
    CacheEvicted,
    /// A cache put making room settled the pending entry `word` in bucket
    /// word `slot` for its writer; `extra` is what the word held after.
    CacheSettledFor,
    /// A list push linked node `slot`, `key` its handle; `word` names the
    /// node it went before, `extra` is 0 at the front and 1 at the back.
    ListLinked,
    /// A list removal marked node `slot`, `word` its `next` high half;
    /// `extra` is 0 for a pop from the front, 1 from the back, 2 by handle.
    ListMarked,
    /// A walk swapped marked node `slot` out for the writer that marked it.
    ListHelped,
    /// Node `slot` was freed, `word` its `next` high half before.
    ListFreed,
    /// A sweep freed node `slot`, left by a dead process.
    ListSwept,
}

const ALL: [What; 38] = [
    What::Probed,
    What::Claimed,
    What::ClaimLost,
    What::Revived,
    What::ReviveLost,
    What::Published,
    What::PublishLost,
    What::GaveUp,
    What::FoundCopy,
    What::Freed,
    What::StoodDown,
    What::Counted,
    What::Present,
    What::Moved,
    What::CursorClaimed,
    What::CursorStopped,
    What::PassGaveUp,
    What::PassRetired,
    What::PassCopied,
    What::HintRaised,
    What::CacheLooked,
    What::CacheEntered,
    What::CacheCommitted,
    What::CacheBeaten,
    What::CacheGaveOwnUp,
    What::CacheGaveUp,
    What::CacheHelped,
    What::CacheWithdrew,
    What::CacheReplaced,
    What::CacheStepped,
    What::CacheTookBack,
    What::CacheEvicted,
    What::CacheSettledFor,
    What::ListLinked,
    What::ListMarked,
    What::ListHelped,
    What::ListFreed,
    What::ListSwept,
];

/// One recorded event.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Event {
    pub(crate) what: What,
    pub(crate) thread: u64,
    pub(crate) table: usize,
    pub(crate) slot: usize,
    pub(crate) round: u32,
    /// The key's first eight bytes, little-endian.
    pub(crate) key: u64,
    pub(crate) word: u64,
    pub(crate) extra: u64,
}

fn events() -> &'static [AtomicU64] {
    EVENTS.get_or_init(|| (0..CAPACITY * WORDS).map(|_| AtomicU64::new(0)).collect())
}

/// Start a trace of the tables at `bases`, dropping every event recorded so
/// far. Tables named before are no longer recorded.
pub(crate) fn name(bases: &[usize]) {
    assert!(bases.len() <= TABLES.len(), "a trace names at most {} tables", TABLES.len());
    for (i, table) in TABLES.iter().enumerate() {
        table.store(bases.get(i).copied().unwrap_or(0), Ordering::Release);
    }
    NEXT.store(0, Ordering::Release);
}

/// Which named table `base` is, if any.
#[inline]
fn named(base: usize) -> Option<usize> {
    TABLES.iter().position(|t| t.load(Ordering::Relaxed) == base)
}

/// The first eight bytes of `key`, little-endian, zero-filled.
pub(crate) fn key_word(key: &[u8]) -> u64 {
    let mut bytes = [0u8; 8];
    let n = key.len().min(8);
    bytes[..n].copy_from_slice(&key[..n]);
    u64::from_le_bytes(bytes)
}

/// Record an event on the table at `base`, when a test named it.
pub(crate) fn record(base: usize, what: What, slot: usize, round: u32, key: u64, word: u64, extra: u64) {
    let Some(table) = named(base) else { return };
    let at = NEXT.fetch_add(1, Ordering::AcqRel);
    if at >= CAPACITY {
        return;
    }
    let thread = THREAD.with(|t| *t);
    let e = &events()[at * WORDS..(at + 1) * WORDS];
    let meta = (what as u64) | (thread << 8) | ((table as u64) << 24);
    e[1].store(slot as u64, Ordering::Relaxed);
    e[2].store(u64::from(round), Ordering::Relaxed);
    e[3].store(key, Ordering::Relaxed);
    e[4].store(word, Ordering::Relaxed);
    e[5].store(extra, Ordering::Relaxed);
    e[0].store(meta | (1 << 63), Ordering::Release);
}

/// The events recorded since the trace was started, in the order their
/// places were taken, and how many were dropped past the capacity.
pub(crate) fn recorded() -> (Vec<Event>, usize) {
    let taken = NEXT.load(Ordering::Acquire);
    let kept = taken.min(CAPACITY);
    let words = events();
    let mut out = Vec::with_capacity(kept);
    for at in 0..kept {
        let e = &words[at * WORDS..(at + 1) * WORDS];
        let meta = e[0].load(Ordering::Acquire);
        if meta & (1 << 63) == 0 {
            continue;
        }
        out.push(Event {
            what: ALL[(meta & 0xFF) as usize],
            thread: (meta >> 8) & 0xFFFF,
            table: ((meta >> 24) & 0xFF) as usize,
            slot: e[1].load(Ordering::Relaxed) as usize,
            round: e[2].load(Ordering::Relaxed) as u32,
            key: e[3].load(Ordering::Relaxed),
            word: e[4].load(Ordering::Relaxed),
            extra: e[5].load(Ordering::Relaxed),
        });
    }
    (out, taken - kept)
}

/// The recorded events about `key` or about a slot one of them touched in
/// that table, one per line, numbered in recording order.
pub(crate) fn story(key: u64) -> String {
    let (events, dropped) = recorded();
    // An event with no slot, or a hint raise, whose slot field holds where
    // the hint went, ties no slot to the key.
    let slots: Vec<(usize, usize)> = events
        .iter()
        .filter(|e| e.key == key && e.slot != usize::MAX && e.what != What::HintRaised)
        .map(|e| (e.table, e.slot))
        .collect();
    let mut out = String::new();
    for (n, e) in events.iter().enumerate() {
        let on_slot = e.slot != usize::MAX && e.what != What::HintRaised && slots.contains(&(e.table, e.slot));
        if e.key == key || on_slot {
            out.push_str(&format!(
                "  #{n} t{} table{} slot {} round {} {:?} key {} word {:#x} extra {}\n",
                e.thread, e.table, e.slot, e.round, e.what, e.key, e.word, e.extra
            ));
        }
    }
    if dropped > 0 {
        out.push_str(&format!("  ({dropped} events past the capacity were not kept)\n"));
    }
    out
}
