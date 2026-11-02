# Changelog

All notable changes to SubEtha are recorded here. The six published
crates (`subetha`, `subetha-core`, `subetha-cxc`, `subetha-ffi`,
`subetha-pointers`, `subetha-sidecar`) share one version number and
release together. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Each version
heading links to the commit that cut it.

Each version after 0.6.0 groups its changes by where they show. Every
surface holds a change in the shared core that Rust, Python, PowerShell
and the C ABI all see. Rust, Python, PowerShell and C ABI each hold what
changed for that surface, even when the cause is in the core, so a
reader of one surface finds everything that touches it in one place.
Documentation holds changes to the documentation alone. Within each,
changes are listed as Breaking, Added, Changed, Removed and Fixed. 0.6.0
and earlier list their changes by kind alone.

## [Unreleased]

### Every surface

#### Changed

- A peer directory claims a free producer or consumer slot by or-ing its
  bit into the bitmap word, where it swapped the whole word: another
  slot's bit changing in the same word no longer fails the claim and sends
  it round again. The `peer_directory` bench times claim and release by
  one thread and by four.
- A refusal of a file of another shape names the first field that
  differs, the value the file holds and the value asked for: "the file's
  capacity is 16 where 32 was asked for", "the file is 4096 bytes where
  8192 were asked for" for a file shorter than the layout, and a magic in
  hexadecimal. Rust's `Mismatch`, the C detail, Python's `LayoutMismatch`
  and the PowerShell module's errors print the same text.
- On Windows a timed wait ends at its timeout rather than at the system
  timer's next tick, on every structure that waits through
  `CrossProcessWaker` and in every surface. Past the monitor budget a
  timed waiter sleeps on its slot's event beside a high-resolution
  waitable timer; on an anonymous waker, where it slept in
  `WaitOnAddress`, the event is unnamed and made on the slot's first
  timed wait. On a Windows 11 test host a blocking receive with a 2 ms
  timeout took 12.8 ms and takes 2.4 ms, and one of 20 ms took 29.8 ms
  and takes 20.3 ms, with no measurable processor time either way.
  Windows before 10 1803 has no high-resolution timer and keeps the
  tick, which is reported once per process.
- The TCP, TCP+TLS and QUIC bridges carry each item with its kind, so
  the receiving ring's frame and any-receive calls see a plain item or a
  frame as it was sent. A plain item or an inline frame is one 64-byte
  record with its kind in the last byte. An offset frame crosses as its
  payload: a header record holding its length, then the payload in whole
  records, which the server sends into its own ring as an offset frame in
  that ring's own payload region; its descriptor never crosses, since it
  names a block in the sending host's region. A frame longer than the
  receiving ring's payload blocks ends the server before its payload is
  read, and a receiving ring's refusal other than full ends the server
  with that refusal, where the server retried every refusal forever. The
  wire changes with this, so both ends of a bridge take the release
  together.
- An adaptive ring's peer directory records each producer's and
  consumer's process start time beside its pid, so a peer whose pid the
  system has handed to another process is reaped and its rings taken
  over, where they stayed with the stale owner until that process exited.
  A word an earlier SubEtha wrote, a pid alone, is still judged by the
  pid.
- The LRU cache records, beside each frame's word, the pid and process
  start time of the process that filled it, and a claim of a frame another
  process filled swaps both with one 16-byte compare-and-swap. Eviction
  frees a frame whose writer's pid now names a process that started later,
  where it kept the frame until that process exited, and a process never
  treats a frame an earlier holder of its pid left as its own. A frame's
  header is 32 bytes and its stride a multiple of 16. Creating or opening
  a cache fails as unsupported on a processor without a lock-free 16-byte
  compare-and-swap.
- The linked list records the pid and process start time of each node's
  owner, of each value frame's taker and of each registered process, so a
  sweep frees what a dead process left, and its registry entry, once the
  system has handed its pid to another process, and a list that finds a
  newly registered process already gone that way owes the sweep at once.
  A value frame carries the taker's word before its value, 8 bytes more.
- A leader election claim by the current leader's own id that finds the
  leader's heartbeat stale steps the election term and refreshes the
  heartbeat, as a takeover does, so a process that drew a dead leader's
  pid, or a leader that lapsed past the grace, shows as a handover to
  anyone watching the term. A fresh self-claim still leaves the term.
- The LRU cache is one file, `<base>.cache.bin`, that any number of
  processes write at once, none of them waiting on another, where 0.6.0's
  cache kept `<base>.map.bin` and a list region and its puts, touches and
  removes took one writer at a time. It evicts by CLOCK: `put`, `touch`
  and `get_and_touch` mark an entry and `get` leaves its mark as it was;
  eviction's hand passes a marked entry once and takes the first unmarked
  one, so eviction follows recency approximately rather than exactly.
  `evict_oldest` takes the entry the hand reaches first unmarked, and
  `snapshot_lru_first` lists entries in the order eviction would take
  them. The file is format 3; its header counts the frames puts have
  used, and the hand walks only those. A cache's 0.6.0 files are not read.
- The linked list takes pushes, pops, removals and sets from any number
  of threads and processes at once, none of them waiting on another,
  live or dead, where 0.6.0's took one writer at a time. A node's handle
  carries its generation beside its slot, so a handle used after its node
  was removed or popped is refused rather than naming whatever node took
  the slot. `len` walks the list and `is_empty` reads its head. A handle
  holding a registry entry frees the nodes it removes by plain stores, its
  entry named in each one's mark; a sweep finishes the removals of a handle
  whose process died. A list holds fewer than 536,870,848 nodes, and a
  larger capacity is refused as an invalid argument. The list's file is
  format 2; a list file an earlier SubEtha made is refused as a layout
  mismatch.
- The strategy-switching set (`SharedUniversal`, `RawUniversal`) is a set
  as a list and as a map: an insert reports whether the value was new,
  and the set holds each value once. Any number of processes insert and
  migrate at once, none of them waiting on another, live or dead, where
  0.6.0's took one writer. It keeps its values in two set-table files,
  one per strategy, reused from one migration to the next, in format 3.
- Creating a structure never waits on another process. Most file-backed
  creates lay out their region in a file of their own under a staging
  name and link it into place; of creators racing on one path, one link
  lands and the rest attach to the region it published, so a creator
  that stops part way through holds nobody up. A region laid out where
  it lies, a named shared-memory region or the file of a frame region or
  a shared holder table, is laid out by swapping each header word in
  from zero, the magic last, so racing creators finish one layout
  together. A file at
  the path that no layout reads is refused at once as invalid data,
  where create waited out a deadline for its layout to finish and then
  returned a timed-out error.
- The once cell, and the lazy value built on it, never wait on the
  caller producing the value. A caller that finds the cell empty copies
  its value into one of eight slots and publishes it with one swap of
  the cell's winner word; a fetch that must run once at a time runs
  under the cell's one fetch claim, and the call returns at once for any
  other caller. A claim, or a slot mid-copy, whose process is gone is
  taken over by the next caller. The lazy value's wait and reclaim calls
  are gone from every surface. The cell's file is format 4: a once cell
  or lazy value file from an earlier SubEtha is refused as a layout
  mismatch.
- A stack, region, B-tree or list records that its slot counter is
  spent, at the first allocation the counter refuses, where its
  allocation already looks: in the free-list head, as the index one below
  the empty mark. An allocation that finds the record is refused with one
  read and no locked instruction. A slot freed onto the head's record
  takes it as its link, so the record is back once the free list empties
  again. A fresh allocation reads only the head before its add.
- The vec takes pushes, pops, sets and clears from any number of threads
  and processes at once, none of them waiting on another, live or dead,
  and no pop erases an element a push answered for. A 16-byte top word in
  its header holds the length, a version every pop and clear steps, an
  epoch every clear steps and the count of threads writing. With one
  writer a push claims the slot at the length and raises it, and a pop
  lowers the length alone. With several, a push takes its slot by one add
  to the length, or, near the capacity and for an element over 8 bytes,
  claims the slot and raises the length after; a pop takes the last slot
  by one compare-and-swap, then lowers the length. A second writer's
  first write moves the vec to the several-writer protocol, and the
  writer left moves it back. A pop whose lowering loses to a push leaves
  a hole below the length, which a get answers as absent, a set refuses
  and the next pop steps past. A full vec refuses a push with one read of
  the top word, and the length never reads above the capacity. An element
  of at most 8 bytes at an alignment of at most 8 sits in one 64-byte
  line, its word beside a state word one 16-byte compare-and-swap claims
  and publishes; a larger one keeps two buffers and a head. A writer
  table after the slots, sized at create, holds one entry per handle
  writing the vec at once: its process's pid and start time beside how
  many of its threads write, so a writer gives back the count of a
  process that died. Creating or opening a vec takes the table's size,
  and a file with another is refused as a layout mismatch, as is a vec
  file an earlier SubEtha made. A vec needs a lock-free 16-byte
  compare-and-swap, and a read-only open of word slots a processor that
  reads 16 bytes whole without one; where either is missing, creating or
  opening fails as unsupported.

#### Removed

- The locks, on every surface: the shared and blocking reader-writer
  locks, the shared and blocking semaphores and the owner lease. A role
  one process holds at a time is claimed through the leader election.

#### Fixed

- A ring or region in named shared memory that its filesystem cannot hold
  now fails at creation with ENOSPC ("No space left on device") instead of
  killing the process with SIGBUS at its first write. On Linux creation
  reserves the object's pages (`posix_fallocate`) before mapping it, so a
  full `/dev/shm`, such as a container's default 64 MiB, refuses the ring
  and leaves no name behind. On FreeBSD an object's swap reservation is
  taken when it is sized and refused (ENOMEM) only on a host that
  enforces swap reservation (`vm.overcommit` bit 0 set); a default
  FreeBSD host backs the object as it is touched. macOS shared memory
  takes no reservation and stays as it was.
- A consumer given back on an adaptive or locale ring now wakes the
  consumers that take over its rings, so one waiting on its own empty
  rings receives the backlog it was handed. A hand-off that left the
  ring's shape as it was woke no one, and the waiter slept until the next
  send. A consumer id past the consumer table now names no consumer: it
  was cut to 16 bits, so 65536 + n handed consumer n's rings away while n
  stayed registered. The C ABI's ring and locale ring refuse a producer
  or consumer id past their tables with `SUBETHA_E_INVALID_ARGUMENT`.
- A process that dies between writing a wait-calibration record and
  renaming it into place no longer leaves its temporary file in the
  per-user cache for good: the next write of that record removes each
  temporary file of it whose writer is gone, judged by pid and process
  start time, which the temporary names now carry.
- A C program waiting on a broadcast, capacity or locale ring, a deque or
  a stack now wakes on a push, receive, steal or pop made from Python or
  PowerShell. The C ABI keeps each structure's two wakers beside it, but
  only C calls woke them, so `subetha_broadcast_recv_wait`,
  `subetha_capacity_pop_wait`, `subetha_locale_ring_pop_wait`,
  `subetha_deque_steal_wait` and `subetha_stack_pop_wait` slept through a
  Python or PowerShell push until their timeout, and the C push waits
  through their receives, steals and pops. Every surface now goes through
  `BlockingBroadcastRing`, `BlockingCapacityRing`, `BlockingLocaleRing`,
  `BlockingDeque` and `BlockingStack`, and giving a broadcast consumer back
  wakes a producer waiting for room from every surface too.
- A morph or a migration now wakes every waiter on the ring. A capacity
  morph starts an empty backing, so a producer waiting for room on a full
  ring has it at once, and a locale migration into or out of the
  anonymous locale, where each handle's backing is its own, changes what
  a waiter in another process finds. No move woke anyone, so a waiter
  slept through it until its timeout or the next push or receive, and one
  with no timeout could sleep for good. The wake comes whether the
  caller, a managed ring's sidecar or a registration made the move; an
  adaptive ring's shape morph and a capacity broadcast ring's morph wake
  their waiters the same way.
- An adaptive ring keeps each producer's items in order across a shape
  morph. When a producer registered or unregistered mid-stream, another
  producer's item could land in the shape the ring was leaving just after
  the consumer had looked there, and its next item in the new shape was
  taken first; frames sent from two producers arrived out of order in
  about one run in five. A producer now keeps pushing into its lane in
  the old shape until the consumer has emptied it, and a producer waiting
  for room waits for its own lane.
- A consumer now finds items pushed through another handle on the same
  ring in a shape its own handle never used. A shape was walked only by
  handles that had been in it, so a handle moved with `morph_to` and
  another pinned with `pin_shape` filled a backing the consumer's handle
  never read, and its waiting receive slept for good. The shapes any
  handle has used are now kept in the ring's peer directory, in a word
  its layout already reserved.
- An ordered receiver's pops now wake a sender waiting for room in its
  ring: the C ABI's `subetha_ordered_try_next` and
  `subetha_ordered_next_wait`, Python's `OrderedReceiver` and
  PowerShell's `SubEtha.OrderedReceiver`. They popped through the ring
  alone, so a sender blocked in `subetha_ring_push_wait`, `Ring.send_for`
  or `SendFor` on a full ring slept until its timeout even after the
  receiver freed a slot. An item the reorder window takes and holds
  frees its slot too, and wakes the same way.
- A blocking wait could sleep for good under load, on every structure
  that waits through `CrossProcessWaker` and in every surface. A waiter
  leaving its slot freed the slot before clearing its bit in the parked
  mask, so a waiter that took the slot in between had its bit erased, and
  no wake scan, which reads the mask, could find it again; a C test of the
  stack hung this way three times in about a hundred and eighty runs on a
  loaded Windows host. The bit is now cleared while the slot is still the
  leaving waiter's. A wake that lands after a timed wait's last check of
  its deadline now ends the wait with success rather than a timeout, so
  the item that wake announces is not left with every other waiter still
  asleep.
- Making an adaptive ring over one another handle already holds attaches
  to it, as the C ABI's `subetha_ring_create` and PowerShell's
  `New-SubEthaRing` say it does. Every backing attached except the Vyukov
  ring's file, which was made afresh: on Windows the second create failed
  with "The requested operation cannot be performed on a file with a
  user-mapped section open" while the first handle lived, and on Unix it
  laid the file out again under the live ring, discarding any item in
  it.
- A C consumer waiting in `subetha_ring_pop_wait` on a file-backed or
  shared-memory ring wakes on a push from Python or PowerShell, and a C
  producer waiting in `subetha_ring_push_wait` on their pops. Only the C
  ABI woke the ring's wakers, so a waiter slept through every other
  surface's push until its timeout, returning `SUBETHA_E_TIMEOUT` with
  the item still in the ring, or for good under `SUBETHA_WAIT_FOREVER`.
- On Windows a file-backed ring's notifiers are reached through any path
  that names the ring's file. Their events were named from the path as
  spelled, so a producer that spelled the file another way (relative
  where the consumer's was absolute, as the PowerShell module always
  resolves it, in another case, or with a `.` component) signaled
  nothing while `attached()` counted the consumer. The events are named
  from the notifier record's volume serial number and file id. A process
  on 0.6.0 and one on this version name them differently, and on Windows
  they do not signal each other's notifiers.
- `PeriodicitySensor::observe`, which every surface's periodicity sensor
  calls, closed one 500-millisecond bin per loop pass up to the new time:
  a jump of 10^14 microseconds blocked one call for 358 ms in a release
  build and 3.5 s in a debug build, and a time near `u64::MAX` hung a
  release build and panicked a debug one. It closes at most one more bin
  than the window holds and moves the bin start to the bin holding the
  new time, which leaves exactly what closing every bin did, and a bin
  ending past `u64::MAX` never closes.
- An `AdaptiveIpc` endpoint, under Python's `AdaptiveQueue` and
  PowerShell's `New-SubEthaAdaptiveQueue`, removes its ring's peer
  directory when it drops; it left `<stem>.ring.peers.bin` behind, and
  removed only the per-producer files its creation hint named.
- On Windows a timed wait on an anonymous waker whose remaining time fell
  under a millisecond spun until its deadline instead of sleeping. The
  remaining time was cut to whole milliseconds, so `WaitOnAddress` was
  asked for zero and returned at once, and the wait ran its monitor
  phase again and again: a thousand blocking receives with a 1 ms timeout
  held two thirds to all of a core on a Windows 11 test host. The wait
  now sleeps the rest on its event and timer without running the monitor
  phase again, and where it still sleeps in `WaitOnAddress` the remaining
  time is rounded up to a whole millisecond.
- On a virtual machine that reports WAITPKG without enabling it for the
  guest, the first wait no longer ends the process with an
  illegal-instruction fault. WAITPKG is used only where `UMONITOR`,
  `UMWAIT` and `TPAUSE` also run in a one-time trial, in a forked child on
  Unix and under a vectored exception handler on Windows; where they
  fault, waits use the host's other monitor family, or none.
- On a processor whose `UMWAIT` does not sleep, as Sapphire Rapids,
  Emerald Rapids, Sierra Forest and Granite Rapids leave it under their
  default IA32_MCU_OPT_CTRL.IGN_UMONITOR, a wait's monitor phase no
  longer spins out its budget. It pauses in `TPAUSE` slices, each as long
  as the phase has run, and checks the value between them, so a store is
  seen within the time the phase has already waited. Calibration measures
  `TPAUSE` beside `UMWAIT` and uses the slices only where no monitor
  holds; before calibration, the fixed ladder takes them where waits at
  first use show `UMWAIT` returning early and `TPAUSE` sleeping. The URD
  deque's thief pauses the same way there.
- A process's calibrated waits now start with its first wait. Each load
  reading records its band in the host's calibration cache
  (`wait-load-band.txt`), a new process takes that band at once, and its
  first reading of its own comes after 100 ms rather than a full second.
  Before, every wait in a process's first second used the fixed ladder
  whatever the cache held, so a process that lived under a second never
  waited calibrated.
- Creating a structure over a file of another layout, such as one an
  earlier SubEtha built, is refused as opening it is, naming the magic
  the file holds and the one asked for: the structure's `LayoutMismatch`
  in Rust, `subetha.LayoutMismatch` in Python, an `OpenError` record in
  PowerShell, and `SUBETHA_E_LAYOUT_MISMATCH` in C
  (`SUBETHA_E_RING_WAKER_LAYOUT` for a ring's waker region). Create built
  a region it then threw away and returned a timed-out I/O error
  saying the file was empty or unfinished and should be removed, the
  wrong cause and a fix that deletes a file the other layout's processes
  may still be using.

### Rust

#### Breaking

- `SharedCell` and `RawCell` take any number of writers and readers at
  once, none waiting on another, where they took one writer under a
  version a reader spun on while it was odd. A write fills a buffer that
  is not current and makes it current with one compare-and-swap of the
  head, whose low bits say where the current value lies and whose high
  bits count the publishes. The first thread to write through a handle
  owns a pair of buffers and alternates within it; other writers claim a
  buffer per write, recording its pid and process start time in the
  buffer's claim word, and a writer that dies mid-write costs one buffer,
  which the next writer takes over, even once the system has handed the
  dead writer's pid to another process. A read copies the value the head
  names and retries only when a write landed meanwhile; each handle keeps
  a copy of the last value it read, up to 56 bytes, and returns it while
  the head has not moved. The file is 4,096 bytes (`CELL_FILE_SIZE`, magic
  `CEL5`), so a cell file from an earlier SubEtha is refused as a
  `LayoutMismatch`, and a value may be 56 bytes where it could be 52.
  `version()` is a `u64` counting the writes that landed, one per write,
  where it was a `u32` stepping twice; `set` returns the generation it
  made.
- `subetha_cxc::blocking_spsc_ring::BlockingError` gains `Notify`, for a
  notify record that could not be made or attached to, and `Closed`, for
  a wait ended by closing its handle; a `match` over it needs the two
  arms.
- `SharedUniversal` keeps its values in set tables of its own, one file
  per strategy, where 0.6.0 kept them in a `SharedVec` or a
  `SharedHashMap`: `UniversalError` loses `VecError` and `MapError`, and
  gains `MigrationRaced { published }`, which a `migrate_to` that loses
  the publish to another process's migration returns with the state word
  that stands.
- The 43 `subetha-cxc` error types whose I/O variant held only an
  `std::io::ErrorKind` (`MapError::IoError`, `RingError::IoError`,
  `CondvarError::Io` and the rest) hold the
  `std::io::Error` itself: `Display` carries the operating system's
  message, `source()` returns the error, and `raw_os_error()` gives its
  code, where a missing file read only `io: entity not found`. These
  types, the errors that wrap them (`BarrierError`, `LinkedListError`
  and others) and the watchdog's
  `ReclaimReport` derive `Debug` only, no longer `Clone`, `Copy`,
  `PartialEq` or `Eq`; compare them with `matches!`. A failure SubEtha
  detects itself carries its own message under its kind, such as a file
  at a structure's path that no layout reads (`InvalidData`).
- Three more I/O failures carry the `std::io::Error`. `NotifyError::Io`
  held a kind and a message and `DequeError::Io` a message; both hold the
  error and return it from `source()`. `AdaptiveError::GrowthFailed` held
  nothing and now holds the `RingError` growing the ring returned, so a
  failed producer registration says why. `DequeError` and
  `AdaptiveError` derive `Debug` only, and `DequeError` no longer
  converts from the KHPD deque's `PushError`.
- `SharedArcDyn::read_at` and `write_at` refuse a range past the end of
  the value with the new `ArcError::OutOfRange { offset, len, value_bytes }`,
  where they returned `ArcError::LayoutMismatch`.
- Every `LayoutMismatch` in `subetha-cxc` carries a
  `subetha_cxc::layout::Mismatch { field, found, wanted }`: the first
  field of the file's layout that differs from what the call asked for,
  the value the file holds and the value asked for. It prints as "the
  file's capacity is 16 where 32 was asked for", as "the file is 4096
  bytes where 8192 were asked for" for a file shorter than the layout,
  and in hexadecimal for a magic. `RawUniversalError::StateMismatch` and
  `BlockingError::WakerLayout` carry one too.
- Refusals reported as `LayoutMismatch` that concern no file have
  variants of their own. `InvalidArgument(&'static str)` on `RingError`,
  `VecError`, `SlabError`, `RegionError` and `ArenaError` names an
  argument the structure cannot take, such as a capacity of zero or a
  frame ring's slot under 16 bytes. `RingError::Stamped` is a stamped
  ring asked for frames or the Vyukov shape,
  `RingError::UnknownStampKind(u32)` an ordering region naming a stamp
  kind this build does not read, and `StackError::PayloadTooLarge` a
  `RawTreiberStack` push longer than a slot.
  `VersionedError::InvalidConfig` carries the B-tree's `InvalidConfig`
  and a value of the wrong size, and
  `LanedError::LaneOutOfRange { lane, lanes }` a lane index past the
  map's lanes.
- `DequeError`'s `InvalidMagic`, `CapacityMismatch` and
  `SlotBytesMismatch` are one `DequeError::LayoutMismatch(Mismatch)`, as
  every other structure's layout refusal is. A `RawDeque` thief whose
  element alignment or layout tag differs from the file's names that
  field, where it reported `InvalidMagic`.
- `wait_plan::MonitorFamily` gains `Tpause(UserWait)`, the monitor phase
  that pauses in `TPAUSE` slices where `UMWAIT` does not hold; a `match`
  over it needs the arm.
- Every `AdaptiveRing` slot says what it holds: a plain item, an inline
  frame or an offset frame's descriptor. In the shapes built of
  single-producer cores (SPSC, MPSC, MPMC) the kind is the slot's last
  byte, so a plain item carries at most 63 bytes
  (`adaptive_ring::PLAIN_ITEM_BYTES`) where it could fill the 64-byte
  slot, and a 64-byte `try_send` returns `RingError::PayloadTooLarge`, on
  `AdaptiveRing` and on the rings built on it (`CapacityAdaptiveRing`,
  `LocaleAdaptiveRing` and their blocking forms).
  The Vyukov shape keeps its 56 bytes and carries the kind in two high
  bits of the slot's sequence word, set by the store that publishes the
  slot. A plain push leaves the kind zero, so plain traffic does no extra
  work.
- `RingError` gains `NotAFrame`: `AdaptiveRing::recv_frame` finding a
  plain item next leaves it on the ring and returns `NotAFrame`, where it
  read the item's bytes as a frame descriptor and could read and free a
  payload block another frame owned. A frame whose kind, class, length or
  descriptor fails its checks is taken off the ring and refused with
  `IoError(InvalidData)`, reading and freeing nothing. A `match` over
  `RingError` needs the arm.
- A frame region's file is laid out as version 2 (magic
  `0x4652_474e_0000_0002`): a 64-bit allocation tag per block, odd while
  the block is held, sits between the header and the blocks, and an
  offset frame's descriptor carries its block's tag. The free-list head
  holds its block index exclusive-ored with `u32::MAX`, so the empty
  list reads as zero, and once the bump counter is spent it reads as one,
  the index below the empty mark, which refuses an allocation without
  touching the counter. The receive checks
  the block index, the length and the tag before it reads or frees the
  block. A region file from an earlier SubEtha is refused as a
  `LayoutMismatch`.
- `SharedTreiberStack`, `RawTreiberStack`, `SharedRegion`, `RawRegion`,
  `SharedBTreeMap` and `RawBTreeMap` keep, in the free-list head, the
  index one below the empty mark once their bump counter is spent, and
  refuse an allocation that finds it without touching the counter. Their
  magics are `0x4150_5432` (`STACK_MAGIC`), `0x4150_5232`
  (`REGION_MAGIC`), `0x4254_5245_454D_5032` (`BTREE_MAGIC`) and
  `0x5241_5742_5452_4532` (`RAW_BTREE_MAGIC`), so an earlier SubEtha's
  file is refused as a `LayoutMismatch`.
- `HeartbeatSlot`'s `pid` and `seq_version` are one
  `identity: AtomicU64`: the occupant's pid in its low half, a generation
  the slot advances each time it is vacated, and a bit held while a
  register writes the slot. A register claims a vacant slot, writes its
  epoch, in-flight bits and role, and publishes them with one
  compare-and-swap, where `snapshot` spun while a register or unregister
  held the slot's version odd. `snapshot` returns `None` for a slot whose
  register has not published, so it never waits on a registrant that died
  mid-register, and retries only when the slot was vacated while it read.
  While a register writes the slot, `identity` names the process writing
  it, which need not be the pid it registers, and the slot's new
  `claimant` word holds that writer's pid and process start time. A
  register that finds every slot held takes back one whose writer exited
  before publishing: its pid names no process, or names one that started
  at another time, so a pid the system has handed to another process does
  not keep the slot held. A register reads no other process's start time,
  whatever pid it registers.
  `unregister` leaves a vacant slot, or one still being registered, as it
  is. The table's magic is `0x4150_4D46_4842_4132`, so
  `HeartbeatTable::open` refuses a table file from an earlier SubEtha as a
  `LayoutMismatch` naming the magic.
- `SharedSlab`, `RawSlab` and `SharedReservoirSampler` keep each element
  in a slot of two buffers, a head naming the current one and a claim,
  where a slot carried a version a reader spun on while it was odd. A
  write takes the slot's claim, fills the buffer the head does not name
  and makes it current with one compare-and-swap of the head; a read
  copies the buffer the head names and retries only when a write landed
  during it, so no read waits on a writer. Writers of one slot take its
  claim in turn and each lands whole, so two writers on one slab or
  reservoir slot no longer tear it, and a writer that dies holding the
  claim is taken over, judged by its pid and process start time.
  `ReservoirSlot` is `{ head, claim, buffers }`, 128 bytes. A slab slot
  holds the record twice behind a 16-byte prefix (`SLAB_SLOT_PREFIX` is
  16), in whole lines: 64 bytes for a record of up to 24 bytes, 128 up to
  56, 384 for 168. `raw_slab_geometry` returns the first buffer's offset
  and the new stride, and `SharedVersionedSlab` slots, chains in slab
  slots, double with them. `slot_version` keeps its meaning: twice the
  writes, odd while a writer holds the slot. The magics are `0x4150_5332`
  (`SLAB_MAGIC`) and `0x4150_5253_4D50_4C32` (`RESERVOIR_MAGIC`), so an
  earlier SubEtha's file is refused as a `LayoutMismatch`.
- `SharedVec` and `RawVec` take `writers` at `create`, `open`,
  `open_read_only` and `reset`: the size of the vec's writer table, one
  entry per handle writing the vec at once. A handle past it writes all
  the same; if its process dies without dropping the handle, the vec
  keeps working as though that process still wrote, correct but slower,
  until it is reset. A file with another size is a `LayoutMismatch` on
  the writer slots, and `vec_file_size::<T>(capacity, writers)` and
  `raw_vec_file_size(capacity, writers, layout)` count the table. An
  element of at most 8 bytes at an alignment of at most 8 sits in a
  64-byte word slot, its word at 0; a larger one, up to 56 bytes
  (`VEC_PAYLOAD_BYTES`, where it was 52) in the typed vec, sits in a
  128-byte `VecSlot` of `{ head, claim, buffers }` behind
  `VEC_SLOT_PREFIX` (16) bytes, and `raw_vec_geometry` returns either
  slot's element offset and stride. `len` counts the slots below the
  length, each an element or a hole, and never exceeds the capacity;
  `get` answers `None` for a hole, and `set` refuses one with
  `OutOfBounds`, which says the index is at or past the length or a hole.
  `VEC_MAGIC` is `0x4150_5633`, so a vec file from an earlier SubEtha is
  refused as a `LayoutMismatch`. A vec needs a lock-free 16-byte
  compare-and-swap, and `open_read_only` of word slots a processor that
  reads 16 bytes whole without one; elsewhere they fail with an
  `Unsupported` I/O error.
- `SharedBroadcastRing`'s slots carry a stamp, the message's sequence
  number plus one, where they carried a version a consumer spun on while
  it was odd. The producer writes only a slot every consumer it counts has
  read: it clears the stamp, writes the payload and stamps the slot. A
  consumer checks the stamp before and after its copy, so no consumer
  waits on the producer, and a producer that stops part way through a
  write leaves a slot no counted consumer reads until the next producer
  writes it whole. A consumer unregistered while it reads, whose slot the
  producer writes again during the copy, gets `InvalidConsumer` rather
  than the later message. `BroadcastSlot` is `{ stamp: AtomicU64,
  payload }`, still 64 bytes with 52 payload bytes, and the magic is
  `0x4150_4232` (`BROADCAST_MAGIC`), so a broadcast ring file from an
  earlier SubEtha is refused as a `LayoutMismatch`.
- The once cell is format 4. `OnceHeader` is one 64-byte line: the magic
  `0x4F4E_4334` (`ONCE_MAGIC`), the value size, the `winner` word, zero
  until a value is published and then 1 for the inline slot or 2 + i
  for slot line i, the `fetcher` word, the fetch claim's holder, and the
  inline slot: a state word and `ONCE_INLINE_BYTES` (32) bytes of value.
  `ONCE_SLOTS` (eight) 64-byte `OnceSlot`s follow, each a state word and
  the 56-byte payload, so `ONCE_FILE_SIZE` is 576. A caller that finds
  the cell empty copies its value into a free slot, the inline slot
  first when the value is 32 bytes or less, and publishes it with one
  swap of the winner word, so `set` and `get_or_init` never wait on
  another caller's producer; `get_or_init` runs its producer in each
  caller that finds the cell empty and returns the value published
  first. A value in the inline slot is read from the header's own line.
  A slot whose copier is gone, judged by its pid and process start, is
  freed by the next caller that needs one. A cell file from an earlier
  SubEtha is refused as a `LayoutMismatch`.
- `LazyConfig::get_or_fetch` returns `Fetch<T>`: `Fetch::Ready(value)`,
  or `Fetch::Pending` at once when a live caller holds the cell's fetch
  claim, where it waited for that caller and returned `T`. A fetch claim
  whose holder is gone is taken over, and a fetch that panics gives the
  claim back.
- `HolderSlot`'s `owner_pid` is `owner: AtomicU64`, the holder's pid and
  process start time, so `HolderTable::reap_dead`,
  `SharedEpochs::dead_tickets` and `free_dead_ticket`, and the holder
  reaps of `SharedArc`, ring holders, the laned versioned maps and
  `RawArena`'s collector free a slot whose pid now names a process that
  started later, where they kept it held until that process exited.
  `release_if` and `SharedHolderTable::owner_pid` still take and return a
  pid. The files that embed holder slots carry new magics
  (`SHARED_HOLDER_MAGIC` `0x484F_4C44_4552_5332`, `EPOCH_MAGIC`
  `0x4550_4F43_4853_5F33`, `ARC_MAGIC` `0x5348_4152_4544_4132`,
  `HOLDERS_MAGIC` `0x5355_4245_484c_4432`, `LANES_MAGIC`
  `0x4C41_4E45_434C_4D32`, `RAW_ARENA_MAGIC` `0x5241_5741_5245_4E32`), so
  an earlier SubEtha's file is refused as a `LayoutMismatch`.
- A `RawArena` block header records its writer's pid and process start
  time in a word beside the state, and the claim that makes a block
  `Writing` sets both with one 16-byte compare-and-swap. `RawArena::collect`
  and `SharedNamedValues::get` treat a block whose writer's pid now names a
  process that started later as a dead writer's, where `get` waited on it
  until that process exited, and `write_payload` and `publish` refuse such
  a block as `NotWriting`. `RawArena::writer_of` still returns the pid.
  `RawArena::create` and `open`, and so `SharedNamedValues`, fail with an
  `Unsupported` I/O error on a processor without a lock-free 16-byte
  compare-and-swap, as the linked lists do.
- `NodeHandle` carries the node's `generation` beside its `index`, and
  `HEAD_INDEX` and `NIL_INDEX` are `u64`; `RawLinkedList`'s handles are
  `u64`, the slot in the low half and the generation in the high half. A
  handle used after its node was removed or popped is refused:
  `SharedLinkedList::get` and `remove` return `None` and `set` returns
  `LinkedListError::InvalidHandle`, as every `RawLinkedList` call taking
  a handle does.
- `SharedUniversal::insert` returns whether the value was new: `Ok(true)`
  when it was added and `Ok(false)` when the set held it, where it
  returned `Ok(())`.

#### Added

- `SharedOnceCell::get_or_fetch` runs a producer under the cell's one
  fetch claim, so at most one caller runs it at a time across every
  process, and returns `Fetch<T>`: `Fetch::Ready(value)`, or
  `Fetch::Pending` at once when a live caller holds the claim. `OnceSlot`
  and `ONCE_SLOTS` describe the cell's slots.
- `LocaleAdaptiveRing::unregister_producer` and `unregister_consumer` give
  an id back on all three backings (anonymous, shared memory and file),
  and the id is handed out again. A producer's items stay until a
  consumer receives them; a consumer's rings pass to the consumers that
  remain. `BlockingLocaleRing` reaches both through `ring()`.
- `subetha_cxc::BlockingBroadcastRing`, `BlockingCapacityRing` and
  `BlockingLocaleRing`: a `SharedBroadcastRing`, `CapacityAdaptiveRing` or
  `LocaleAdaptiveRing` with the two wakers the C ABI keeps beside it
  (`<path>.cwaker.bin` and `<path>.pwaker.bin`, and `{name}_cwaker` and
  `{name}_pwaker` beside a broadcast or capacity ring in shared memory).
  A broadcast push wakes every waiting consumer and a capacity or locale
  send one; a receive, or a broadcast consumer given back, wakes one
  waiting producer; and the `_blocking` forms wait across processes. A
  capacity morph and a locale migration wake every waiter on the wakers,
  as a shape morph does on a ring under `BlockingAdaptiveRing`, whoever
  makes it. Waits through the family blocking types fail with the generic
  `BlockingWaitError<E>`, which carries the structure's own error as
  `Op`.
- `subetha_cxc::BlockingDeque` and `BlockingStack`: a `RawDeque` or
  `RawTreiberStack` with the two wakers the C ABI keeps beside its file. A
  push wakes one waiter for an item, and a steal from the deque or a pop
  from the stack wakes one waiter for room; the deque owner's pop from its
  own end wakes no one, since the owner is its only producer.
  `steal_blocking` and `pop_blocking`, whose only refusal is finding
  nothing, fail with `BlockingWaitError<Infallible>`.
- `subetha_cxc::BlockingVyukovRing`: a `SharedRing` with the two wakers
  the C ABI keeps beside it (`<path>.cwaker.bin` and `<path>.pwaker.bin`,
  or `{name}_cwaker` and `{name}_pwaker` beside a ring in shared memory).
  A push wakes one waiting consumer and a pop one waiting producer, and
  `push_blocking` and `pop_blocking` wait across processes. The C ABI's
  Vyukov ring waits through it.
- `SharedRing::create_or_attach` is public: it builds the ring when no
  file is there and attaches to it, items and all, when one is, where
  `create` starts the file over.
- `SharedCell::load` and `RawCell::load` return the value with the
  generation that made it current, and `compare_and_set(seen, value)`
  makes a value current only if the generation is still `seen`: of
  writers that all saw one generation exactly one lands, and the rest get
  `Err` with the generation the winner made, their values never visible.
- The `trace-index` feature counts, per thread, what each
  `SharedVersionedIndex` search and read does (levels walked, keys visited
  and compared, versions examined, timestamp ticks spent searching and
  picking the version), read with `shared_versioned_index::trace::take`.
  It is off by default and compiles to nothing when off; the
  `vindex_trace` example prints the counts for the bench's `get_at` store,
  and the `vindex_read_arms` example runs one `get_at` arm alone for
  hardware counters.
- `AdaptiveOrderedReceiver::over_blocking` builds an ordered receiver
  over a `BlockingAdaptiveRing`: each pop also wakes one producer parked
  for room, as a pop through the waiting ring does. A receiver made with
  `new` pops through the ring alone and wakes no one.
- `subetha_cxc::SharedVersionedIndex<K, V>`: an ordered map in shared
  memory that any number of threads and processes write at once with no
  lock, keeping every version of a key a pinned reader can still need.
  A key's versions sit newest first behind one atomic word, so one
  compare-and-swap decides each write: of writers racing on a key
  exactly one wins and the rest get `IndexError::Conflict`, and of
  writers racing to create a key exactly one version lands. A write
  takes the statement's `EpochTicket` and `PinGuard` and builds only on
  the version the pin sees; reads take a `PinGuard` and see their own
  ticket's writes when given it. `void` takes back an epoch's version of
  a key, `void_epoch` takes back everything an epoch wrote from the
  write list kept for its ticket, for a writer whose process died, and
  `sweep` drops what the epoch table's reclaim horizon allows and reuses
  a slot only once no pin that could be reading it is left. The index
  grows in chunk files beside its main file as it needs them.
- `EpochTicket::slot`, the ticket table slot a write holds, and
  `EpochTicket` itself at the crate root beside `PinGuard`.
- `subetha_cxc::raw_shared_versioned_index::RawSharedVersionedIndex`: the
  same index, one lock-free algorithm with `SharedVersionedIndex`, at key
  and value sizes given at run time, keys ordered as unsigned bytes. It is
  built over an `Arc<SharedEpochs>` the caller holds; a ticket crosses as
  the slot and epoch `SharedEpochs::claim_ticket` returns and a pin as its
  epoch, and every call holds a pin of its own while it walks. A typed
  index and a raw one carry different magics and refuse each other's
  file.
- `subetha_cxc::BlockingAdaptiveRing`: an `AdaptiveRing` with the two
  wakers its waiters park on and the notify record its pollers attach
  to, under the names the C ABI keeps beside a ring
  (`<prefix>.cwaker.bin`, `<prefix>.pwaker.bin`, `<prefix>.notify.bin`;
  `{name}_cwaker`, `{name}_pwaker`, `{name}_notify` in shared memory).
  A push through it wakes one waiting consumer and signals every attached
  notifier, a pop wakes one waiting producer, and `send_blocking`,
  `recv_blocking` and their stamped and framed forms wait. The C ABI's
  ring, Python's `Ring` and PowerShell's `SubEtha.Ring` all push and wait
  through it.
- `subetha_cxc::claim::Claim`, exclusive access to a value through one
  atomic word. An uncontended claim is one compare-and-swap to take and
  one to give back. A contended caller waits by the plan in force, taking
  the value whenever it is free, until the plan's park point; then it
  joins a first-in, first-out queue whose head is handed the value by the
  second release after it starts waiting. No caller, however tight its
  loop, keeps another from the value.
- `subetha_cxc::wait_active::plan_in_force`, with `PlanInForce` and
  `PlanSource`: the plan a wait of a given park kind would use now on the
  calling thread, and whether calibration made it or it is the fixed
  ladder. Unlike `active_plan`, it never starts the calibration.
- `subetha_cxc::ByteValue<N>`: up to `N` bytes that keep their length,
  marshaled as a 32-bit length then the `N` bytes, with `SlotValue` (52
  bytes, the most one channel slot carries) and `LeaseValue` (44 bytes)
  named. `ByteValue::from_bytes` refuses a longer string with
  `ValueTooLong`, naming the most and what was given. Python's and
  PowerShell's channels, maps and slot families hold it, and the C ABI's
  channel encodes its items the same way, so each reads the others'
  items at one path.
- Every error type in `subetha-cxc` and `subetha-pointers` implements
  `std::fmt::Display` and `std::error::Error`, so `?` carries any of them
  into `Box<dyn Error>` or `anyhow`. Fifty-nine `subetha-cxc` errors
  (`MapError`, `RingError`, `DequeError` and `CondvarError` among them)
  and `subetha-pointers`' `CapabilityError`
  and `RaspError` implemented only `Debug`. An error that wraps another
  returns it from `source()`, so a reporter prints each layer.
- `subetha_cxc::layout::Mismatch` implements `std::error::Error`, and an
  `std::io::Error` that refuses a file or named region of another shape
  carries one (`From<Mismatch> for std::io::Error` builds it, as
  `InvalidData`): `Mismatch::carried_by(&e)` reads it, and the error
  prints as the `Mismatch` does, "the file is 64 bytes where 128 were
  asked for", where a file-backed refusal printed its path and a longer
  text.
- `HybridLogicalClock::checked_advance` and `checked_merge` and
  `VectorClock::checked_increment` in
  `subetha_pointers::versioned_pointer`, and `SharedFenceClock::checked_tick`
  and `checked_merge` in `subetha_cxc::shared_fence_clock`, return `None`
  for a step from a count already at `u64::MAX`; the fence clock's leave
  the slot as it was.
- `TemporalSensor::checked_observe` and `ReceiveTimeWentBack` in
  `subetha_cxc::temporal_sensor`.
- `subetha_cxc::adaptive_ipc::AdaptiveIpc::try_send_batch` sends a run
  without waiting until the queue is full, records what landed as one
  batch, and returns how many landed; `subetha_cxc::adaptive_ipc::unlink`
  removes every file an endpoint lays out, for a process that ended
  without dropping one.
- `subetha_cxc::api::BLOCKING_HEAL` is public: the longest one park of a
  channel's blocking send or receive lasts before it looks at the ring
  again, a millisecond.
- `subetha_pointers::bloom_pointer::BloomFine` gains `from_words` and
  `words`, the four words the C ABI's `subetha_fine_bloom` holds.
- `subetha_cxc::monitor_wait::umwait_holds` and `tpause_holds` return
  whether each WAITPKG wait sleeps to its deadline on this host, judged
  at first use by the length of real waits.
- `AdaptiveRing::recv_any` takes the next item whatever it is and says
  which: `RingItem::Plain` with the slot's payload (63 bytes in the
  shapes built of single-producer cores, 56 in the Vyukov shape), or
  `RingItem::Frame(FrameClass)` with the frame's payload.
  `BlockingAdaptiveRing::recv_any` and
  `recv_any_blocking(consumer_id, out, timeout)` do the same and wake a
  producer waiting for room. The receive reads the kind from the slot
  before it claims it, and an item keeps its kind through shape morphs,
  capacity morphs and locale migrations.
- `RawArena::writer_gone(h)`: whether a block is being written by a
  process that has exited, judged by its pid and process start time, so a
  pid the system has handed to another process does not read as the
  writer.
- `SharedUniversal::unlink(base)` removes every file of the set at
  `base`, its state file and each table, and reports them in an
  `UnlinkReport`: a missing file is counted, not an error.
- `atomic_offset`, offsets into a mapping held as integers relative to a
  base the reader supplies, so they mean the same in every process
  whatever address it mapped the region at, re-exported at the crate
  root: `AtomicOffset`, one 64-bit offset with a bounded `reserve`;
  `Tagged32`, a 32-bit offset beside a 32-bit tag, whose `swap_counted`
  installs an offset with the tag one past; `Pair32`, two 32-bit indexes
  that change together; `Quad32`, four 32-bit indexes or two beside a
  64-bit version in one 16-byte word, on a processor with a lock-free
  16-byte compare-and-swap (`Quad32::supported`); and `AtomicRel`, a
  pointer stored relative to its own address.
- `RawVec::for_each_range` hands each element of a range to a closure
  with its index, holes left out, as `SharedVec::for_each_range` does.
  `writer_slots()` on `SharedVec` and `RawVec` returns the size of the
  writer table an open must be given, and `raw_vec_table_offset` where
  the table starts.

#### Changed

- An error that wraps another prints the inner error's message where it
  printed its `Debug` form: `ApiError`'s transport, marshal and map
  errors, `BlockingTcpBridgeError`, `CapacityMorphError`,
  `BroadcastCapacityMorphError` and `NamedValuesError`.
- The structures whose constructors return `std::io::Error` refuse a file
  of another shape with `InvalidData` carrying the `Mismatch`, where they
  gave a text naming no values: `PubSubRing`, the KHL, KHPD, LOH and URD
  deques, `SubscriberPosition` and the hugetlbfs region.
- `SharedHashMap::create` no longer writes to stderr when the operating
  system refuses its region; the error carries the system's code and
  message.

#### Removed

- `SharedRWLock` with `ReadGuard`, `WriteGuard`, `RWLockError`,
  `RWLockHeader` and `RWLOCK_MAGIC`; `BlockingRWLock` with
  `BlockingReadGuard`, `BlockingWriteGuard` and `BlockingRWLockError`;
  `SharedSemaphore` with `Permit` and `SemaphoreError`;
  `BlockingSemaphore` with `BlockingPermit` and `BlockingSemaphoreError`;
  `OwnerLease` with `LeaseError`, `LeaseHeader`, `LeasePayload`,
  `LEASE_FILE_SIZE`, `LEASE_MAGIC`, `NO_OWNER` and `LEASE_PAYLOAD_BYTES`;
  and `raw_owner_lease::RawOwnerLease`.
- `SharedLinkedList::region()`, `LinkedListNode<T>`, `raw_node_geometry`
  and `raw_node_layout`, which exposed the list's earlier storage.
- `SharedOnceCellDyn::wait`, `SharedOnceCellDyn::reclaim` and
  `shared_once_cell::WaitError`. A caller that did not win `claim` reads
  the value with `try_get`, which never blocks, and `claim` takes over a
  claim whose process is gone.

#### Fixed

- A `SharedOnceCell` caller that lost `get_or_init`'s race no longer
  spins for good when the winner never publishes. It spun on the state
  word at a full core until the value appeared, so a winner whose process
  died partway through its initializer, or whose initializer panicked,
  left every other caller spinning forever. No caller now waits on
  another's initializer: each publishes its own value, and the first
  published is the one every caller gets. `SharedAsyncPointer::get_or_lazy`,
  which spun the same way after losing `set`, no longer waits either.
- A slot past a holder table's or a heartbeat table's capacity reached
  memory outside the table in a release build, from safe code.
  `HolderTable` checked the slot only in debug builds, so `publish`,
  `release` and `payload` read and wrote past the slots, and
  `HeartbeatTable` checked it in no build, in `beat`, `unregister`,
  `snapshot` and the in-flight marks. Every call that takes a slot on
  `HolderTable`, `SharedHolderTable`, `HeartbeatTable`,
  `SharedFenceClock` and `SharedEpochs::publish_ticket` now panics on
  one past the capacity in every build, as each call's `# Panics`
  section says, and `SharedFenceClock`'s `checked_tick` and
  `checked_merge` return `None`. `SharedFenceClock::unregister` refuses
  such a slot where it ignored it, and `slot_snapshot` where it returned
  `None`, what it returns for a vacant slot. A heartbeat in-flight bit past
  the 64 a slot holds now panics in every build, where a release build
  wrapped it onto bit `bit % 64`. The C ABI already refused both.
- A holder table claim with a payload of `HOLDER_FREE` (0) or
  `HOLDER_RESERVED` (`u64::MAX`), the values the table keeps for a free
  slot and a forming one, reserved a slot before `publish` refused the
  payload, so the panic left that slot held and no caller holding its
  index. `HolderTable::claim` now refuses the payload before it
  reserves.
- `SharedDeque::open_as_thief` read the 64-byte header of a file shorter
  than it, and the slots its header records of a file cut short, past
  the end of the mapping. Both are refused as `DequeError::LayoutMismatch`
  naming the length, as `RawDeque::open_as_thief` already refused them.
- A clock step from a count already at `u64::MAX` is refused. Release
  builds wrapped the count to zero and returned a reading ordered before
  the input: a `HybridLogicalClock` at (5, `u64::MAX`) advanced at
  physical 5 returned (5, 0), a merge with a received (5, `u64::MAX`)
  returned (5, 0), and a `VectorClock` ticked at `u64::MAX` returned a
  clock that happened before the one it stepped from. Debug builds
  panicked. `advance`, `merge`, `increment` and `SharedFenceClock`'s
  `tick` and `merge` panic in every build, and the checked forms return
  `None`.
- `TemporalSensor` refuses a receive time earlier than the previous one,
  which is the receiving clock stepping back rather than the link. A step
  back of 140 microseconds moved the mean spacing from 62.5 microseconds
  to 1.15e18 in every build, and one below the window's first receive time
  panicked in debug builds ("attempt to subtract with overflow") and gave
  a meaningless trend in release. `checked_observe` returns
  `ReceiveTimeWentBack` with both times and folds nothing in, and
  `observe` panics. A send time may still go back, for an item that
  arrived out of order.
- An attach to a named shared-memory region shorter than it asks for is a
  layout mismatch on every platform. `ShmFile::open_named_secured`, and on
  Windows a create that finds a smaller section of its name, return
  `InvalidData` carrying the length `Mismatch`. The adaptive ring's
  backings, payload and ordering regions, peer directory, notifier record
  and holder table report it as their `LayoutMismatch`. Windows reported
  the refusal only as access denied. A Windows section is a whole number
  of pages, so the length it reports is the size it was created with
  rounded up to a page.
- A channel's and a work queue's I/O and layout failures, and an adaptive
  queue's on a send, a morph or an ordering change, reach the caller as
  `ApiError::Io` carrying the operating system's error or the
  `Mismatch`, where they were `TransportError::Other` and printed as
  "transport: the transport failed in a way the other variants do not
  name".
- `AdaptiveIpc::send_batch` waits for room when the side path that carries
  batches of items of 16 bytes or fewer is full, as it waits on the ring
  and the deque; it returned `TransportError::Other`. Each item it lands
  wakes a receiver waiting on the endpoint, which otherwise woke at its
  next heal tick.
- `SharedArray::create` over an array of another shape resized the file
  before refusing it, so a create with the wrong length truncated the
  array's data. The shape is compared first and the file is left as it
  was.
- `PriorityFanout::open` reported every failure to open its bitmap, a
  missing file among them, as a layout mismatch; it returns the atomic's
  own error.
- `DirectFileRing` and `SubscriberPosition` turned their counters' errors
  into an `io::Error` holding `Debug` text, so a missing counter file lost
  its code. The operating system's error arrives as it came; a counter
  file of another shape is the ring's `LayoutMismatch` and the position's
  `InvalidData`.
- `SharedDequeUrd::wait_and_drain` on a processor with WAITPKG loaded its
  mailbox's claim, then armed `UMONITOR` and slept in `UMWAIT` without
  checking the claim again, so a batch published between the load and
  the arming went unseen until the wait's deadline, the operating
  system's cap on `UMWAIT` or an interrupt. It arms the monitor, checks
  the claim, then sleeps, as the monitor tier's waits do.
- `subetha-cxc` builds for musl targets such as
  `x86_64-unknown-linux-musl`, where it stopped at six type errors.
  musl's `libc` types `msghdr::msg_controllen` and `cmsghdr::cmsg_len`
  as `socklen_t` (`u32`) where glibc's are `size_t`, and `recvmmsg`'s
  flags as `unsigned int` where glibc's are `int`, and each was set as
  glibc's type. Each is now set as its own C library's type, so a glibc
  build compiles to the same code, and a musl build names neither
  `time_t` nor `suseconds_t`, the aliases musl's `libc` marks deprecated.
- The tests that making room through the structure alone leaves a waiting
  producer asleep no longer fail under load: the ordered receiver's, and
  those of the blocking broadcast, capacity, locale and Vyukov rings, the
  blocking stack and the blocking deque. Each gave the producer 100 ms to
  start waiting before making room, and a producer that had not yet looked
  for room found the freed slot and sent. Each now makes room once the
  producer's wait has begun, which follows its last look for room.

### Python

#### Breaking

- `Cell` takes any number of writers at once, none waiting on another,
  over the cell's new 4,096-byte layout; a value may be 56 bytes.
  `Cell.version` counts the writes that landed, one per write, where it
  stepped twice, and `set` returns the version it made.
- The binding raises Python's own exception types. An operating-system
  failure is an `OSError` carrying `errno`, and `filename` when the call
  opened a path, so `FileNotFoundError`, `PermissionError` and the rest
  match on their own; on Windows `winerror` holds the system's code. A
  file holding a structure of another shape raises the new
  `subetha.LayoutMismatch`, an `OSError`. A refused argument raises
  `ValueError`, an index or a range past the end `IndexError`, an absent
  key `KeyError`, a wait that ran out `TimeoutError`, and a write to a
  structure opened read-only `io.UnsupportedOperation`; a full or
  exhausted structure stays an `OSError`. Messages are SubEtha's own
  text, where they printed a Rust variant such as `LayoutMismatch` or
  `IoError(NotFound)`. Refusals that concern no file, which Rust now
  reports with variants of their own, raise `ValueError`, `IndexError`
  or `io.UnsupportedOperation`.
- `Ring.send`, `CapacityRing.send` and `LocaleRing.send` take an item of
  at most 63 bytes while the ring is in a shape built of single-producer
  cores, where they took 64, and raise `ValueError` past it: the slot's
  last byte holds the item's kind.
  `Ring.recv_frame` finding a plain item next leaves it on the ring and
  raises the new `subetha.NotAFrame`, where it read the item as a frame;
  a frame that fails its checks is taken off and raises `OSError`.
- A `Heartbeat` slot is published by one compare-and-swap once its
  fields are written, so an `EpochBarrier` counting live peers never waits
  on a registrant that died mid-register, and `register` on a full table
  takes back a slot whose registrant's process exited before publishing,
  even once the system has handed its pid to another process.
  The table's file has a new magic: `Heartbeat.open` refuses a heartbeat
  file from an earlier SubEtha with `subetha.LayoutMismatch`.
- `Slab` keeps each record twice in its slot, behind a head a read
  checks, so a read never waits on a writer, any number of processes may
  write one slot with each write landing whole, and a writer that dies
  mid-write is taken over. A slot is its record twice behind 16 bytes, in
  whole 64-byte lines, and `slot_version` still steps by two per write.
  Its file has a new magic: opening one from an earlier SubEtha raises
  `subetha.LayoutMismatch`.
- `Vec` and `Vec.open` take `writers`, the size of the vec's writer
  table, after `capacity`: one entry per handle writing the vec at once,
  and opening with another size raises `subetha.LayoutMismatch`. Pushes,
  pops and sets never wait on another writer. `len()` counts the slots
  below the length, each an element or a hole: `get` returns `None` for a
  hole, `set` raises `IndexError` for one, and `read_range` leaves holes
  out, so past one an element's place in its result is not its index.
  Opening a vec file from an earlier SubEtha raises
  `subetha.LayoutMismatch`.
- `LazyValue.claim` takes over a claim whose process is gone, judged by
  its pid and process start time, where such a claim stood for good and
  `wait` raised `OSError` for it. A lazy value file from an earlier
  SubEtha raises `subetha.LayoutMismatch`.
- Holder tables, epochs, arenas and laned maps record each holder's
  process start time beside its pid, so their dead-holder reaps free a
  slot whose pid the system has handed to another process; their files
  have new magics, and opening one from an earlier SubEtha raises
  `subetha.LayoutMismatch`.
- `Stack`, `Region` and `BTreeMap` keep the record of a spent slot
  counter in their free-list head, so a full one refuses an allocation
  without touching the counter. Their files have new magics: opening one
  from an earlier SubEtha raises `subetha.LayoutMismatch`.
- A list handle is the node's slot with its generation above it, so its
  value no longer fits 32 bits, and a handle used after its node was
  removed or popped is refused.
- `Universal.insert` returns `True` when the value was new and `False`
  when the set held it, where it returned `None`; `Universal.version()`,
  the set's version, which each migration and each clear steps, takes the
  place of `migrations()`.

#### Added

- `LocaleRing.unregister_producer(producer)` and
  `unregister_consumer(consumer)` give an id back on all three backings,
  and the id is handed out again; a position past the consumer table is
  ignored rather than raising.
- `Cell.load()` returns `(version, value)`, and
  `Cell.compare_and_set(seen, value)` writes only if the version is still
  `seen`, returning `(True, version)` with the version it made or
  `(False, version)` with the version a write that landed first made.
- `subetha.AtomicU32` and `subetha.AtomicBool`, the 32-bit atomic and
  the flag, over the files Rust's `SharedAtomicU32` and
  `SharedAtomicBool` and C's `subetha_atomic_u32_*` and
  `subetha_atomic_bool_*` share. The 32-bit atomic has every operation of
  the 64-bit one except the batched add; the flag loads, stores and
  swaps. Opening a file laid out at another width raises
  `LayoutMismatch`.
- `reset(path, init)` on `Atomic`, `AtomicU32` and `AtomicBool` remakes
  the file holding `init` at the width asked for, whatever it held, as
  Rust and C reset one.
- `Ring.send_for(producer, item, timeout)` and
  `recv_for(consumer, timeout)` wait, returning false or `None` on a
  timeout, as `Channel`'s do, and release the interpreter while they
  wait.
- `Ring.recv_buffer` receives a run into one buffer, each item in a whole
  slot, with no object per item: the receiving half of `send_buffer`, as
  `pop_buffer` already was on the single-producer ring.
- `subetha.VersionedIndex`, the shared versioned index with integer keys
  and values, and its `IndexPin` and `IndexTicket`. A statement takes a
  pin and then a ticket; `write(key, value, ticket, pin)` writes under
  them, a `value` of None deleting; `get_at`, `range_at` and
  `changed_since` read at the pin, with the ticket's own writes when given
  it; and the ticket publishes when its `with` block ends. A write that a
  version born after the pin beat raises `subetha.WriteConflict`, whose
  `epoch` names that version's epoch. `void`, `void_epoch`, `sweep`,
  `version_count`, `live_count`, `now`, `dead_tickets` and
  `free_dead_ticket` complete it.
- `subetha.VyukovRing`, the bounded queue any number of producers and
  consumers share with each item taken once, over the files and
  shared-memory names Rust's `SharedRing` and C's `subetha_vyukov_*`
  use. The constructor creates or attaches, `open` attaches, `anon` makes
  one for threads, and `create_shm` and `open_shm` place it in named
  shared memory. `push_for` and `pop_for` wait, returning false or `None`
  on a timeout, and release the interpreter while they wait; a push or
  pop through any handle, in any process, wakes them. `stats`,
  `next_stuck_slot`, `heal_stuck_slot`, `wake_all`, `close`, `flush` and
  `unlink` complete it.
- `Ring.recv_any(consumer)` takes the next item, a plain item or a frame,
  and returns `(data, kind)`, `kind` being `"plain"`, `"frame_inline"` or
  `"frame_offset"`, or `None` when the ring is empty;
  `recv_any_for(consumer, timeout)` waits for one and releases the
  interpreter while it waits.
- `Universal.unlink(path)` removes every file of the set at `path` and
  returns what it removed.
- `alignment` and `tag` on `Vec`, `Slab`, `Region`, `Stack`, `Deque` and
  `LinkedList` return the alignment and the tag the file's layout records,
  `slot_size` on `Region` returns its slot size and `writers` on `Vec` the
  size of its writer table, so each of these handles names every layout
  argument another handle on the same file must be given.

#### Changed

- Messages from the front door's types change with `ApiError`'s: an error
  that wraps another prints the inner error's message where it printed
  its `Debug` form.
- Any I/O error carrying a layout `Mismatch` raises
  `subetha.LayoutMismatch`, so `subetha.PubSub` opened at another
  capacity raises it where it raised a plain `OSError`.

#### Removed

- `RWLock` and its `Hold`, `Semaphore` and its `PermitHold`, and
  `OwnerLease` and its `LeaseHold`.
- `LazyValue.wait`. A caller that did not win `claim` reads the value
  with `get`, which never blocks and returns `None` until the value is
  published.

#### Fixed

- `HolderTable`, `Heartbeat`, `FenceClock` and `Epochs` raise
  `IndexError` for a slot past the capacity, naming the slot and the
  capacity, before the call reaches the table. The binding handed such a
  slot straight to the core, so in a release build, which every
  published wheel is, `HolderTable`, `Heartbeat` and `Epochs` read and
  wrote memory outside the table, and `FenceClock` raised
  `PanicException`, which `except Exception` does not catch.
- `HolderTable.claim` and `publish` raise `ValueError` for a payload of 0
  or 2**64 - 1, the values the table keeps for a free slot and a forming
  one. Both raised `PanicException`, and a claim left the slot it had
  reserved held; a refused claim now takes no slot.
- `Clock`, `CausalClock` and `FenceClock` raise `OverflowError` for a
  step from a count already at 2**64 - 1, leaving the clock as it was,
  where a release build wrapped the count to zero and returned a reading
  ordered before the input.
- `Timing.observe` raises `ValueError` for a receive time earlier than
  the previous one, which is the receiving clock stepping back rather
  than the link, and folds nothing in; a step back of 140 microseconds
  had moved the mean spacing from 62.5 microseconds to 1.15e18. A send
  time may still go back, for an item that arrived out of order.
- A load or a store on an atomic refuses an ordering it cannot take with
  `ValueError` naming the orderings the operation takes, as the C ABI
  refuses it, where the native library panicked: a load under release or
  acquire-release, and a store under acquire or acquire-release.
  `Atomic.load` had listed all five as accepted.
- `Channel` and `Channel.open` round a capacity up to a power of two, as
  their documentation said they did; one that was not a power of two
  panicked inside `SharedRing::create`.
- `AdaptiveQueue.send_many` returns how many items went in, stopping at a
  full queue, as it said; it waited for room instead, with the
  interpreter lock held. `AdaptiveQueue` rounds its capacity up to a
  power of two, as it said it did; a capacity that was not one panicked
  in the ring underneath.
- A channel or queue that is not there raises `FileNotFoundError`, and
  one of another capacity `LayoutMismatch`, where both raised a bare
  `OSError`.

### PowerShell

#### Breaking

- `SubEtha.Cell` takes any number of writers at once, none waiting on
  another, over the cell's new 4,096-byte layout; a value may be 56 bytes.
  `Version()` is a `UInt64` counting the writes that landed, one per write,
  where it was a `UInt32` stepping twice, and `Set` returns the version it
  made.
- `Send` on `SubEtha.Ring`, `SubEtha.CapacityRing` and
  `SubEtha.LocaleRing` takes an item of at most 63 bytes while the ring
  is in a shape built of single-producer cores, where it took 64, and
  fails past it: the slot's last byte holds the item's kind.
  `RecvFrame` finding a plain item next leaves it on the ring and fails
  with an error named SubEthaNotAFrame, where it read the item as a
  frame.
- `Snapshot` on `SubEtha.Heartbeat` returns `$null` for a slot whose
  register has not yet published, where it waited for the register, so it
  never waits on a registrant that died mid-register; `Register` on a full
  table takes back a slot whose registrant's process exited before
  publishing, even once the system has handed its pid to another process.
  The table's file has a new magic: `Open-SubEthaHeartbeat`
  over a heartbeat file from an earlier SubEtha fails with a message
  naming the magic.
- `SubEtha.Slab` keeps each record twice in its slot, behind a head a
  read checks, so a read never waits on a writer, any number of processes
  may write one slot with each write landing whole, and a writer that dies
  mid-write is taken over. A slot is its record twice behind 16 bytes, in
  whole 64-byte lines, and `SlotVersion` still steps by two per write.
  Opening a slab file from an earlier SubEtha fails with a message naming
  the magic.
- `New-SubEthaVec` and `Open-SubEthaVec` take `-Writers`, the size of the
  vector's writer table, as their fourth positional parameter: one entry
  per handle writing the vector at once, and an open naming another size
  fails with a message naming the writer slots. Pushes, pops and sets
  never wait on another writer. `Count()` counts the slots below the
  length, each an element or a hole: `Get` returns `$null` for a hole,
  `Set` fails on one, and `ReadRange` leaves holes out, so past one an
  element's place in its result is not its index. Opening a vector file
  from an earlier SubEtha fails with a message naming the magic.
- `Claim` on `SubEtha.LazyValue` takes over a claim whose process is
  gone, judged by its pid and process start time, where such a claim
  stood for good and `Wait` threw for it. Opening a lazy value file from
  an earlier SubEtha fails with a message naming the magic.
- `SubEtha.HolderTable`, `SubEtha.Epochs`, arenas and laned maps record
  each holder's process start time beside its pid, so their dead-holder
  reaps free a slot whose pid the system has handed to another process;
  opening one of their files from an earlier SubEtha fails with a message
  naming the magic.
- `SubEtha.Stack`, `SubEtha.Region` and `SubEtha.BTreeMap` keep the
  record of a spent slot counter in their free-list head, so a full one
  refuses an allocation without touching the counter. Opening one of
  their files from an earlier SubEtha fails with a message naming the
  magic.
- The `shm:` drive's default store is `shm.<format>.map`,
  `shm.<format>.arena` and `shm.<format>.epochs`, `<format>` being sixteen
  hex digits derived from the three files' magics and the drive's own
  layout, so importing the module after an upgrade that changed any of
  them builds a fresh store beside the earlier one instead of failing on
  its layout. Values an earlier release wrote stay in its own files and do
  not appear in the drive. A default drive whose store still does not open
  is left unmounted, and the module imports with a warning naming the
  store's directory and what refused it, where the import failed.
- A list handle is a `[uint64]`, the node's slot with its generation
  above it, where it was a `[uint32]`; a handle used after its node was
  removed or popped is refused.
- `SubEtha.Universal`'s `Insert` returns whether the value was new, and
  its `Version()`, the set's version, which each migration and each clear
  steps, takes the place of `Migrations()`.

#### Added

- `UnregisterProducer(producer)` and `UnregisterConsumer(consumer)` on
  `SubEtha.LocaleRing` give an id back on all three backings, and the id
  is handed out again.
- `Load()` on `SubEtha.Cell` returns a `SubEtha.CellReading` with the
  value and the version that made it current, and
  `CompareAndSet(seen, value)` writes only if the version is still `seen`,
  returning a `SubEtha.CellWrite` whose `Landed` says whether it did and
  whose `Version` is the version it made or the version a write that
  landed first made.
- `SubEtha.AtomicU32` and `SubEtha.AtomicBool`, the 32-bit atomic and
  the flag, with `New-` and `Open-SubEthaAtomicU32` and `New-` and
  `Open-SubEthaAtomicBool`, over the files Rust's `SharedAtomicU32` and
  `SharedAtomicBool` and C's `subetha_atomic_u32_*` and
  `subetha_atomic_bool_*` share. The 32-bit atomic has every operation of
  the 64-bit one except the batched add; the flag loads, stores and
  swaps. Opening a file laid out at another width fails with the file's
  width and the width asked for.
- `-Reset` on `New-SubEthaAtomic`, `New-SubEthaAtomicU32` and
  `New-SubEthaAtomicBool` remakes the file holding the initial value at
  the width asked for, whatever it held, as Rust and C reset one.
- `SendFor` and `RecvFor` on `SubEtha.Ring` wait, returning false or
  `$null` on a timeout, as the channel's do.
- `RecvPacked` on `SubEtha.Ring` receives a run into one buffer, each
  item in a whole slot, with no object per item: the receiving half of
  `SendPacked`, as `PopPacked` already was on the single-producer ring.
- `SubEtha.VersionedIndex` with `New-` and `Open-SubEthaVersionedIndex`,
  and its `SubEtha.IndexPin` and `SubEtha.IndexTicket`, with the calls
  Python's has: `Write($key, $value, $ticket, $pin)`, `Delete($key,
  $ticket, $pin)` where Python passes None, `GetAt`, `RangeAt`
  returning a `SubEtha.Scan`, `ChangedSince`, `Void`, `VoidEpoch`,
  `Sweep`, `VersionCount`, `LiveCount`, `Now`, `DeadTickets` and
  `FreeDeadTicket`. A write that a version born after the pin beat fails
  with an error named SubEthaWriteConflict naming that version's epoch.
- `RecvAny` on `SubEtha.Ring` takes the next item, a plain item or a
  frame, and returns a `SubEtha.RingItem` whose `Kind` is `Plain`,
  `FrameInline` or `FrameOffset` and whose `Bytes` hold the item, or
  `$null` when the ring is empty; `RecvAnyFor` waits for one.
- `Remove-SubEthaUniversal` removes every file of a set.
- `Alignment` and `Tag` on `SubEtha.Slab`, `SubEtha.Vec`,
  `SubEtha.Region`, `SubEtha.Stack`, `SubEtha.Deque` and
  `SubEtha.LinkedList` hold the alignment and the tag the file's layout
  records, beside `ElementSize`, or `SlotSize` on a region, and `Writers`
  on a vector holds the size of its writer table, so each object carries
  every layout parameter its `Open-` cmdlet must be given.

#### Changed

- Errors end in SubEtha's error message where they ended in a variant
  name such as `LayoutMismatch` or `Full`.
- The channel, queue, map, lease, laned map and tower opens name the path
  in their errors, as the module's other opens already did.
- Opening a structure over a file of another shape, a pub-sub ring among
  them, fails with a message naming the field that differs.
- Reading `$shm:name` no longer waits on a value whose writer died before
  publishing it once the system has handed the writer's pid to another
  process: the store's arena records each writer's process start time
  beside its pid.
- The module is built with PoWerRuSt and cargo-pwrs 0.7.0, where 0.6.0
  was built with 0.3.2; building it from source still needs Rust 1.98.
  In Windows PowerShell 5.1, where the first PWRS module a session
  imports serves every later one, import it before a module built by
  cargo-pwrs 0.2.x through 0.6.x: after one, it fails with
  `pwrs_module_init failed with status 4`, as the troubleshooting page
  describes.

#### Removed

- `New-SubEthaRWLock`, `Open-SubEthaRWLock`, `New-SubEthaSemaphore`,
  `Open-SubEthaSemaphore`, `New-SubEthaOwnerLease` and
  `Open-SubEthaOwnerLease`, with the objects they returned.
- `Wait` on `SubEtha.LazyValue`. A caller that did not win `Claim` reads
  the value with `Get`, which never blocks and returns `$null` until the
  value is published.

#### Fixed

- `SubEtha.HolderTable`, `SubEtha.Heartbeat`, `SubEtha.FenceClock` and
  `SubEtha.Epochs` fail with an argument error for a slot past the
  capacity, naming the slot and the capacity, before the call reaches the
  table. The module handed such a slot straight to the core, so in a
  release build, which every published module is, the holder table, the
  heartbeat table and the epoch table read and wrote memory outside the
  table, and the fence clock panicked.
- `Claim` and `Publish` on `SubEtha.HolderTable` fail with an argument
  error for a payload of 0 or `[UInt64]::MaxValue`, the values the table
  keeps for a free slot and a forming one. Both panicked, and a claim
  left the slot it had reserved held; a refused claim now takes no slot.
- `SubEtha.Clock`, `SubEtha.CausalClock` and `SubEtha.FenceClock` refuse
  a step from a count already at `[UInt64]::MaxValue` with an argument
  error, leaving the clock as it was, where a release build wrapped the
  count to zero and returned a reading ordered before the input.
- `Observe` on `SubEtha.Timing` refuses a receive time earlier than the
  previous one with an argument error, which is the receiving clock
  stepping back rather than the link, and folds nothing in; a step back
  of 140 microseconds had moved the mean spacing from 62.5 microseconds
  to 1.15e18. A send time may still go back, for an item that arrived out
  of order.
- A load or a store on an atomic refuses an ordering it cannot take with
  an argument error naming the orderings the operation takes, as the C
  ABI refuses it, where the native library panicked: a load under release
  or acquire-release, and a store under acquire or acquire-release.
- `New-SubEthaChannel` and `Open-SubEthaChannel` round a capacity up to a
  power of two, as their documentation said they did; one that was not a
  power of two panicked inside `SharedRing::create`.
- `SendMany` on an adaptive queue returns how many items went in,
  stopping at a full queue, as it said; it waited for room instead.
  `New-SubEthaAdaptiveQueue` rounds its capacity up to a power of two; a
  capacity that was not one panicked in the ring underneath.

### C ABI

#### Breaking

- The cell takes any number of writers at once, none waiting on another,
  over the cell's new 4,096-byte layout. `subetha_cell_version` writes a
  `uint64_t` counting the writes that landed, one per write, where it wrote
  a `uint32_t` stepping twice, and `subetha_cell_stats.version` counts the
  same way. `SUBETHA_CELL_PAYLOAD_BYTES` is 56, where it was 52.
- Refusals that concern no file return `SUBETHA_E_INVALID_ARGUMENT`,
  `SUBETHA_E_OUT_OF_BOUNDS` or `SUBETHA_E_RING_PAYLOAD_TOO_LARGE` with
  the text, where they returned `SUBETHA_E_LAYOUT_MISMATCH`: a capacity
  of zero, a frame ring's slot under 16 bytes, a stack push longer than a
  slot, a versioned value of the wrong size, a lane past a map's lanes.
  `SUBETHA_E_LAYOUT_MISMATCH` stays for a stamped ring asked for frames
  and an unknown stamp kind.
- `subetha_ring_try_push`, `subetha_ring_try_push_many`,
  `subetha_ring_push_wait`, `subetha_capacity_try_push`,
  `subetha_capacity_push_wait`, `subetha_locale_ring_try_push` and
  `subetha_locale_ring_push_wait` take at most
  `SUBETHA_RING_PLAIN_ITEM_BYTES` (63) bytes in a shape built of
  single-producer cores, where they took `SUBETHA_RING_SLOT_BYTES`, and
  refuse more with `SUBETHA_E_RING_PAYLOAD_TOO_LARGE`: the slot's last
  byte holds the item's kind. A pop still yields the whole slot. `subetha_ring_recv_frame`
  and its waiting form finding a plain item next, on the ring or held for
  the consumer, leave it where it is and return the new
  `SUBETHA_E_NOT_A_FRAME` (135); a frame that fails its checks is taken
  off and refused with `SUBETHA_E_RING_IO`.
- `subetha_heartbeat_read_slot` writes `SUBETHA_HEARTBEAT_EMPTY_PID` for
  a slot whose register has not yet published, where it waited for the
  register, so neither it nor `subetha_epoch_barrier_live_peers` waits on
  a registrant that died mid-register; `subetha_heartbeat_register` on a
  full table takes back a slot whose registrant's process exited before
  publishing, even once the system has handed its pid to another process.
  The table's file has a new magic: `subetha_heartbeat_open`
  refuses a heartbeat file from an earlier SubEtha with
  `SUBETHA_E_LAYOUT_MISMATCH`.
- The slab, versioned slab and reservoir keep each element twice in its
  slot, behind a head a read checks, so `subetha_slab_get` and the
  reservoir's reads never wait on a writer, any number of processes may
  `subetha_slab_set` one slot with each write landing whole, and a writer
  that dies mid-write is taken over. Slots are larger, so `slot_stride` in
  `subetha_slab_stats` reads larger (a record twice behind 16 bytes, in
  whole 64-byte lines), and `subetha_slab_slot_version` still steps by two
  per write. Their files have new magics: opening one from an earlier
  SubEtha returns `SUBETHA_E_LAYOUT_MISMATCH`.
- `subetha_vec_create`, `subetha_vec_open`, `subetha_vec_open_read_only`
  and `subetha_vec_reset` take `writers`, the size of the vec's writer
  table, after `capacity`: one entry per handle writing the vec at once.
  An open naming another size returns `SUBETHA_E_LAYOUT_MISMATCH` naming
  the writer slots. `subetha_vec_stats` ends with `writer_slots`, the size
  an open must be given, so it is 8 bytes longer; its `len` counts the
  slots below the length, each an element or a hole, and `slot_stride`
  reads 64 for an element of at most 8 bytes at an alignment of at most 8
  and 128 for one of up to 56. Pushes, pops and sets never wait on
  another writer. `subetha_vec_get`, `subetha_vec_get_many` and
  `subetha_vec_set` refuse a hole with `SUBETHA_E_OUT_OF_BOUNDS`, the
  detail naming the index below the length, and `subetha_vec_clear` is
  safe against a push or a pop in any process. Opening a vec file from an
  earlier SubEtha returns `SUBETHA_E_LAYOUT_MISMATCH`.
- `subetha_lazy_claim` takes over a claim whose process is gone, judged
  by its pid and process start time, where such a claim stood until
  `subetha_lazy_reclaim` broke it. `subetha_lazy_open` on a file from an
  earlier SubEtha returns `SUBETHA_E_LAYOUT_MISMATCH`.
- Holder tables, epochs, arenas, laned maps and a ring's holders record
  each holder's process start time beside its pid, so their dead-holder
  reaps free a slot whose pid the system has handed to another process;
  opening one of their files from an earlier SubEtha returns
  `SUBETHA_E_LAYOUT_MISMATCH`.
- The stack, region and B-tree keep the record of a spent slot counter in
  their free-list head, so a full one refuses an allocation without
  touching the counter. Their files have new magics: opening one from an
  earlier SubEtha returns `SUBETHA_E_LAYOUT_MISMATCH`.
- The list's functions take and return a node's handle as `uint64_t`,
  the slot in the low half and its generation in the high half, where
  they used `uint32_t`; a handle used after its node was removed or popped
  is refused.

#### Added

- `subetha_locale_ring_unregister_producer` and
  `subetha_locale_ring_unregister_consumer` give an id back on all three
  backings, and the id is handed out again; an id past the table is
  `SUBETHA_E_INVALID_ARGUMENT`.
- `subetha_cell_load` copies the value with the generation that made it
  current, and `subetha_cell_compare_and_set` writes only if the
  generation is still the one given, returning `SUBETHA_E_WRITE_CONFLICT`
  with the generation a write that landed first made when it is not.
  `SUBETHA_E_WRITE_CONFLICT` now names any write a newer version beat.
- The shared versioned index: `subetha_versioned_index_create`,
  `_open`, `_write`, `_delete`, `_void`, `_void_epoch`, `_get`, `_range`,
  `_changed_since`, `_sweep`, `_version_count`, `_live_count` and
  `_read_stats`, with `SUBETHA_KIND_VERSIONED_INDEX`,
  `SUBETHA_VERSIONED_INDEX_CHUNK_BITS_DEFAULT` and
  `SUBETHA_E_WRITE_CONFLICT` (134). An index is built from an epoch table
  handle and takes its pin and ticket tokens from that handle alone. A
  write that a version born after the pin beat returns
  `SUBETHA_E_WRITE_CONFLICT` with that version's epoch in `out_conflict`.
  Keys order as unsigned bytes, at the sizes given at create.
- `subetha_ring_recv_any` and `subetha_ring_recv_any_wait` take the next
  item, a plain item or a frame, writing its length to `out_len` and its
  kind to `out_kind`: `SUBETHA_RING_ITEM_PLAIN`,
  `SUBETHA_RING_ITEM_FRAME_INLINE` or `SUBETHA_RING_ITEM_FRAME_OFFSET`.
  An item larger than the buffer is held for the consumer, with its size
  in `out_len` and its kind in `out_kind`, as a frame already was.
  `SUBETHA_RING_PLAIN_ITEM_BYTES` names the largest plain item.
- The front door's channel: `subetha_channel_create`,
  `subetha_channel_open`, `subetha_channel_try_send`,
  `subetha_channel_send_wait`, `subetha_channel_try_recv`,
  `subetha_channel_recv_wait`, `subetha_channel_try_send_many`,
  `subetha_channel_try_recv_many` and `subetha_channel_unlink`, with
  `SUBETHA_CHANNEL_ITEM_MAX` (52) and `SUBETHA_KIND_CHANNEL`. A channel
  made from C, Python or PowerShell at a path is one channel to all
  three: an item keeps its length, and a capacity rounds up to a power of
  two as the other surfaces round it. The batch forms take a length for
  each item sent and report one for each item received, the first C batch
  calls to carry lengths. A wait ends with `SUBETHA_E_DESTROYED` when its
  handle is destroyed, within one heal tick of the channel's blocking
  call.
- The front door's key-value map, work queue and adaptive queue, and the
  reorder window on its own: `subetha_kv_map_` (create, insert, get,
  `insert_many`, `get_many`, `read_stats`, unlink), `subetha_work_queue_`
  (create, `steal_from`, push, pop, steal, `push_many`, `steal_many`,
  `read_stats`, unlink), `subetha_adaptive_queue_` (create, `try_send`,
  `send_wait`, `try_recv`, `recv_wait`, `try_send_many`, `try_recv_many`,
  `maybe_change_shape`, `change_shape_to`, `set_ordering`, `read_stats`,
  unlink) and `subetha_reorder_window_` (create, push, take, flush,
  `push_many`, `flush_many`, `widen_to`, `read_stats`), with the kinds 74
  to 77. A map or work queue made from C, Python or PowerShell at a path
  is the same to all three, its capacity rounded up to a power of two as
  the others round it. The adaptive queue's ordering takes
  `SUBETHA_QOS_PER_PRODUCER` or `SUBETHA_QOS_GLOBAL_FIFO`, its
  `auto_order` takes `SUBETHA_AUTO_ORDER_NEVER` or a rate, and in managed
  mode its scanner changes its shape; a thief's push or pop on the work
  queue returns `SUBETHA_E_DEQUE_NOT_OWNER`.
- Sidecar registration: `subetha_observe`, `subetha_unobserve` and
  `subetha_observation_stats`, which fills a `subetha_instance_stats`
  field for field as Rust's `InstanceStats`, and the sidecar-wide
  `subetha_sidecar_scan_now`, `subetha_sidecar_instance_count`,
  `subetha_sidecar_max_instances`, `subetha_sidecar_set_max_instances`
  and `subetha_sidecar_node_count`. A registration lives inside the
  object its handle names and ends when the handle is destroyed, before
  the object goes. The hash map, the two Bloom filters, the bit vector,
  the broadcast ring, the epoch barrier, the heartbeat table, the
  HyperLogLog, the count-min sketch, the histogram, the rate limiter, the
  topology map, the string arena, the
  leader election, the fence clock, the three atomics, the reservoir
  sampler, the versioned chain, the Vyukov ring and the adaptive ring
  record observations through it. An adaptive ring records the pops that
  undercut a stamp, an ordered receiver's among them, so a receiver is
  observed through its ring's handle; any other kind returns
  `SUBETHA_E_NOT_SUPPORTED`.
- Custom policies and the adaptive object.
  `subetha_observe_with_policy` registers any observable object under a
  policy written in C: `subetha_policy_decide` takes the caller's context,
  the statistics and the current tag, and returns `SUBETHA_POLICY_STAY`
  or `SUBETHA_POLICY_MOVE` with the new tag in an out-parameter; any
  other value it returns is counted, as
  `subetha_observation_policy_errors` reads, and leaves the tag alone.
  `subetha_unobserve` and `subetha_handle_destroy`
  return only once no scan is inside `decide`, an optional
  `subetha_policy_release` is called once after the last `decide`, and a
  `decide` that would end its own registration is refused.
  `subetha_adaptive_` (create, record, tag, `set_tag`), kind
  `SUBETHA_KIND_ADAPTIVE`, is Python's `Adaptive`: a strategy tag and a
  ring of the caller's own operations.
- The handle table, `subetha_handle_table_` (create, open, reset, insert,
  get, remove, contains, `insert_many`, `get_many`, `read_stats`, flush,
  `flush_async`, unlink), kind `SUBETHA_KIND_HANDLE_TABLE`: values of up
  to `SUBETHA_HANDLE_TABLE_VALUE_MAX` (44) bytes reached by the same
  `uint64_t` handle Python's `HandleTable` and PowerShell's
  `New-SubEthaHandleTable` return, over the same file. A handle whose
  value was removed, or whose slot has been reused, returns
  `SUBETHA_E_MAP_KEY_ABSENT`, a full table `SUBETHA_E_MAP_FULL`, and a
  remove refused for a short buffer leaves the value in the table. The
  table is observed through `subetha_observe`.
- The eight link estimators, each an in-process handle of its own kind
  (`SUBETHA_KIND_LINK_LOSS_KIND` through
  `SUBETHA_KIND_LINK_PATH_CHANGES`): `subetha_link_loss_kind_`,
  `subetha_link_loss_bursts_`, `subetha_link_timing_`,
  `subetha_link_round_trip_shape_`, `subetha_link_periodicity_`,
  `subetha_link_capacity_`, `subetha_link_forecast_` and
  `subetha_link_path_changes_`, with the observe calls Python's sensors
  have, batch forms where Python has `observe_many`, and
  `subetha_link_loss_kind_classify` returning `SUBETHA_LINK_LOSS_WIRELESS`
  or `SUBETHA_LINK_LOSS_CONGESTION`. Each reads in one call into a
  `subetha_link_*_reading` filled under the sensor's claim; a value that
  needs enough data first carries a `has_` flag and reads 0.0 while it is
  false.
- TinyBloom and FineBloom as values the caller holds, as the NaN-boxed
  values are: `subetha_tiny_bloom_` (insert, contains, `insert_many`,
  `contains_many`, `set_bits`, `false_positive_rate`) over a `uint64_t`,
  and `subetha_fine_bloom_` (insert, contains, `insert_many`,
  `contains_many`) over a `subetha_fine_bloom` of four words, with
  `SUBETHA_TINY_BLOOM_SUGGESTED_CAPACITY` (8) and
  `SUBETHA_FINE_BLOOM_SUGGESTED_CAPACITY` (32). A key is hashed as its
  bytes, as the Python classes hash it, so a word built from the same keys
  is the same word.
- The hybrid logical clock and the causal clock as values the caller
  holds, as the Bloom filters are: `subetha_hlc_` (now, advance, merge,
  compare) over the `subetha_hlc` a fence clock already returns, so a
  fence clock's reading is advanced, merged and compared directly, and
  `subetha_causal_clock_` (tick, merge, compare) over a
  `subetha_causal_clock` of `SUBETHA_CAUSAL_CLOCK_NODES` (16) counts,
  whose comparison returns `SUBETHA_CAUSAL_BEFORE`,
  `SUBETHA_CAUSAL_EQUAL`, `SUBETHA_CAUSAL_AFTER` or
  `SUBETHA_CAUSAL_CONCURRENT`.
- `SUBETHA_E_LAYOUT_MISMATCH` (102) and `SUBETHA_E_IO` (107): the codes
  every family returns for a backing of another layout and for the
  operating system's refusal, under names that are not a ring's.
  `SUBETHA_E_RING_LAYOUT_MISMATCH` and `SUBETHA_E_RING_IO` stay, with the
  same values.
- `subetha_universal_unlink(base_path, report)` removes every file of the
  set at `base_path` and fills a `subetha_unlink_report`.

#### Changed

- The C ABI takes no lock. Everything in `subetha-ffi` that serialized
  calls behind `parking_lot::Mutex` (the handle table's issue list, the
  borrow guard's registry, init and shutdown, the last panic message, the
  link estimators, the rings, channels and clocks, the bridges and the
  Sens-O-Matic halves) does so through `Claim`, and a handle's panic
  message is a lock-free cell read without waiting. `subetha-ffi` no
  longer depends on `parking_lot`, and the workspace's `disallowed-types`
  lint holds for it as for every other crate.
- A failing call that was given a path starts its detail with the path,
  quoted: `'map.bin': a map of another shape: the file's capacity is 1024
  where 512 was asked for`. A call given two paths, such as a map and its
  epoch table, names both.
- A failed producer registration's growth-failed detail says why growing
  the ring failed.

#### Removed

- The reader-writer lock (`subetha_rwlock_*`, `SUBETHA_KIND_RWLOCK` 35),
  the counting semaphore (`subetha_semaphore_*`, `SUBETHA_KIND_SEMAPHORE`
  37) and the owner lease (`subetha_owner_lease_*`,
  `SUBETHA_KIND_OWNER_LEASE` 40).
- `subetha_lazy_wait`, `subetha_lazy_reclaim` and
  `SUBETHA_E_LAZY_CLAIMANT_GONE`, whose code 132 no new code takes. A
  caller that did not win `subetha_lazy_claim` reads the value with
  `subetha_lazy_try_get`, which never blocks and returns
  `SUBETHA_E_LAZY_NOT_CLAIMED` until the value is published.

#### Fixed

- A panic caught at the C boundary could poison the next object issued in
  its handle's slot. The mark checked the handle's generation and then
  marked the slot without holding it, so a destroy and a fresh create
  between the two gave the new handle the old panic, and every call on it
  failed with `SUBETHA_E_HANDLE_POISONED`. The mark is now made with the
  slot held as a call holds it, so a destroy waits for the mark to finish.
- `subetha_sens_stats.loss` read a negative number on a sender with no
  loss estimate yet, though its documentation said zero to one; the
  estimate exists only once the receiver's reports, from a second into
  the link, have covered 30 datagrams the sender sent. The struct
  carries `has_loss`, false until then and on every half but a unified
  sender, and `loss` reads 0.0 while it is false, the cases in which
  Python's `loss` returns `None` and PowerShell's `Loss()` `$null`. The
  flag occupies padding after `code`, so the struct is still 104 bytes
  with every other field at its earlier offset: a program built against
  the earlier header runs unchanged, reading 0.0 where it read a
  negative number.
- A failing C call leaves a detail that describes it. About 120 returns
  gave a code without writing the thread-local detail, so
  `subetha_last_error_detail` could hand back an earlier call's text: a
  hash map opened at the wrong capacity reported the destroyed handle of
  the call before it. Every entry point now finishes a failing call by
  giving it the code's own name, as `subetha_strerror` spells it, when the
  call wrote no specifics of its own; a refusal on a polling path stores
  that name without allocating, and a repeated one writes nothing. I/O
  failures carry the operating system's message. A refusal costs about
  0.5 ns more on a Ryzen 9 7900X under Windows 11 (an empty ring's
  `subetha_ring_try_pop` measured 13.7 ns without the step and 14.2 ns
  with it); the C ABI bench gains rows for an empty pop and a missing key.
- `subetha_ring_open_shm`, `subetha_vyukov_open_shm`,
  `subetha_pubsub_open_shm` and `subetha_broadcast_open_shm` return
  `SUBETHA_E_LAYOUT_MISMATCH` for a named region shorter than they ask
  for, with both lengths in the detail, where they returned
  `SUBETHA_E_IO`; Windows reported the refusal only as access denied. A
  Windows section is a whole number of pages, so the length it reports is
  the size it was created with rounded up to a page.
- `subetha_fence_clock_tick` and `subetha_fence_clock_merge` return
  `SUBETHA_E_INVALID_ARGUMENT` for a step from a count already at
  `UINT64_MAX`, leaving the clock as it was, where a release build
  wrapped the count to zero and returned a reading ordered before the
  input.
- `subetha_fence_clock_tick` and `subetha_fence_clock_merge` refuse a
  null `out` before they step the slot, where they stepped it and then
  returned `SUBETHA_E_INVALID_ARGUMENT`.

### Documentation

#### Added

- A reference page,
  [Shared Versioned Index](https://variably-constant.github.io/SubEtha/docs/reference/subetha-cxc/maps/shared-versioned-index/),
  for the lock-free versioned map: its files and chunk files, writes and
  conflicts, reads at a pin, reclamation, recovery from a writer that
  dies, its errors and what its tests verify. The map's family page, the
  catalog and the index of every type list it.
- The Guide's examples run as tests. A Rust, Python, PowerShell or C
  block that names a file is a program; each runs as a process of its
  own, a page's programs in page order from one fresh folder and for at
  most 60 seconds, and what it prints is compared with the Output block
  the page shows after it. The unpublished `subetha-wiki-tests` crate
  compiles every Rust and C program into one test binary (the C with the
  host's compiler, every warning an error) and runs them; the Python
  package's suite runs the Python programs and the PowerShell module's
  suite the PowerShell ones. The same crate checks every line of a Rust
  listing against the crates' source, and every SubEtha package name and
  version a command states against the workspace.
- The installation tutorial's smoke test has a C tab, and the Guide's
  tests run its four programs, each naming its file relative to the
  folder it runs from; the Rust one depends on the umbrella crate.
- A tutorial,
  [Send items between processes](https://variably-constant.github.io/SubEtha/docs/tutorial/send-items/),
  in Rust, Python, PowerShell and C: a channel one process sends into and
  another reads after it has gone, a reader that waits for items a
  second process sends a fifth of a second apart, and a ring where a
  producer and a consumer each register for an id and the consumer waits
  for jobs. The Guide's tests run all twenty-four programs.
- A tutorial,
  [Share a counter](https://variably-constant.github.io/SubEtha/docs/tutorial/share-a-counter/),
  in Rust, Python, PowerShell and C: two processes add to one atomic
  counter with no lock, a compare-and-exchange loop hands out five
  tickets across processes, a flag raised with release ordering and
  lowered with acquire ordering publishes a result, and a 32-bit word
  holds a set of bits. It closes with each surface's names for the three
  widths and how each names a memory ordering. The Guide's tests run all
  twenty-four programs.
- A tutorial,
  [Claim a role across processes](https://variably-constant.github.io/SubEtha/docs/tutorial/claim-a-role/),
  in Rust, Python, PowerShell and C: one process claims a role through
  the leader election, beats to keep it and exits without stepping down;
  a standby's claim is refused at once while the holder's beat is fresh,
  and once it steps the epoch past the grace period it takes the role
  over. The page's earlier address, `tutorial/hold-a-lock/`, leads to it.
  The Guide's tests run all eight programs.
- A tutorial,
  [Move many items per call](https://variably-constant.github.io/SubEtha/docs/tutorial/move-many-items/),
  in Rust, Python, PowerShell and C: a thousand items through a ring in
  one call each way, then sent and received as one buffer packed end to
  end, with the measured cost of a call on each surface and why Rust
  needs neither shape. The Guide's tests run all eight programs.
- A tutorial,
  [Read a failure](https://variably-constant.github.io/SubEtha/docs/tutorial/read-a-failure/),
  in Rust, Python, PowerShell and C: each surface attaches to a map built
  at another size and to one that is not there, offers a key of the
  wrong size, and reads what comes back, from the `Mismatch` a Rust error
  carries and the `io::Error` its `source()` returns to the C detail. It
  then shows the outcomes that come back as values rather than errors,
  and how each surface reports a panic. The Guide's tests run all
  seventeen programs.

#### Changed

- Each published crate compiles its README's Rust examples as doctests,
  so `cargo test` fails on an example that stops compiling.
- The cross-process waker's page says the `UMONITOR`/`UMWAIT` path has
  run on an emulated Sapphire Rapids under Bochs, the wait path's tests
  passing with the monitor tier on and off, and what Bochs does not
  model; no Windows host with WAITPKG has run it.
- The wait calibration page says a process takes the host's last load
  band from the cache before its first reading, which comes 100 ms after
  the intrinsic facts and then every second, and that a record's
  temporary file carries its writer's pid and start stamp, the files of
  writers that are gone being removed at the next write.

#### Removed

- `crates/subetha-cxc/docs/CROSS_PLATFORM_BENCHMARKS.md`, an older copy
  of the Guide's cross-platform benchmarks page; the Guide's page is the
  one kept.

#### Fixed

- Documentation that disagreed with the code or its measurements. The
  README's two cost figures had no recorded source and are re-measured
  on a Ryzen 9 7900X under Windows 11: `SharedRing::try_push` then
  `try_pop` on one thread at 10.5 ns, and the `SharedRing` SPSC fast
  path at 100.2 M items/s. Its 4-producer / 4-consumer claim now matches
  the competitor results (`crossbeam_channel` leads on macOS and
  `flume` on the EPYC VPS). The Guide's front page reports the same
  six-host cross-process figures as the README in place of an earlier
  five-host capture. The `subetha-cxc` README's ring example no longer
  pops into a buffer smaller than `PAYLOAD_BYTES`, which panicked, and
  its tables name only types that exist. The `api` module's
  constructors say what they do with a capacity that is not a power of
  two, which only `AutoIpc::capacity` rounds.
- The `subetha-ffi` README states the boundary cost its
  own table gives (8 to 22 ns a pair) and the thirty-two C peers a test
  run starts; the `subetha-core` README lists its seventh module,
  `swap_cell`; and the `subetha-sidecar` README says a policy is asked
  after each scan that drained something, with one scan thread per NUMA
  node on Windows and one elsewhere.
- The Python and PowerShell READMEs list everything the bindings carry,
  the sidecar surface 0.6.0 added among it, and the Python one names the
  front door, the stack, the deque, the LRU cache and the epoch table.
  The PowerShell README counts its 139 cmdlets and the 227 Pester tests
  that pass on every platform. The Python binding's own documentation
  gives its measured call cost, an atomic load at 30 to 37 ns against
  7.1 ns through the C ABI, in place of a ratio no measurement of it
  supports.
- The substrate-axes design note names `qos_policy::Ordering::GlobalFifo`,
  the locales the code has (anon, file and shmfs), the ring type the
  pinned chain and the QUIC bridge hold (`AdaptiveRing`, not
  `SharedRing`), and `SharedDeque` and `SharedHashMap` as file-backed
  only; the compression note names both erasure codes the link switches
  between.
- The unified Sens-O-Matic endpoint's module documentation gives the
  thresholds its default policy uses, up to RS at about 15% loss and back
  to RLC at about 10%, where it gave a 23.5% switch the code no longer
  makes. The transport comparison's internet range starts at the
  258 Mbit/s it measured, and the Sens-O-Matic performance page sends
  `--no-pace` and `--no-proactive` to `bridge_lan`, the harness that has
  them.
- `SharedRing` is named without a type parameter it does not have, its
  module documentation describes the slot as the sequence and payload it
  holds, the catalog gives `fd_handoff`'s Windows functions, and the
  wire locale is named as such rather than as a `Locale::Wire` variant
  that does not exist.
- The installation tutorial counts the primitives and pointer families as
  the other pages do, shows the whole toolchain file, and changes into
  the `SubEtha` directory the clone makes; the cross-process tutorial
  registers the Python and PowerShell maps with the sidecar through
  `observe()`, which it said they could not do. The observations
  tutorial's Rust example compiles, and it describes the 24-byte
  observation record's packing correctly. The C and C++ tutorial
  says the header declares its functions `extern "C"` for a C++
  compiler itself.
- The architecture page describes the sidecar as it runs. It keeps
  statistics for every `subetha-cxc` primitive, each of which registers
  a policy that never migrates, and the adaptive primitives change
  strategy through scanners of their own (`AdaptiveRingSidecar`,
  `CapacityAdaptiveRingSidecar`, `LocaleAdaptiveRingSidecar`,
  `AdaptiveIpcSidecar`, `UnifiedSidecar`). Its scan takes no lock, NUMA
  nodes are detected on Windows only, and the page no longer says each
  op enters and exits the handshake header, which no shipped primitive
  does. It counts more than sixty primitives and seven end-to-end
  scenarios, and the `InstanceStats` reference says a scan that drained
  nothing asks no policy.
- The exotic pointers are described as what they are on the
  architecture page, the Guide's front page and the `subetha-pointers`
  reference: none implements `Marshal`, all but `KTower2` and `KTower3`
  hold an address valid only in the process that made it, the
  capabilities check bounds and permissions in software, and
  `BloomPointer` hashes the query without touching its target. The
  `subetha-pointers` README says `StridedIter` multiplies the index by
  the stride and that most of its pointers declare a `SIGNATURE`; the
  `subetha-core` README says `MigrationGuard` drains only when asked
  and that `subetha-ffi` builds on `subetha-core` as well.
- The README quickstart and the Guide's installation page depend on the
  umbrella crate `subetha` (`use subetha::cxc::AutoIpc`) where they named
  `subetha-cxc`, and the installation page has a C tab. Rust examples
  written `rust,no_run` highlight as Rust, the colon of a PowerShell
  drive variable such as `$shm:greeting` renders as code rather than as
  a highlighted error, and a leftover page listed as "404th" in the
  Guide's contents is gone.
- `SharedCondvar::create` and `subetha_condvar_create` are documented as
  attaching to an existing condition variable with its live generation,
  where the header said the generation starts at zero.
- `New-SubEthaChannel`'s help said it created the channel only when the
  file did not exist. It makes the file afresh, discarding a channel
  already there, and says so; `Open-SubEthaChannel` joins one.
- The cross-process tutorial shows its producer and consumer in Rust,
  Python, PowerShell and C, and the Guide's tests run all eight. Its Rust
  programs named `/tmp/subetha-roundtrip.bin`, which on Windows is a
  `\tmp` folder at the root of the drive that seldom exists; every
  program names the file relative to the folder it runs from.
- The Reading observations tutorial registers a hash map and reads what
  the sidecar counted in Rust, Python, PowerShell and C, and the Guide's
  tests run all four programs. Its map holds 2048 entries for the 1000 it
  takes, twice the entries as the hash map's documentation asks, where
  the page sized it at 1024.
- Python's `Forecast.mean_rate` and `next_rate` and PowerShell's
  `MeanRate` and `NextRate` said bits a second; they return bytes a
  second, as `ArrivalForecast` computes them.
- Python's `Timing.clock_skew` and PowerShell's `ClockSkew` said they
  return how far the two clocks differ in microseconds; they return how
  fast the receiving clock runs against the sending one, in microseconds
  gained per microsecond, the units of the trend, as
  `TemporalSensor::skew` computes it.

## [0.6.0] - 2026-10-01

### Added

- The Python package and the PowerShell module reach the process's
  sidecar as Rust and the C ABI do. Twenty-six structures, and a new
  `Adaptive` object, register with it through `observe()` in Python
  and `Observe()` in PowerShell, which return a registration reading
  the sidecar's `InstanceStats` and the object's tag; closing it, or
  its collection, unregisters the object. A registration may carry a
  policy, a Python callable or a PowerShell ScriptBlock taking the
  stats and the current tag, which the sidecar asks on its scan thread
  after every scan that drained something new; one that raises or
  answers anything but a tag is counted, and its exception or error
  record kept. `Adaptive` is an object of the caller's own: it records
  the caller's operations and runs at the tag its policy answers.
  `subetha.sidecar`, and `Get-SubEthaSidecar`, `Set-SubEthaSidecar`
  and `Invoke-SubEthaSidecarScan`, read the instance count, move the
  cap and scan now. `Ring`, `CapacityRing` and `LocaleRing` take
  `managed` / `-Managed`, which starts the ring's own sidecar at
  `scan_interval_us` / `-ScanIntervalUs` (for a `Ring`, 250
  microseconds when not given; the other two need it), and report the
  morphs, prewarms and migrations that sidecar made; a managed
  `LocaleRing` moves where `request_locale` / `RequestLocale` asks.
  See the
  [Python](https://variably-constant.github.io/SubEtha/docs/reference/subetha-py/)
  and
  [PowerShell](https://variably-constant.github.io/SubEtha/docs/reference/subetha-pwrs/)
  references and
  [Write a custom Policy](https://variably-constant.github.io/SubEtha/docs/how-to/custom-policy/).

- `subetha_cxc::adaptive_ring::SCAN_INTERVAL_DEFAULT_US`, 250: the
  shape sidecar's cadence when a managed ring's caller names none. The
  C ABI's `SUBETHA_SCAN_INTERVAL_DEFAULT_US` is held equal to it at
  compile time.

- `FailoverWatchdog::watch_ring` hands the watchdog `SharedRing`s to
  heal. Each `scan` heals a slot a producer claimed and never
  published when the previous scan found it stuck at the same position
  and some registered process's last beat is more than `grace_epochs`
  behind; `ReclaimReport::healed_slots` lists what the scan healed and
  `heal_errors` any heal a ring refused. A publish takes nanoseconds,
  so a live producer is not mid-publish across a scan interval.
  `ReclaimReport` gains the two fields and `FailoverWatchdog` a private
  one, so neither is built with a struct literal any more.

- `SwapCell<T>` in `subetha-core` holds one `Arc<T>` that any thread
  reads, replaces, or replaces only while the cell still holds the
  value it expects. A read changes no reference count, and a replaced
  value is dropped when its last reader lets go. `SwapCellOption<T>` is
  the same cell over an `Option<Arc<T>>`. See
  [SwapCell](https://variably-constant.github.io/SubEtha/docs/reference/subetha-core/swap-cell/).

- `HolderTable::release_if` and `SharedHolderTable::release_if` free a
  slot only while it still holds the process and payload the caller
  observed. A claimant that finds a claim stale, its pid now naming
  another process as a creation time in the payload shows, frees it
  without freeing a holder that claimed the slot since, which
  `release` and a pid-only `reap_dead` cannot do.

- Blocking waits take their spin, monitor and park lengths from the
  host's measured costs. The first blocking wait in a process starts a
  background thread that measures, for each class of core, the
  counter's rate, a spin hint's cost and how soon a monitor wait can
  return, and for each load band it meets, the wake latency of a spin,
  the monitor and the park. A wait then spins to the monitor's break-even and
  monitor-waits to the park's, or spins to the park's break-even where
  the host has no monitor; a band not yet measured keeps 0.5.1's fixed
  ladder. The facts are cached per user, so a later process measures
  nothing, and `wait_calibration::report()` says what was measured,
  where it came from and what it cost. `SUBETHA_WAIT_CALIBRATION=off`
  keeps the fixed ladder and measures nothing, and
  `SUBETHA_PRE_PARK_SPIN` sets the spin's rounds. The calibrated wait
  is held within 2x of the better of a pure spin and a pure park at
  every gap from 1 to 1024 us and below the spin summed: 0.78 to 1.83
  times the better fixed arm on an Ubuntu guest with no monitor
  family, 0.80 to 1.90 on a FreeBSD guest, and on a Windows 11 / Ryzen
  9 7900X host with 23.85 to 24.00 of its 24 logical processors busy
  with other work it cost 0.47 to 0.81 of the fixed ladder summed over
  the gaps. See
  [Wait calibration](https://variably-constant.github.io/SubEtha/docs/reference/subetha-cxc/coordination-types/wait-calibration/).

- `examples/wait_verdict.rs` runs the comparison the calibrated wait is
  held to: each arm in a process of its own, a pure spin, a pure park,
  the fixed ladder and the calibrated plan, at gaps of 1 to 1024 us on
  an anonymous ring and on a file-backed ring across two processes, with
  spin controls, load readings and a co-runner pass on the waiter's SMT
  sibling. Each gap is read round by round, and reads undecided when its
  rounds fall on both sides of the bound; `wait_verdict --read <file>`
  prints a saved run's verdict again.

- `SharedHashMap::swap` and `RawHashMap::swap` insert or replace a key
  and hand back the value they replaced, read and overwritten under the
  slot's lock, so a value leaves the map exactly once: through `swap`,
  `remove`, or a `compare_exchange` that succeeds.

- `RawArena`: power-of-two blocks carved from one shared file, handed
  to a writer, published, retired under a `SharedEpochs` epoch and
  reused once no pin can see them. Each class of block has a free
  bitmap and a retired bitmap, so no step can lose a block or list it
  twice, and `collect` walks the block headers with the caller's roots
  to recover what a process that died left behind: an unbuilt chunk, a
  half-written block, a replaced block nobody retired. See
  [Raw Arena](https://variably-constant.github.io/SubEtha/docs/reference/subetha-cxc/arenas/raw-arena/).

- `SharedNamedValues`: byte values of any size under case-insensitive
  names, over a `RawHashMap`, a `RawArena` and a `SharedEpochs` table in
  three files of one stem. A name keeps the spelling it was first
  written with; two names whose 128-bit hashes collide are refused,
  never merged. See
  [Shared Named Values](https://variably-constant.github.io/SubEtha/docs/reference/subetha-cxc/maps/shared-named-values/).

- AppContainers. `ShmNamespace::AppContainer(sid)` names a region in an
  AppContainer's named-object directory from outside the container, and a
  process inside it reaches the region with `ShmNamespace::Session`, since
  inside a container `Local\` is that directory. `ContainerSid` holds the
  SID inline, so the namespace stays `Copy`; it parses from `S-1-15-2-...`
  and, on Windows, derives from the container's name. An `AdaptiveRing`
  and its `NotifierSet` created in that namespace put every region and
  event there, including a producer slot grown later, the payload region
  of the first offset frame, and the notifier events. The directory exists
  only while a process of the container runs, so the creator starts it
  suspended, creates, and resumes it. On Windows 11 / Ryzen 9 7900X a
  process started in a container attaches a ring and a notifier created
  that way, is signaled, and receives a frame from a slot grown after it
  attached and one carried by a payload region made after it attached;
  with the ring created in the session namespace instead, the attach fails
  with `LayoutMismatch`. Through the C ABI the namespace is
  `SUBETHA_SHM_APPCONTAINER`, with the SID in `shm_container_sid`, a field
  added at the end of `subetha_ring_options`. See
  [Shm File](https://variably-constant.github.io/SubEtha/docs/reference/subetha-cxc/specialized/shm-file/#appcontainers).

- `ShmFile::create_named_secured` for the creator of a region,
  `ShmFile::open_named_in` / `open_named_secured` for a region that must
  already exist, `ShmFile::owns_name`, and `ShmFile::keep_name`, which
  gives a handle's name up so its drop leaves the region for whatever
  removes it by name; `PeerDirectory::create_shm_secured` and
  `PeerDirectory::open_shm_secured` the same for the peer directory.

- `AdaptiveRing::unlink_shmfs` and `unlink_shmfs_in` remove every name a
  shm-backed ring holds, the per-producer backings it grew included, as
  `unlink` removes a file-backed ring's files, and `UnlinkReport::remove_shm`
  removes one name into a report. `with_last_holder` takes holds on a
  shm-backed ring too, with its holder table in the ring's namespace. Through
  the C ABI, `subetha_ring_unlink_shm` removes the ring's names, its two
  wakers, its notifier record and the notifier FIFOs a consumer that died
  left behind, and `max_holders` accepts a shared-memory ring. See
  [the ring's lifetime](https://variably-constant.github.io/SubEtha/docs/reference/subetha-cxc/rings/shared-ring-adaptive/#lifetime).

- PowerShell: the `shm:` drive. `$shm:name = value` publishes a value
  every process of the user reads as `$shm:name`, live, outliving the
  session, in PowerShell 7 and Windows PowerShell 5.1 alike. Booleans,
  integers, singles, doubles, strings, byte arrays and DateTimes come
  back as their own type; anything else comes back as remoting returns
  it, through CLIXML at depth 2. `Get-ChildItem shm:` lists the names,
  `Remove-Item shm:name` or assigning `$null` removes one, and
  `New-PSDrive -PSProvider SubEthaShm -Root <directory>` opens the
  files of another directory. See
  [The shm: drive](https://variably-constant.github.io/SubEtha/docs/reference/subetha-pwrs/shm-drive/).

- On Windows a waiter on a file- or shm-backed `CrossProcessWaker`, and
  so every cross-process blocking ring, channel, condvar, lock and
  semaphore wait, sleeps in the kernel once its monitor budget runs out,
  on a named event the waker sets. An idle waiter was charged a
  processor for as long as it waited, 0.953 to 1.008 of a core, and is
  now charged 0.000 to 0.003; a round trip with a 100 microsecond hold
  pays 4.0 microseconds more at p50. Measured on Windows 11 / Ryzen 9
  7900X. A waiter re-checks its slot at least every 20 ms, so a wake
  whose event is never set still ends the wait. The waker's tests pass
  on Windows 11 ARM64 (Azure Cobalt 100) too, with the monitor tier on
  and off. No processor with WAITPKG has run them: under Intel SDE's
  Sapphire Rapids model the waker takes the `UMONITOR`/`UMWAIT` path,
  and SDE does not execute `UMONITOR`.

- The release gate, `cargo run -p xtask -- gate`, lints and tests every
  crate at its defaults and with every feature it declares, each crate
  in a build of its own, and runs the Python package's suite as it
  ships and with every feature. The gates before it built no
  feature-gated code: none of the C API's transports, the Python bridge
  classes, or subetha-cxc's `tls`, `tcp-tls-bridge`, `residue-fec`,
  `wire-locale`, `linux-futex-raw` and benchmark features, and not
  subetha-cxc with no features at all. That is how the C bridge servers
  and the `wire-locale` link fixed below shipped broken in 0.5.1. The
  C suite's `transports` feature runs it against a library carrying
  every transport. What the gate leaves off, on which host and why, is
  in [Run the release gate](https://variably-constant.github.io/SubEtha/docs/how-to/run-the-release-gate/).

- The macOS module workflow runs the Pester suite under a watchdog
  before `cargo pwrs test`, which prints a host's output only once the
  host exits, so a test that never returned named nothing: one run sat
  silent for 45 minutes. Each test is printed as it finishes. A suite
  still running after 600 s has its threads sampled and its UDP sockets
  listed before it is stopped, and the file it stopped in runs again by
  itself. The job ends at 30 minutes.

- `SensOMaticRlcSender::retransmits()` counts the source symbols the
  sender sent again, whether a NAK asked, the RTO on a stalled
  cumulative ACK, or the end-of-stream tail.

### Changed

- On a managed locale ring, `subetha_locale_ring_migrate` also sets
  the locale the ring's sidecar is asked to keep, as
  `subetha_locale_ring_request` does, so the sidecar holds the ring
  where it was moved instead of moving it back to the locale last
  requested. The Python and PowerShell managed locale rings do the
  same from `migrate_to` / `MigrateTo`.

- The PowerShell module is built with PoWerRuSt and cargo-pwrs 0.3.2,
  which compile its PowerShell 7 half against .NET 8 and PowerShell
  7.4's references whichever pwsh does the build, so it imports in
  PowerShell 7.4 and later where 0.5.1 needed 7.6, and building it from
  source needs Rust 1.98. In Windows PowerShell 5.1, where the first
  PWRS module a session imports serves every later one, import it
  before a module built by an older cargo-pwrs: after one built by 0.2.x
  through 0.3.1 it fails with `pwrs_module_init` status 4, and after one
  built by 0.1.8 or earlier with a type initializer error, as the
  troubleshoot page describes. All 227 Pester tests pass on Windows x64 in pwsh 7.6.6
  and Windows PowerShell 5.1, on Linux x64 in pwsh 7.6.5, 7.5.11 and
  7.4.20, on macOS arm64 in pwsh 7.6.5, and on FreeBSD x64 in pwsh
  7.5.5.

- `subetha-pointers`' capabilities carry the lifetime of the memory
  they were made from: `ReadableCapability<'a, T>` and
  `WritableCapability<'a, T>`. `WritableCapability::from_slice_mut`
  returns the capability alone, and `WritableCapability::narrow` takes
  `&mut self`, reborrowing it. `OwnedReadableCapability::cap` and
  `OwnedWritableCapability::cap` lend a `ReadableCapability` borrowing
  the owner, `OwnedWritableCapability::cap_mut` lends a
  `WritableCapability`, and neither owner dereferences to a
  capability. `CapabilityError` gains `Misaligned`.
  `UmbraPointer::with_content_prefix` requires `T: Marshal` and takes
  its prefix from the value's `Marshal` encoding, which is
  little-endian on every host.

- `subetha-sidecar` holds no lock. A node's registrations sit in slots
  claimed and freed with compare-and-swaps, a node's scan thread is the
  only consumer of its instances' observation rings, and `scan_now` asks
  that thread for a scan and waits for it rather than draining a ring
  itself. `unregister` waits only for a scan or stats read inside that
  instance's own registration, and `stats` reads a whole snapshot the
  scan publishes. Called from a node's scan thread, `scan_now` does not
  wait on that node; after the sidecar stops its threads it returns at
  once. `VersionedChain` swaps its head atomically, and the Python
  `Condvar.wait_for` keeps its predicate's first error without a lock.
  Neither crate depends on `parking_lot` any more.

- `subetha-cxc`'s async paths hold no lock. The `Waker` a task
  registers on a `Channel`, an `AdaptiveIpc`, a `WakerRing` or a
  reactive ring sits in a slot of its own ordered by one atomic word,
  where a wake that lands mid-registration is handed to the registrant,
  and `AsyncSpscRing`'s worker
  thread hands its result to the future through one atomic pointer.
  `TaskPool` and `RingExecutor` keep each task's run state in one atomic
  word: a wake that lands while a worker polls the task is queued by
  that worker when the poll returns, so no task is polled by two workers
  at once and no worker waits on another's poll. `TaskPool`'s ready
  queue is an unbounded lock-free queue of subetha-cxc's own, with idle
  workers parked beside it, and `AdaptiveIpc` keeps the surplus of a KHL
  slot steal in one.

- The pass registry keeps its handlers in a copy-on-write map: a
  dispatch takes the handler it finds and runs it holding nothing, and
  a registration swaps in a new map. `PassHandler` is an
  `Arc<dyn Fn(&[u8]) -> PassResult + Send + Sync>` where it was a
  `Box`, so `register` and `unregister` return an `Arc`.

- `SharedDequeKhpd` stages through a `KhpdStager` from `stager()`, a
  buffer its holder mutates through `&mut self`: `stage` and `publish`
  are the stager's, and the deque's `snapshot_size` returns
  `(head, tail, ring_size)` without a pending count. `SharedDequeLoh`
  stages through a `LohStager` the same way, with `push`, `flush` and
  `pop_local`, and its `snapshot_size` drops the LIFO length. Every
  publish on either deque reserves with a compare-and-swap on `tail`.

- `BlockingSpscRing`'s phase estimator, `SharedUniversal`'s backing
  handle and the wait calibrator's records hold no lock. The consumer
  takes the estimator for the length of a wait-path update, a
  `SharedUniversal` handle swaps a re-opened backing in whole, and the
  calibration thread replaces a record whole.

- `subetha-cxc`'s transports hold no lock. `DemuxQueue` is a struct
  over the same lock-free queue, where it was an
  `Arc<Mutex<VecDeque<...>>>`: clones share one queue, and its `push`,
  `pop`, `len` and `is_empty` take the place of locking, so a demux
  reader pushes and a receiver pops without either waiting. A listening
  Sens-O-Matic receiver's TLS
  handshakes run in a table its demux thread owns, which publishes the
  completed peers' keys for the poll side. Virtual endpoint bindings,
  the cross-process notifier's registries and the unified endpoint's
  peer and code-switch fields are swapped whole. The io_uring datagram
  backend receives and sends through separate rings, each held by one
  caller at a time: a receive that finds its ring held reads
  `WouldBlock`, and a send that finds its ring held goes through the
  socket directly. The NIC-bypass backend holds its wire the same way,
  and a caller that finds it held reads `WouldBlock`, which it treats as
  it treats a full socket buffer.

- `subetha-cxc`'s capacity rings and `AdaptiveRing`'s growth hold no
  lock. A capacity morph publishes its new state with one
  compare-and-swap against the state it read, and a morph that loses the
  swap decides again from the winner's state, so concurrent morphs each
  land in turn. `PubSubRing` takes any number of producers: a publish
  claims its position with one atomic add, and waits only when it has
  come a full lap round to a slot whose earlier item has not landed.
  `CapacityPubSubRing` links its backings and a subscriber holds the
  backing it reads; `CapacityPubSubSubscriber::backing_idx` is that
  backing's generation, 0 for the first and one more per morph, which
  `subetha_capacity_subscriber_position` reports too. `AdaptiveRing`'s
  shared-memory growth lays out a per-producer pair or attaches one
  already laid out, so threads and processes growing at once share one
  layout, and `create_shmfs` removes the pairs an earlier ring grew
  under the same name.

- No crate but `subetha-ffi` holds a lock. A `clippy.toml` at the
  repository root disallows the `std::sync`, `parking_lot` and
  `lock_api` lock types, and `subetha-ffi` allows them crate-wide, its
  handle table and per-handle guards serializing the calls C makes on
  one handle from several threads. The benches that measure against a
  lock-based baseline, and the C ABI's integration tests, which take
  turns at the library's process-wide init, allow them per file.
  `subetha-cxc` no longer depends on `parking_lot`.

- No crate depends on `arc-swap`. Every value a SubEtha crate swaps
  whole sits in a `SwapCell`, and `subetha-ffi` depends on
  `subetha-core`.

- The 64 per-primitive pages under `crates/subetha-cxc/docs/pointers/`,
  which the published `subetha-cxc` package carried, are gone. Each
  primitive's page in the
  [subetha-cxc reference](https://variably-constant.github.io/SubEtha/docs/reference/subetha-cxc/)
  carries what they held that it did not, and every link that named a
  pointer page names that page.

- `InstanceStats::last_seen_us_ago` is `last_drain_us`. It holds the
  microseconds from an instance's registration to the last scan that
  drained it, which the old name read as a time since.

- A process that used the global sidecar no longer writes
  "[subetha-sidecar atexit] shutdown complete" to stderr as it exits. The
  exit hook still stops and joins the scan threads, and writes only when
  one of them ended by panic.

- `BloomFine::SUGGESTED_CAPACITY` is 32, where it was 64: at 64 keys a
  `BloomFine` passes about 31% of absent keys by the standard estimate,
  and at 32 about 2.5%. Python's `FineBloom.suggested_capacity` and the
  PowerShell filter's `SuggestedCapacity()` report the new value.

- The `Blocking*` rings, `BlockingSemaphore` and `BlockingRWLock` spin
  the rounds of the process's wait plan before they park, where they
  spun 32. `CrossProcessWaker::wait` runs the plan's monitor phase, and
  `CrossProcessWaker::wait_with_plan` takes a plan of the caller's.

- Every dependency requires its newest stable release. Three of them
  reach what a consumer builds: `subetha-cxc` takes `windows-sys` 0.61
  on Windows, where it took 0.59; `rcgen` 0.14 behind the `quic-bridge`
  and `tls` features, where it took 0.13; and `xsk-rs` 0.11 behind
  `wire-locale`, where it took 0.8. The rest raise a floor within the
  version already required, or move a benchmark or test dependency:
  `criterion` 0.8, `cbindgen` 0.29, `ipc-channel` 0.23, `rtrb` 0.4 and
  `iceoryx2` 0.10. No release SubEtha requires needs a Rust newer than
  1.89, and the workspace's minimum stays 1.96.

- `AdaptiveRing::open_shmfs`, `open_shmfs_in` and `open_shmfs_secured`
  attach and make nothing: a ring that is not there is
  `RingError::IoError(NotFound)`, where the attach made empty regions of
  its own and reported `LayoutMismatch`. Through the C ABI,
  `subetha_ring_open_shm`, `subetha_vyukov_open_shm`,
  `subetha_pubsub_open_shm` and `subetha_broadcast_open_shm` return
  `SUBETHA_E_RING_IO` for a ring that is not there, where they returned
  `SUBETHA_E_RING_LAYOUT_MISMATCH`. `create_shmfs` and its peers report a
  region they could not make as `IoError` with the OS error's kind, where
  they reported `PayloadTooLarge`.

- A shm-backed `AdaptiveRing`'s names outlive every handle, as a
  file-backed ring's files do, whichever handle made them: the creator's
  backings, a per-producer backing a peer grew, the payload region of the
  first oversized frame and the ordering region, and through the C ABI the
  ring's wakers and notifier record. `unlink_shmfs`,
  `subetha_ring_unlink_shm` or the last holder removes them, where on Unix
  each went when a handle dropped. A `NotifierSet::shm` record outlives
  its sets the same way. A `LocaleAdaptiveRing` removes its shared-memory
  backing's names when it drops, as it removes its files, and a capacity
  ring's shared-memory generations go when it drops them. On Windows a
  section goes with its last handle either way.

- `ShmNamespace` gains the `AppContainer` variant, so a `match` that names
  every variant needs an arm for it, and `subetha_ring_options` gains
  `shm_container_sid` at its end, so a C consumer rebuilds against the new
  header. `LastHolderError::ShmNotSupported` is gone, since a shm-backed
  ring now takes holds, so a `match` that names it loses that arm.

- The wiki's performance figures for the failover watchdog, the pass
  registry, the KHPD and LOH deques, the adaptive ring's dispatch, the
  async conventions and fan-out, the Umbra, Bloom, Versioned, KStep,
  CHERI capability and RASP pointers, and the ring throughput matrix
  (a full sweep, every family) are measured again, on an AMD Ryzen 9
  7900X under Windows 11, with each page naming its host and the load
  during the run. Several conclusions change with them, and each page
  says what was measured: the watchdog's single-threaded scan is slower
  than a `Vec<Mutex<u64>>` baseline on this host, a `VersionedPointer`
  visibility check costs more than a raw `u64` compare, and the RASP
  validators beat a native checked read.

- `Send-SubEthaItem` works out once, when the pipeline begins, whether
  it is sending to a `BroadcastRing`, rather than asking every item's
  target for its type name.

- The workspace crates, the Python package and the PowerShell module
  name Mark Newton as author, and the module's copyright reads the same.

### Fixed

- A Windows process in which `subetha-sidecar` ran from a DLL, as it
  does under the Python package and the PowerShell module, ended with
  0xC0000409 on its way out: the sidecar's exit handler runs at the
  DLL's detach, after the process exit has already ended the scan
  threads, and joining an ended thread panicked. The handler returns
  at once when the loader reports the process is shutting down
  (`RtlDllShutdownInProgress`).

- A consumer that registered on a `CapacityBroadcastRing` while a
  morph was building its backing had no slot on that backing: the
  morph mirrored the consumers it had counted before, so nothing pushed
  after the morph reached the late consumer, and the next consumer to
  register was handed the same index. The wrapper now hands out
  consumer indices itself, and a backing a morph makes active claims
  every index handed out so far before anything is pushed into it,
  read from it or handed out through `pin_current_capacity` or
  `ring_handle`. A consumer that registers after a morph reads past
  the stale backings it has no slot on.

- A producer that registered on a `CapacityAdaptiveRing` while a morph
  was building its backing was not registered on that backing: the
  morph mirrored the counts it had read before, so the new backing
  could be a single-producer ring with two producers sending into it.
  A morph now carries each registered producer and consumer id onto
  its backing under the same id, and a backing a morph makes active
  carries every registration its predecessor took since, before
  anything registers on it, sends into it, receives from it or pins
  it; a registration that lands on a backing already carried from is
  made again on the backing in place.

- Two processes creating a `SharedHolderTable` on an empty or missing
  file together could both lay it out, each deciding from the length it
  read, so a claim one made was cleared by the other's layout and both
  held the slot. The creators now elect one layout with a
  compare-and-swap on the table's magic and the rest wait for it; one
  that has not finished within five seconds, such as one a process died
  during, is refused as `IoError(TimedOut)`, and `reset` recovers the
  file. A create asking for more slots than the file's table holds is
  refused as `LayoutMismatch` and leaves the file as it is, where it
  laid a new, empty table over the old one.

- `Bloom64` and `BloomFine` passed absent keys at up to 1.65 times
  their estimated false-positive rate. Every bit index was sliced from
  a single multiply, whose low bits depend only on the key's low bits,
  so sequential keys collided: `Bloom64` holding 16 keys passed 26.4%
  of 10,000 absent keys against an estimate of 16.0%, and `BloomFine`
  holding 64 passed 42.3% against 31.2%. The hash now goes through
  MurmurHash3's 64-bit finalizer before it is sliced, and the same
  filters pass 16.9% and 26.3%. A filter's bits differ from those an
  earlier version built for the same keys, so rebuild from its keys a
  filter carried as bits, such as one passed to Python's
  `TinyBloom.from_bits` or PowerShell's `New-SubEthaTinyBloom -Bits`.

- `AdaptiveIpc` could drop a batch shape from the filter `maybe_promote`
  consults. A batched send read the filter, added its shape and wrote
  the filter back, so a shape another sender recorded between the read
  and the write was overwritten. A send now ORs its shape's bits into
  the filter in one atomic operation.

- Two publishes on a `SharedDequeLoh` racing for its last free slots
  could both pass the capacity check and both reserve. The one past
  the capacity then waited for a slot no thief had freed, forever when
  none came, where it should have returned `Full`. A reservation is a
  compare-and-swap that checks the capacity against the tail it
  replaces.

- `SharedUniversal::migrate_to` lost an insert another thread of the
  same process made after the migration's snapshot: the new backing
  held the snapshot, and the insert stayed in the old one. The
  migration swaps the new backing in, waits for the ops still holding
  the old one to return, and copies what they inserted after the
  snapshot. An insert into a Vec backing reports the backing's
  failures other than a full vec as `LayoutMismatch`, `IoError` or
  `VecError`, where it reported every one as `Full`.

- Two `PubSubRing` publishes at once could take the same position: one
  item overwrote the other and the head counted one publish. Each
  publish now claims a position of its own.

- `CapacityPubSubRing::gc` reclaimed a backing a subscriber had not
  finished reading, and a reclaim shifted every subscriber's place in the
  chain, so a subscriber lost the rest of its backing or everything
  published after the reclaim. A subscriber holds its backing, and gc
  passes only backings nothing but the chain holds.

- `subetha-pointers`' capabilities could outlive their memory from
  safe code. `ReadableCapability::from_slice` and
  `WritableCapability::from_slice_mut` returned a capability holding a
  raw pointer and no lifetime, so a capability kept past its slice
  read or wrote freed memory; `from_slice_mut` also handed back the
  `&mut` slice beside the capability, and `WritableCapability::narrow`
  made a second writable capability over the same region. A
  capability now borrows its memory for its whole life, a writable
  one has one user at a time, and a `new` or `narrow` at an address
  misaligned for `T` returns `Misaligned` rather than a capability
  whose read is undefined behavior. `UmbraPointer::with_content_prefix`
  read the first four bytes of any `T` as initialized, padding
  included. `RaspBatch`'s AVX2 and AVX-512 checks compared addresses
  as signed numbers, so an entry whose base and pointer straddle 2^63,
  which `push_raw` accepts, was valid to the scalar check and invalid
  to the SIMD ones; they compare unsigned. Without the change, on the
  Windows host (Ryzen 9 7900X), a `u64` capability was made and
  narrowed one byte past a `u64`, the dispatched `RaspBatch` count was
  0 where the scalar count was 16, and all four compile-fail examples
  compiled.

- A primitive's observation ring takes pushes from any number of
  threads at once. Each instance owns one ring and every thread that
  operates on the instance pushes into it, but a push claimed its slot
  by reading the ring's tail and storing it back, so two threads
  pushing together wrote one slot, both reported success, and a record
  could be read while another thread was still writing it. With eight
  threads filling an armed ring together on the Windows host (Ryzen 9
  7900X), 694 of 4,096 accepted observations came out, and a sidecar's
  counts of ops and producer threads fell short by as much. A push now
  claims its slot with a compare-and-swap and publishes the record
  through the slot's sequence number, so every accepted observation is
  popped once and whole, and an armed ring's buffer stays 96 KiB.

- `AdaptiveRing::unlink` removes a file-backed ring's holders region last,
  after the peer directory, and a last holder removes it after whatever
  the layer above keeps beside the ring, as the shared-memory path already
  did. The holders region is what a process arriving mid-teardown reads to
  decide whether the ring is still held; `unlink` removed it fifth, before
  the per-producer backings and the peer directory, and a last holder
  removed the layer above's files after it.

- On Windows a shared-memory `AdaptiveRing` keeps the regions one handle
  makes after that handle leaves, and says so when it cannot. A section
  lasts only while some handle holds it, and a per-producer pair a peer
  grew, or the payload region its first oversized frame made, was held
  only by the peer until another handle's next call opened it. On the
  Windows host (Ryzen 9 7900X), with the peer gone before then, an attach
  failed with `IoError(NotFound)`, and the owner's `recv_frame` of the
  peer's frame made an empty region of its own and returned 3,000 zero
  bytes as the payload, with no error. Now:
  - every `AdaptiveRingSidecar` scan opens what other handles have
    published, as the handle's own next call does, so a managed handle
    holds them between calls, and a `LocaleAdaptiveRingSidecar` does the
    same for its shared-memory backing;
  - the handle that makes the payload region moves the topology epoch,
    and every other handle opens the region at its next sync;
  - a receiver opens the payload region and never makes one, so a frame
    whose region is gone is `IoError(NotFound)`;
  - an offset frame names the laying-out of the payload region its
    payload went into, from a value the region's header now carries in
    padding it already had, so a frame sent into a region that has since
    been replaced under the same name is `IoError(NotFound)`, not a read
    of the new region's block; a zero from a region or a sender that
    records none is accepted as before;
  - a published pair whose every handle closed is laid out again empty by
    the next handle to look for it, an attach included, so the producer
    slot it serves keeps working, and the loss is written to stderr;
  - a handle that cannot open a published pair writes that to stderr once
    for each change of topology, where it retried silently on every call.
  On Unix a pair's name outlives its handles, and a missing one was
  removed while the ring was in use, so it is not made again. Without
  the change, a sidecar opened neither a grown pair nor the payload
  region within 30 s, and a frame sent into a payload region since
  replaced under the same name was read as the 3,000 bytes a later
  sender had put in the new region: on the Windows host when the region
  went with its maker, and on the FreeBSD and Ubuntu guests when its name
  was removed while the ring was in use.

- On Unix a shared-memory ring's names no longer go with the first handle
  to drop. Every `ShmFile` dropped unlinked its name, so once one attached
  process let go, a process attaching afterward found nothing: on the
  FreeBSD and Ubuntu guests an `AdaptiveRing` attach after another
  attacher had left got `LayoutMismatch`, and a `ShmFile` open after an
  opener had dropped got `NotFound`. A `ShmFile` handle owns its name when
  it made the region, which it learns from an exclusive create, or when a
  creator made it with `create_named_secured`, and only the owner unlinks.
  A shm-backed `AdaptiveRing` keeps every name it makes, whichever handle
  makes it, so a peer that grew the ring or made its payload region takes
  nothing with it either: with each name its maker's, on both guests an
  attach after the peer that grew the ring had left got `NotFound`, and a
  later receiver of an offset frame read 3,000 zero bytes where the
  payload was, once the peer that made the payload region had left.
  Windows keeps a name until the last handle closes and needed no change.

- An `AdaptiveRing` handle on shared-memory backings no longer lays out
  afresh a region another handle built. When a producer registered past
  the ring's initial count, every other handle opened the new
  per-producer backing on its next operation by initializing it, which
  dropped whatever had already been sent into it. Turning ordering
  stamps on in an attached handle rebuilt the creator's ordering region
  the same way, resetting its mode, counters and watermarks and writing
  the attacher's stamp kind over the creator's. Both are now opened as
  they stand, and an attached handle asking for a different stamp kind
  is refused with `LayoutMismatch`, as a file-backed ring already was.

- Attaching to a file-backed structure that several callers reach
  together no longer fails with `Access is denied` on Windows. Every
  attacher created and removed the election marker on its way to a
  region that was already published, and NTFS refuses a create of a
  name whose removal is in flight, although the name is gone once the
  removal returns: eight threads creating, closing and removing one
  name 200 times each were refused 58 to 66 times, whether or not a
  handle stayed open across the removal. An attacher now looks at the
  region before it touches the marker, and a marker create refused
  that way is tried again after another look, for up to the five
  seconds an attacher gives a builder. Eight threads attaching to one
  region 200 times each were refused 32, 15 and 34 times in three runs
  before the change and none after, on Windows 11 / Ryzen 9 7900X.

- `SharedHashMap` and `RawHashMap` writers no longer race one another
  on a slot. An update, a `compare_exchange` or a `remove` matched its
  key under a read and then wrote, so a remove of that key, or another
  key's claim of the slot the remove left, could land in between: a
  value was handed back twice or never, and one key's bytes could sit
  under another key's hash, where neither was found again. Two inserts
  of one new key, racing a remove in the key's chain, could each place
  it. Every write to a published entry now takes the slot's lock and
  re-checks the state, hash and key under it, and an insert of a new
  key gives its claim back and walks again when a remove landed during
  its walk.

- A `LocaleAdaptiveRing` leaves no files behind when it drops. Its drop
  removed the file backing's rings but not its peer directory, nor the
  per-producer pairs registration grew past the construction hint, so a
  ring created later under the same base path attached to that
  directory with every claim the dropped ring held, and its first
  `register_producer` panicked: the file backing handed out the next
  free id while the new anonymous backing handed out 0. The drop now
  removes every file `AdaptiveRing::unlink` names for the file backing.

- `bench_throughput`, the example the ring throughput matrix is measured
  with, keeps each run's file-backed ring in a directory of the run's
  own under the temp directory and removes it when the run ends. It
  named those files by process id and left them in place, so a run
  whose process got a reused id attached to an earlier run's ring, and
  one of another capacity was refused as `LayoutMismatch`.

- A `LocaleAdaptiveRing` handle from `open`, as `subetha_locale_ring_open`
  and the Python and PowerShell bindings' open paths make, registers
  producers and consumers beside other handles. Each handle's anonymous
  backing numbered its registrations from 0 while the shared file and
  shared-memory backings handed out the next free id, so a handle's
  first registration after another handle's panicked. The id is now
  claimed on the file backing, whose peer directory every handle on the
  base path shares, and carried onto the other two backings under the
  same id.

- A Sens-O-Matic RLC receiver delivered a stream from the first source id
  it read. When a stream's first datagrams were lost before the receiver
  read them, it started above them, its first ACK released them at the
  sender, and they were never delivered. On FreeBSD one netisr thread
  queues all loopback traffic in 256 packets and drops the rest, and
  under the release gate's parallel load the test suite delivered
  streams from items 3, 23, 45, 53 and 95 and lost a peer's first item.
  A window now starts at item 0 once it holds it or a repair covers it.
  Otherwise it asks the sender with NAK rounds naming 0, the ids just
  below the lowest one seen, and that lowest id last. The sender resends
  that last id only after every lower id it still holds, so two rounds
  in which it came back with nothing below start the window there, which
  is how a restarted receiver joins a stream partway through. A window
  admitted by challenge finds its start the same way; one that a
  restarted receiver admitted started at 0 and waited for items its
  sender had already released. The wire format is unchanged. Six new
  tests cover a lost head with and without its repairs and over the
  unified endpoint, a restarted receiver with one peer and with an
  admitted second, and the probe's own NAK; all six fail without this
  change.

- A blocking ring could lose a wake. A producer stores its ring's head
  and then reads the waker's parked mask; a consumer sets its mask bit
  and then re-checks the ring. x86 and ARM64 both let each of those
  loads complete before the store ahead of it, so the producer could
  find no parked bit while the consumer found no item, and the consumer
  slept beside the item until its timeout. A wake scan now starts with a
  SeqCst fence and a park ends with one. With two processes echoing
  frames over a pair of file-backed `BlockingSpscRing`s, 20 runs of
  2,000 frames each with no hold on Windows 11 / Ryzen 9 7900X, 7 runs
  lost a wake without the fences and none with them. 0.5.1 lost a wake
  in 5 of 15 runs of the same round trip with holds of 0, 100
  microseconds and 1 ms. On the send path the fences cost nothing that
  host resolves: 10.114 ns against 10.118 ns per push-and-pop pair.

- The shared condvar page said a second `open` of the same file in one
  process misses wakes on Windows. No platform's park for a file-backed
  condvar is keyed by virtual address, so it works; it costs a second
  mapping.

- 0.5.1 said the PowerShell module works on FreeBSD x64. Its PowerShell
  7 half was built on PowerShell 7.6 and references .NET 10, so it does
  not import in FreeBSD's PowerShell 7.5.5, nor in PowerShell 7.4.20 or
  7.5.11 on Linux; the FreeBSD figures came from a module built on
  FreeBSD.

- The install, explanation and troubleshoot pages listed the win-x64
  and linux-x64 natives. The published folder has carried osx-arm64 and
  freebsd-x64 as well since 0.5.1.

- `QuicBridgeClient::run` wrote a line of UDP counts to stderr each
  time it finished, so every caller's stderr carried it, the C, Python
  and PowerShell bridges included. It now writes nothing when it
  succeeds.

- `QuicBridgeClient::run` could fail with a lost connection after the
  server had received every item. The server closed the connection as
  soon as it had read the items its header declared, and that close
  could reach the client ahead of the acknowledgment of the client's
  last data. The server now reads the stream to its end, refuses one
  that carries more than its header declared, and waits for the client
  to close; the client closes once its data is acknowledged. Over 200
  rounds of the PowerShell module's round trip on Linux x86-64, 4
  failed that way in PowerShell 7.4.20 and 3 in 7.6.5 before, and none
  in either after. `run` now returns after QUIC's closing period, three
  probe timeouts, which is what delivers the close when the caller's
  runtime ends with the call, as the C API's does. The 200 rounds took
  26 s in place of 4 s in 7.4.20 and 52 s in place of 7 s in 7.6.5.

- The PowerShell pages said every call on a module object is serialized
  and left out what that costs: a call that waits holds its object until
  it returns, so a second call on the same object from another thread
  waits behind it. A server's `LocalAddr()` asked for after its
  `AcceptOne` has started in another runspace waits for a client that
  cannot be made without that port. The module's QUIC round-trip test
  did exactly that, and the macOS arm64 run of the release candidate
  printed nothing for 45 minutes after its suite started. On Linux
  x86-64, with `AcceptOne` given 1.5 s to start first, `LocalAddr()` was
  still waiting 10 s later and returned 1 ms after a client made by
  other means ended `AcceptOne`. The pages now say so, and the test
  reads the port before `AcceptOne` starts.

- The C API's QUIC bridge server could not be made:
  `subetha_quic_bridge_server` bound its endpoint with no runtime
  entered, which quinn refuses, so every call answered
  `SUBETHA_E_RING_IO`. The TCP bridge server could be made but never
  received: it bound its listener on a runtime that ended with the
  constructor and accepted on another. Each server now keeps the runtime
  it bound in and runs every accept on it. The C suite carries 40 items
  end to end through each bridge; on FreeBSD x64 with the transports
  built, it failed 176 checks before and passes after.

- With `wire-locale` on, a shared library linking subetha-cxc failed to
  link on Linux: libxdp-sys's make build compiles libxdp's static
  objects without `-fPIC`, and the link stopped at a `R_X86_64_PC32`
  relocation against `stderr`. subetha-cxc now selects libxdp-sys's cc
  build, which compiles them position-independent. On Linux x86-64,
  `cargo build -p subetha-ffi --features subetha-cxc/wire-locale`
  failed before and links after, and subetha-cxc's 1,527 tests pass
  with every feature on, `wire-locale` among them.

- `Sidecar::scan_now` could drain an observation ring while the node's
  own scan thread drained it too, although a ring has one consumer, so
  observations were counted twice or replayed and a slot could be read
  while its producer rewrote it. Each node's scans now take turns. With
  four threads calling `scan_now` beside the node's thread while one
  producer pushed 200,000 observations, the sidecar counted 535,879 to
  627,373 of them before and 200,000 after, on Linux x86-64.

## [0.5.1] - 2026-09-21

### Added

- The Python wheel and the PowerShell module both work on macOS arm64,
  and the module works on FreeBSD x64 as well. Every platform was
  measured rather than assumed: 502 Python tests and 181 Pester tests on
  macOS, and 181 Pester tests on FreeBSD under PowerShell 7.5.5 on
  .NET 9. The module's managed half turns out to be reproducible, since
  building one commit on Windows, Linux and macOS produces a
  byte-identical manifest and shell assembly, so only the native library
  is genuinely per-platform.

  FreeBSD needs two things arranged that SubEtha does not control.
  Building wants `PWRS_TOOLSET=5.3.0`, because the C# toolset the build
  tool fetches by default requires .NET 10 and FreeBSD packages nothing
  past 9. Running the suite wants `$IsLinux` set, because Pester decides
  the platform from three booleans that are all false there and throws
  rather than guessing. Neither is needed to use the module: it imports
  on FreeBSD unaided and all 135 cmdlets work.

### Fixed

- The wheel workflow installs the distribution by its own name. Both of
  its check steps asked pip for `subetha`, which is what the package
  imports as; the distribution is `subetha-ipc`, so pip matched no wheel
  in `dist` and answered "from versions: none" on every platform. The
  workflow had never been run, so a name wrong in both places had never
  failed anything. Running it produced the first Linux wheels SubEtha
  has published.

- The README links the published Guide. Its Documentation section called
  the Guide the canonical reference and then pointed at `wiki/`, the Hugo
  source, so a reader following it got markdown rather than the Guide.
  The site was named nowhere in that file although every crate README
  carries a badge to it. There is now a badge, a line naming it, and
  links to the four sections. GitHub Pages does not appear under a Wiki
  tab, so a README link and the repository's Website field are the only
  ways anyone finds it.

- Three pages stated test counts measured before 0.5.0: the Python
  install page said 494 and 7 skipped for a default build and 500 and 1
  with both bridges, where the measured figures are 502 and 7, and 508
  and 1. Both PowerShell pages said 178 Pester tests, where the measured
  figure is 181 on each of PowerShell 7.6.6, Windows PowerShell 5.1 and
  PowerShell 7.6.5 on Linux. The README badge said 1331, which matched
  no suite, and is now one badge per surface.

- Both PowerShell pages said that identical manifests follow from one
  checkout, which is what `cargo pwrs merge` requires. One checkout is
  not sufficient: the manifest gained fields between releases of the
  tool, so two hosts on different versions of it produce different
  manifests from the same source and the merge is refused. They now say
  the tool version has to match as well, and that `cargo install --list`
  is what settles it, because the newest binary on disk can be the
  oldest version.

## [0.5.0] - 2026-09-20

Three surfaces answer `lag` differently for a consumer that is not
there, which is why this is a minor rather than a patch. Python's
`BroadcastRing.lag` returns `int | None` where it returned `int`,
PowerShell's `Lag` returns `$null` where it returned a number, and
`subetha_broadcast_lag` returns
`SUBETHA_E_BROADCAST_INVALID_CONSUMER` where it returned `SUBETHA_OK`
and wrote a number into `out`. A caller that only ever passes a
registered consumer id sees no change. The `### Changed` entry below
says what the old answers were and why they were wrong.

### Added

- A guide layer for both bindings, under
  `wiki/content/docs/how-to/python/` and
  `wiki/content/docs/how-to/powershell/`: installing, choosing a
  structure, making it fast, what a thread or a runspace does to a
  handle, bridging two hosts, and troubleshooting. An explanation page
  beside each says what the binding is and what crossing it costs. The
  reference says what a call takes; these say which call to reach for,
  and what a symptom means when it was the wrong one.

- A generated reference for both bindings.
  `crates/subetha-pwrs/tools/Export-Reference.ps1` reads the built
  module's own help and type metadata;
  `crates/subetha-py/tools/export_reference.py` pairs the type stub's
  signatures with the doc comments in `crates/subetha-py/src/lib.rs`,
  which is where PyO3 takes `__doc__` from. Between them they cover the
  135 cmdlets, 118 PowerShell classes, 92 Python classes, the enums and
  the module-level values. Neither page is written by hand, so neither
  can describe a surface the build does not have.

- A description on every method of the Python surface. 273 of the 686
  methods carried no doc comment anywhere, so `help()` had nothing to
  say about them. Each was written from the method body and the
  `subetha-cxc` call under it rather than from the name, which is what
  the two disagreeing looks like: `RateLimiter.reset` refills the bucket
  rather than emptying it, `Slab.__len__` and `BitVec.__len__` answer
  the capacity and never change, and `__exit__` on a structure closes
  nothing, flushes nothing and releases no registration.
  `crates/subetha-py/tools/check_docs.py` reads `__doc__` off the
  compiled module for every method the stub declares and exits non-zero
  naming any that is bare; the wheel workflow runs it, so the claim is
  held against the built artifact rather than against the source that
  describes it.

- A values page for each binding, generated by running real scenarios
  against the built module and capturing what they answered
  (`crates/subetha-pwrs/tools/Export-Examples.ps1`,
  `crates/subetha-py/tools/export_values.py`). A type name says what
  comes back and not what it looks like: a ring hands back a whole slot,
  payload then zeros, rather than only what was pushed.

- `SharedBroadcastRing::wait_for_consumers(want, timeout)`, and with it
  `wait_for_consumers` on the Python `BroadcastRing`, `WaitForConsumers`
  on the PowerShell one, and `subetha_broadcast_wait_for_consumers` on
  the C ABI. A consumer registers at the head, so everything published
  before it registered is lost to it and nothing reports that: the push
  succeeds, the ring does not error, and a reader that started late is
  indistinguishable from one that is slow. Publishing only once the
  readers are here is the only thing that closes the window, because
  afterward there is nothing left to detect. All four answer how many
  consumers are present when the wait ends rather than failing on a
  shortfall, and all four refuse a wait without end.

- `subetha.aio.wait`, which awaits a `Notifier` on an asyncio loop. On
  Unix the loop watches the notifier's descriptor and the wait occupies
  nothing; on Windows the blocking wait runs on a thread from the loop's
  executor, because asyncio has no public way to watch an event handle
  and the default proactor loop does not implement `add_reader` at all.
  What a caller awaits is the same either way. Like `Notifier.wait` it
  does not consume the signal, so a caller that does not drain gets an
  immediate second wake on the signal it already saw.

- `crates/subetha-spec-c`, a second reader of the Sens-O-Matic wire
  specification. It implements section 2's field, the RLC code of
  section 4 and the Reed-Solomon code of section 5 in C, written from
  the document and checked against the frozen vector files. It depends
  on nothing of SubEtha's deliberately: a program built on
  `subetha-ffi` inherits every assumption of the first implementation
  and so cannot disagree with it about anything, and an implementation
  that cannot disagree says nothing about the document. This one
  disagreed with the vectors on the sparsest of the six geometries,
  which is where the specification changes below come from.

- A bench, `crates/subetha-cxc/benches/park_vs_retry.rs`, for where
  parking starts beating spinning on a full ring. `send_blocking` spins
  and then parks; a caller can write a `try_push` retry loop instead,
  and which wins depends on how long the producer waits, which the
  consumer's drain rate sets. Wall time alone flatters the retry arm, so
  it also reports the iterations it burned, and both arms run again with
  every core saturated: that second pass is the one a staged pipeline
  lives in, where the cores a spinning producer occupies are the cores
  its own consumer needs.

- `crates/subetha-cxc/tools/dropped_errors.py`, which separates
  production sites from test ones across the three shapes a dropped
  `Result` takes here: `.ok();` discards it, `unwrap_or_default()`
  substitutes for it, and `if let Ok(` walks past it. A grep for the
  first alone returns 110 hits in `subetha-cxc`, almost all of them a
  test removing a file or joining a thread.

### Changed

- `SENS_O_MATIC_WIRE.md` states the recovery condition for the RLC code
  and says what a conforming decoder must do. Section 4 defined the
  encoder completely and never described a decoder, so the recovery
  condition had to be derived, and the obvious derivation, that exactly
  one symbol of the window is missing, is correct at density 15 and
  wrong at every density below it: a coefficient of zero means the
  symbol does not enter the equation, which the coefficient rule says a
  long way from anything about recovering. A receiver must now derive
  the coefficients as written, refuse a generator it cannot reproduce,
  and recover a symbol when a repair determines it. It may solve the
  linear system that repairs with overlapping windows form, which
  recovers symbols no single repair determines; that is neither required
  nor forbidden, so recovery power is a quality of a receiver rather
  than of the wire.

- The residue code, behind the `residue-fec` feature, no longer divides
  or allocates per column. The moduli are a published constant table, so
  every reciprocal is known at compile time and Barrett reduction with
  `floor(2^34 / m)` replaces the 64-bit hardware divide in Garner's
  inner loop; the per-column `Vec` for the residue list and the second
  one for Garner's digits are gone; and `MODULI[i] mod MODULI[j]`, the
  Horner factor that was constant all along, is a table built once
  rather than a reduction per digit per column. On one host at k=8 with
  a 1 KiB packet, encode fell from 105.02 to 73.59 microseconds and
  recovery from 112.85 to 81.86, against 3 to 4 percent drift on the
  unchanged RLC arm. The allocations rather than the divides were what
  the time was going to: taking out the divide alone bought 9.7 percent.

- `lag` refuses to answer about a consumer that is not there. An index
  past the consumer table and a slot nothing holds both count, because
  `unregister_consumer` clears the active flag and leaves the cursor
  where its last holder stopped, so the distance from the producer to
  that cursor is a reading about nobody and it grows with every push.
  `SharedBroadcastRing::try_lag` answers `None` and `lag` answers
  `u64::MAX`, where `lag` used to answer `0` for an index past the
  table. The Python and PowerShell bindings return `None` where they
  returned an integer. `subetha_broadcast_lag` returns
  `SUBETHA_E_BROADCAST_INVALID_CONSUMER` where it used to write a number
  into `out` and return `SUBETHA_OK`.

## [0.4.1]

### Added

- A PowerShell binding, `crates/subetha-pwrs`, built as the module
  `SubEtha` for PowerShell 7 and Windows PowerShell 5.1. It covers the
  same families as the Python binding: `New-` and `Open-` cmdlets
  obtain a structure, the object each writes carries the operations as
  methods, and `Send-SubEthaItem` and `Receive-SubEthaItem` move items
  through the pipeline. Every cmdlet also answers to a short name with
  the `SE` prefix. Bytes cross as `byte[]` pinned in place, guards come
  back as objects that release on `Release()`, `Dispose()` or
  collection, and the TCP and QUIC bridges are always built in. It is
  built with `cargo pwrs` from the crate. Every cmdlet, class, enum and
  method it exports is reached by a Pester suite, and a gate suite fails
  the run when one is not. `Get-Help` carries a synopsis, a
  description, help on every parameter and one example for each of the
  135 cmdlets, and the reference page names them all.
  All 178 tests pass in pwsh 7.6.6 and Windows PowerShell 5.1 on
  Windows x64 and in pwsh 7.6.5 on Ubuntu 24.04 on Linux x64; the
  native libraries of the two builds fold into one module folder that
  imports on both.

- A gate over the C ABI, `crates/subetha-ffi-tests/tests/surface_gate.rs`.
  It reads the exported functions out of the library's source and fails
  when one is called by no test, no bench and no C program, the way the
  Python and PowerShell bindings are already gated. All 839 exported
  functions are reached; 182 of them were reached by nothing before.

### Fixed

- A region whose builder died while making it can be built again. The
  builder is elected on a marker beside the region rather than by
  creating the region's own name, and the region is built under a
  staging name and published by linking it into place, so that name only
  ever appears over complete bytes. Before this, a process that died
  between winning the election and sizing the file left an empty region
  that every later attacher waited five seconds on and none could
  replace, permanently.

- `SharedHashMap` reports the raw operating-system error before
  `MapError` discards it, so a failure to attach says which system call
  refused and why.

- Three `SharedRateLimiter` tests no longer fail under load. They
  measured their refill window from after the bucket was built, while
  the bucket credits refill from the build itself: an acquire a full
  bucket satisfies spends tokens without restamping the clock, by
  design, so an under-limit caller pays no clock read.

## [0.4.0] - 2026-09-13

### Added

- The Python binding now covers the whole surface a Python caller can
  use. The front door it was missing: `Channel`, a queue between
  processes that can be waited on, `WorkQueue`, work one process owns
  and others steal from, `KvMap`, and `AdaptiveQueue`, which picks its
  shape from the traffic it actually sees rather than from what was
  declared and moves between a ring and a work-stealing deque while
  running.
- Bounded waits throughout. `RWLock.read_for` and `write_for`,
  `Semaphore.acquire_for`, and the channel's `recv_for` and `send_for`
  sleep rather than spin and give up at their deadline. Only the
  bounded forms are exposed: an unbounded park sleeps until something
  signals it, which against a peer that releases without signaling
  would never wake.
- The sensing plane: `LossKind`, `LossBursts`, `Timing`,
  `RoundTripShape`, `Periodicity`, `Capacity`, `Forecast` and
  `PathChanges`. Each is fed measurements and answers what it worked
  out, holds no shared memory and touches no network, and answers
  `None` rather than a number until it has seen enough.
- The value types that ride beside a pointer: `TinyBloom`, a whole
  bloom filter in one machine word whose state crosses as a single
  number, `FineBloom`, `Clock`, which orders two events sharing a
  wall-clock reading, and `CausalClock`, which answers before, after,
  equal, or concurrent.
- `Atomic` gains subtract, the bitwise operations, swap, and compare
  and exchange. It had only add, so a counter could only go up.
- `subetha.aio`, a small pure-Python module letting a coroutine wait
  without blocking its loop. The Rust async engine is not bound and
  cannot be: every entry point takes or returns a Rust future.
- `tests/test_threading.py` runs every call that releases the
  interpreter under eight threads, which is what backs the module's
  declaration that it does not need the interpreter lock.

### Fixed

- `blocking_rw_lock`: `signal_unlock` is public. A caller that hands a
  hold to another language releases it at a moment of its own choosing
  rather than by dropping a guard, and nothing was then telling parked
  waiters the lock had changed hands, so a bounded wait slept to its
  deadline with the lock already free.

## [0.3.3] - 2026-09-13

### Fixed

- The published Python wheel carried instructions the machine that
  built it has and most do not. The build host's own cargo config sets
  `target-cpu=native`, which on that machine means AVX-512, and an
  environment variable replaces such a setting rather than adding to
  it, so nothing on the command line was overriding it. The wheel died
  with an illegal instruction on import on any other x86-64 machine,
  before a single call. Published artifacts now pin
  `-C target-cpu=x86-64`, which costs nothing because the crate
  dispatches its wide kernels at run time, and the wheels workflow sets
  the same so a runner can never inherit a host's choice. The 0.3.2
  wheel on PyPI should be yanked; its source distribution is unaffected
  because it is compiled on the machine that installs it.

## [0.3.2] - 2026-09-12

### Added

- `subetha-py`, the Python binding, bound to the Rust directly rather
  than through the C ABI, covering every family the substrate has: the
  rings including the two that change themselves under traffic, ordered
  delivery through a ring's own stamps, shared state and state with a
  history read through a pin, the locks and the semaphore and the
  condition variable and the owner lease, the probabilistic and
  specialist structures, the quality of service policy, and the
  Sens-O-Matic link that reaches another machine. Measured on the
  AVX-512 Windows host, the same atomic load costs 7.1 ns reached from
  Rust through the C ABI, 30.1 ns through this binding, and 584.4 ns
  through a C shim driven by ctypes, which is what binding the Rust
  directly is worth; batched a thousand at a time it is 1.3 ns an
  operation, and a `Region`'s buffer read whole is 0.3 ns a byte.
  `bench/call_shapes.py` reproduces all of it. It ships as a wheel
  rather than to crates.io.
- Bridges carrying a whole ring to another host over TCP or QUIC, each
  behind a cargo feature and off by default because each brings a
  network stack a process sharing memory on one host does not need. A
  wheel exports `transports`, naming what it was built with, so a name a
  caller cannot find can be told from a feature left out.
- Wheels for free-threaded interpreters. The stable ABI is now a cargo
  feature rather than fixed, since free-threading has none to target
  until 3.15; leaving it out builds for the one interpreter. The module
  declares it does not need the interpreter lock, which is a claim about
  every call in it and is made because `tests/test_threading.py` passes
  on an interpreter that has none: eight threads through the counters,
  both kinds of lock hold, the semaphore's permits, a shared ring, a
  pinned scan running beside writers, and a buffer view held across
  other threads' work. Verified against CPython 3.14.7 free-threaded,
  384 tests with the lock reported off.
- `.github/workflows/python-wheels.yml` builds both kinds of wheel per
  operating system and runs the suite against each one after installing
  it. The free-threaded job additionally asserts that importing the
  module leaves the lock off. Nothing publishes.
- Every SubEtha value held in a `#[pyclass]` is boxed, and a
  compile-time assertion per class enforces it. Python's object
  allocator aligns to sixteen bytes, `HandshakeHeader` is cache-line
  aligned, and 44 of the primitives embed one, so a class holding any of
  them inline compiles and imports and then faults inside its
  constructor on the first aligned store.

### Fixed

- `laned_versioned_map` and `raw_laned_versioned_map`: `sweep` treated a
  lane with nothing to free as a failure of the whole sweep. A lane that
  frees none reports `Full`, and the sweep propagated it, so it
  abandoned every remaining lane and discarded the count of what earlier
  ones had already freed. Any laned map with an idle lane therefore
  swept nothing while reporting it was out of room. Each lane's `Full`
  now means only that this lane freed none; the sweep reports `Full`
  itself only when no lane freed anything, which is the signal an insert
  out of room needs.
- `sens_rlc`: an item too large for a symbol reached the slice copy
  inside `pack_symbol` and panicked, in release as well as debug,
  because the assert guarding it is a debug one. The send path refuses
  it with both sizes named.

### Changed

- `subetha-py`'s adaptive ring is held by a shared handle rather than a
  box, so a network bridge can take one of its own and both name the
  same ring.

- `SENS_O_MATIC_WIRE.md` section 10 gains the literature the two codes
  come from, and two further RFCs, each entry naming the mechanism it
  bears on and where this format departs from it: RFC 9407 (Tetrys)
  against the sender-named coding window of section 4.2, RFC 3393 (IPDV)
  against the unsynchronized clocks section 4.1 differences, and a new
  section 10.3 carrying Reed and Solomon 1960, the Cauchy construction of
  Blömer et al. 1995, random linear coding in Ho et al. 2006, the finite
  sliding window of Wunderlich et al. 2017, tunable sparse coding in Feizi
  et al. 2012, and the SIMD field arithmetic of Plank et al. 2013 that
  section 3.1's log tables have as a conforming alternative.

## [0.3.1] - 2026-09-12

### Changed

- `subetha-ffi`: a handle borrow is a generation check, two acquire
  loads and the epoch guard. The handle table keeps its slots in a
  directory of fixed chunks placed once and never moved, and the object
  behind a slot as a pointer swapped in on insert and out on destroy; the
  two `arc-swap` snapshots a borrow took before, four locked
  read-modify-writes per call for the debt slot each claims and repays,
  are gone, and the epoch guard was already what kept a destroy from
  freeing an object under a call. Per call on the AVX-512 Windows build
  host: 12.4 ns in 0.3.0, 7.1 ns now, the minimum of three sweeps each.
  Every family's measured cost is in the crate README.
- `subetha-ffi`: an entry point takes its handle borrow inside the panic
  guard rather than before it, so a panic while borrowing is caught and
  answered as `SUBETHA_E_PANIC` like any other, and poisoning after a
  panic checks the handle's generation, since the borrow has been given
  back by the time the panic is recorded.

### Added

- `subetha-ffi`: a `test-hooks` cargo feature gating three entry points
  that measure the boundary a layer at a time, the panic guard alone
  (`subetha_test_entry_only`), the borrow alone
  (`subetha_test_borrow_only`) and the atomic family's dispatch alone
  (`subetha_test_atomic_borrow_only`), for `benches/ffi_overhead.rs`,
  which gains those rows and two that pass a memory ordering through.
  No shipped build carries the feature; the header declares the hooks
  under `SUBETHA_TEST_HOOKS`.
- `subetha-ffi`: `benches/ffi_overhead.rs` ships in the published crate,
  since the README describes it. It builds only under `cargo bench`.

### Fixed

- The crate README's "Cost of the boundary" said the difference between
  the direct and the C ABI columns was the same in every row, about
  11 ns; the table under it ran from 9 to 31 ns a call. The text now
  gives the range from the table and says what the ring rows carry
  beyond the boundary. The 0.3.0 entry below repeats the 11 ns figure
  and is superseded by this one.

## [0.3.0] - 2026-09-11

### Added

- `subetha-ffi`, the C ABI: generation-checked 64-bit handles, domain-
  grouped error codes named by `subetha_strerror` with a thread-local
  detail, panics caught at every boundary and reported as
  `SUBETHA_E_PANIC` with the handle poisoned, strict and managed modes per
  object with a process default set at `subetha_init`, and the adaptive
  ring end to end: anonymous, file-backed and shared-memory, registration,
  try and waiting push and pop, stats, wake and unlink. The header
  `include/subetha.h` is generated by cbindgen and committed, `subetha.def`
  lists the exports for the Windows linkers, and drift in either fails
  the test suite.
- The C ABI's data plane: every ring and channel through `subetha-ffi`,
  each with a try and a waiting form of its operations parking on
  cross-process wakers kept beside the backing, stats, wake and unlink.
  Frames past the slot on the adaptive ring (`subetha_ring_send_frame`,
  `subetha_ring_recv_frame`); ordering stamps, merge modes and the
  producer/consumer contract in `subetha_ring_options`, with the stamped
  pop; the SPSC ring (`subetha_spsc_*`); the MPSC pool
  (`subetha_mpsc_*`); the MPMC grid (`subetha_mpmc_*`); the Vyukov ring
  with its stuck-slot scan and heal (`subetha_vyukov_*`); the Lamport
  pair (`subetha_lamport_*`); the broadcast ring (`subetha_broadcast_*`);
  pub/sub with anonymous and file-kept subscriber positions
  (`subetha_pubsub_*`, `subetha_subscriber_*`); the capacity-adaptive
  ring with morph, compound morph and prewarm (`subetha_capacity_*`); the
  locale ring with migrate and the managed request
  (`subetha_locale_ring_*`); the capacity-adaptive broadcast and pub/sub
  rings (`subetha_capacity_broadcast_*`, `subetha_capacity_pubsub_*`,
  `subetha_capacity_subscriber_*`); the exact-order receiver on a stamped
  ring (`subetha_ring_ordered_receiver`, `subetha_ordered_*`); the shared
  Treiber stack (`subetha_stack_*`) and the work-stealing deque
  (`subetha_deque_*`), both at an element size the caller declares in
  `subetha_element_layout`, whose tag and alignment are stored in the
  region and checked on every attach. Handle kinds 2 through 20, codes
  113 through 117.
- `subetha_capacity_open` and `subetha_locale_ring_open` attach a second
  process to a capacity or locale ring without re-creating its backings,
  which on Windows refuses a file another process has mapped. In
  `subetha-cxc`, `CapacityAdaptiveRing::open` and
  `LocaleAdaptiveRing::open`.
- The pollable notifier: `subetha_ring_notifier` and `subetha_spsc_notifier`
  attach, for the calling process, a named FIFO on Unix or a named
  manual-reset event on Windows that every push on the ring from any
  process signals; `subetha_notifier_native` hands out the file descriptor
  or event `HANDLE` for an event loop, `subetha_notifier_drain` clears a
  pending signal, `subetha_notifier_wait` and `subetha_notifier_is_signaled`
  serve a caller without a loop. Handle kind 21. In `subetha-cxc`,
  `cross_process_notifier::{NotifierSet, Notifier}` with a 64-byte record
  beside the ring's backing.
- The C ABI's shared state begins: the shared hash map (`subetha_hashmap_*`,
  handle kind 22) at key and value sizes the caller declares, with
  insert, insert-if-absent, get, contains, remove, compare-exchange, a
  cursor walk, compact, clear, stats, flush and unlink; codes 118
  `SUBETHA_E_MAP_FULL` and 119 `SUBETHA_E_MAP_KEY_ABSENT`. In
  `subetha-cxc`, `RawHashMap`, the same table at run-time sizes, sharing
  the typed map's layout and protocol.
- The shared string arena (`subetha_arena_*`, handle kind 23): intern, get
  by copy and view in place through the 64-bit reference an intern
  returns, the reference helpers, a read-only open, clear, stats, flush
  and unlink; codes 120 `SUBETHA_E_ARENA_FULL`, 121
  `SUBETHA_E_ARENA_INVALID_REF` and 122 `SUBETHA_E_READ_ONLY`.
- The shared vec (`subetha_vec_*`, handle kind 24) and the shared slab
  (`subetha_slab_*`, handle kind 25) at an element layout the caller
  declares in `subetha_element_layout`, each with a read-only open; code
  123 `SUBETHA_E_OUT_OF_BOUNDS`. In `subetha-cxc`, `RawVec` and
  `RawSlab`, the same regions at run-time layouts, and the vec and slab
  headers record the element size, the slot geometry, the alignment and
  a layout tag; an open that states another layout is refused, and an
  element wider than a cache line or aligned wider than 8 gets a slot
  geometry of its own.
- The shared region (`subetha_region_*`, handle kind 26) at an element
  layout the caller declares: allocate, free, get, set, clear, stats,
  flush and unlink, a slot named by a 32-bit index every process resolves
  the same way. In `subetha-cxc`, `RawRegion`, the same arena at a
  run-time layout, and the region header records the element size, the
  alignment, where the slots start and a layout tag.
- Batch entry points: `_try_push_many` and `_try_pop_many` on the adaptive
  ring, the SPSC ring, the MPSC pool, the MPMC grid, the Vyukov ring and
  the Lamport pair, `_try_push_many` with `_try_recv_many` on the
  broadcast ring, `_try_push_many` with `_try_pop_many` on the stack and
  `_try_steal_many` on the deque. One handle lookup and one panic guard
  carry a run of operations over the caller's own array, addressed by a
  base and a stride. A batch stops at the first refusal and reports how
  many it completed.
- A handle borrow raises the object's active count and takes the
  object's addresses, with no reference count cloned per call. The
  boundary costs about 11 ns a call, and every family's measured cost is
  in the crate README.
- The shared B-tree map (`subetha_btree_*`, handle kind 29), ordered by
  unsigned byte comparison of the key so every language agrees on it
  without a comparator crossing the boundary: insert, get, contains,
  remove, first, last, clear, stats, flush and unlink. In `subetha-cxc`,
  `RawBTreeMap` at run-time key and value sizes, with an ordered range
  walk taking both bounds and a limit, which carries its own magic
  because the typed map orders by `K: Ord` and would put the same keys
  elsewhere in the same file.
- The shared fence clock (`subetha_fence_clock_*`, handle kind 44): a
  hybrid logical clock per participant in one file, and the global fence:
  the latest clock across live slots, so every event any participant has
  recorded stands at or below it. Register,
  unregister, tick, merge, the local clock, the fence computed or
  published or read back, a slot's snapshot, stats, flush and unlink.
  `subetha_hlc` carries the `(physical_us, logical)` pair a merge orders
  by. A slot is a plain `uint32_t` index rather than a handle, so nothing
  is allocated per participant and nothing has to be closed. Code 128.
- The cross-process waker (`subetha_waker_*`, handle kind 48): parking
  and waking between processes, down on the platform's own futex -
  `WaitOnAddress` and the hardware monitor on Windows, `futex` on Linux,
  `_umtx_op` on FreeBSD - so a parked thread costs nothing until it is
  woken. A consumer parks at the sequence number it waits for and a
  producer that reaches it wakes it; the sequence is the caller's own.
  Create, open, reset, park, wait, release, the three wakes, stats and
  unlink. A park is a token from the shared hold table, so a second
  release of one is refused rather than freeing a slot its next holder
  relies on, and `wait` gives the park back itself. A full table is
  `SUBETHA_E_RING_WAKER_FULL`, which a caller answers by spinning on
  whatever it wanted.
- The lazy value (`subetha_lazy_*`, handle kind 47): a value produced
  once across every process that asks for it, so a fleet starting
  together makes one request against whatever serves the value rather
  than one each. `claim` hands the right to produce it to a single caller
  and refuses the rest; the winner publishes; the others wait. Codes 132
  and 133.

  In `subetha-cxc`, `SharedOnceCellDyn`: the once cell with its payload
  size given at run time and its initialization split into claim, produce
  and publish, for a caller that cannot pass a closure. A claim stands
  across the caller's own code, so it carries the process that took it:
  `wait` answers `ClaimantGone` once that process is gone rather than
  waiting out a deadline for a publish that will never come, and
  `reclaim` returns the cell to empty for the next caller. `publish`
  takes the claimant's pid, so a caller that lost the claim cannot
  overwrite the winner's value.
- The shared value (`subetha_shared_arc_*`, handle kind 46): a region of
  shared memory kept alive by the processes holding it and released when
  the last one lets go. Create, open, read and write
  by offset, the holder count, dead-holder reaping, stats, flush and
  unlink. `SUBETHA_ARC_UNLINK` removes the backing when the last holder
  releases; `SUBETHA_ARC_KEEP` leaves it for a process attaching later.
  Code 131 when every holder slot is held by a live process.

  In `subetha-cxc`, `SharedArcDyn`: `SharedArc` with the value's size
  given at run time rather than by a type, for callers that cannot name a
  Rust one. It shares the header, the holder table, the reaping and the
  last-holder policy, and because the header already recorded the value's
  size, a dyn arc of `size_of::<T>()` bytes and a `SharedArc<T>` attach to
  each other's backing while a length that disagrees is refused. Two
  obligations are stated rather than implied: the layout inside those
  bytes is the caller's to agree on, and a write is a copy with no
  ordering, so mutation needs an atomic in the region or a lock around it.
- The epoch barrier (`subetha_epoch_barrier_*`, handle kind 45): a
  rendezvous where each process waits at an epoch until the others reach
  it. Create, open, four waits - every live peer or a quorum, each with
  and without a deadline - the current epoch, the live peer count, stats,
  flush and unlink. What it promises is order rather than duration: the
  early arriver does not pass until the late one is there. It counts
  arrivals in one shared word and takes how many peers exist from a
  heartbeat table, so a process that dies between rounds stops being
  waited for once its slot goes stale instead of stalling the round for
  good. It is built from a heartbeat handle rather than a path, so it
  counts peers in the same table its caller beats into rather than a
  second view of the same file. Codes 129 and 130 name the two refusals a
  caller acts on differently: nobody is there to wait for, and the round
  asked for is already over.
- A lock hold, a semaphore permit, an epoch pin and an epoch ticket are
  tokens rather than handles, released by `subetha_rwlock_unlock`,
  `subetha_semaphore_release`, `subetha_pin_release` and
  `subetha_ticket_publish`. A handle is issued with a heap allocation and
  two mutexes and closed with a process-wide barrier, whose price rises
  with how many threads the process runs. That is right for a ring closed
  once at the end of a run and wrong for a lock released in a loop.
  Taking a token is a compare-exchange and giving it back is another,
  with nothing allocated and no lock taken. Measured on the AVX-512
  Windows build host, a lock acquire and release costing 3.0 ns of direct
  work: 42.1 ns through the ABI with eight threads in the borrow guard's
  registry, and 45.0 ns with seven other threads merely running. An epoch
  pin and release reads 52.7 ns and a semaphore acquire and release
  57.4 ns, against 21 to 35 ns of boundary for every other family. A
  registry size belongs with any of these figures rather than being a
  detail, because the registry never shrinks, and
  `subetha_borrow_registry_len` reports it so a bench records what it
  measured against instead of inferring it.
- A token and a handle are told apart by the ABI rather than by the
  caller. Both are 64 bits and both carry an index and a generation, so
  every token carries a tag bit that no handle has:
  `subetha_handle_destroy` refuses a token and names it, and the release
  entry points refuse a handle and name that. Destroying the object that
  issued a token gives that token's hold back, so nothing is stranded.
  Each slot carries a generation that steps on every release, so a token
  given back twice is refused, as is one whose slot has been taken since;
  an all-zeroes token, which is what an uninitialized variable holds,
  names nothing.
- The borrow guard's registry is read through an `ArcSwap`, so a handle
  close takes its snapshot with one reference count operation and no
  lock. The mutex serializes threads joining and leaving.
- Batch forms for the three shared-state families whose item is not a
  uniform payload: `subetha_vec_push_back_many` and
  `subetha_vec_get_many`, `subetha_arena_intern_many`, and
  `subetha_hashmap_insert_many` with `subetha_hashmap_get_many`. A vec
  push answers with the index the element landed at and an arena intern
  with the reference naming the bytes, so those two fill a word per item
  that landed rather than dropping the answer the call exists to give;
  another process may append between two batches, so a caller cannot work
  the indices out from a length it read beforehand. The map's item is a
  key and a value, which a caller already holds as two columns, so its
  forms walk two strided arrays rather than making it pack pairs.
- `subetha_abi_version()` reports 0.6.0: the minor is the highest shipped
  tier plus one, so a consumer reads it to learn which families it is
  linked against rather than which release it holds.
  `abi_version_matches_the_tiers_document` in `tests/header.rs` holds the
  constant against `C_ABI_TIERS.md` so the two cannot drift, and asserts
  the major is 0: the freeze is gated on a consumer outside this
  repository having used the ABI, not on the tiers having shipped.
- The holder table (`subetha_holders_*`, handle kind 43): the substrate
  under the epoch table's pins and tickets, reached directly. A fixed
  array of claimable slots, each carrying one `uint64_t` of the caller's
  own meaning and the process that claimed it, so
  `subetha_holders_reap_dead` can free every slot whose process is
  gone - the question a bare count cannot answer, because a number
  cannot be asked whether it is still running.
  `SUBETHA_HOLDER_FREE` and `SUBETHA_HOLDER_RESERVED` are refused as
  payloads rather than stored, since either makes a held slot read as
  something else. Claim in one step, or reserve and publish when the
  payload depends on state read after the slot is visible. In
  `subetha-cxc`, `SharedHolderTable` gives the bare `HolderTable` view a
  file of its own.
- The heartbeat table (`subetha_heartbeat_*`, handle kind 42): the lease
  and the election each track one holder, and this tracks a whole fleet.
  Each process registers a slot and beats into it; the slot carries the
  process id, the epoch it last beat at, a bitmap of up to
  `SUBETHA_HEARTBEAT_IN_FLIGHT` work units it has taken, and a role the
  caller assigns meaning to. A slot nobody holds reads as empty rather
  than as an error, so a supervisor walks every index to find who has
  gone quiet and what work they were holding when they went. Each slot is
  its own cache line and is written under a version a reader retries on.
- Leader election (`subetha_leader_*`, handle kind 41): among the
  processes attached to one file, exactly one leads, and which one
  converges without a vote. The lowest live process id wins; ids are
  unique on a host, so a set of live processes always agrees, and it
  takes one compare-exchange rather than a quorum. The election term goes
  up on every handover, so a follower watching for a change polls the
  term rather than the process id, which can come back to a value it held
  before.
- The owner lease (`subetha_owner_lease_*`, handle kind 40): one process
  at a time holds a resource, and a holder that dies loses it rather than
  keeping it for good, which is what `subetha_rwlock_*` cannot do. A
  lower process id preempts outright; a heartbeat that has fallen more
  than `grace_epochs` behind the global epoch marks a holder gone. The
  window is counted in epochs and nothing advances them on its own, so
  whatever calls `subetha_owner_lease_tick_epoch` sets how quickly a
  stale holder is detected, and a holder keeps its claim by beating
  faster than that. A payload of up to `SUBETHA_LEASE_PAYLOAD_MAX` bytes
  travels with the lease at a size the region records and checks at every
  attach, readable and writable only by the holder and under a version so
  a torn value is never published. `subetha_current_pid` is there so a
  program using the ABI needs nothing else to work the lease. In
  `subetha-cxc`, `RawOwnerLease` at a run-time payload size, which lays
  out its region through `begin_lease_region` and `publish_lease_magic`,
  as the typed lease does. Code 127.
- The condition variable (`subetha_condvar_*`, handle kind 39): park
  until another process says something changed. The Rust API takes a
  predicate closure, which does not cross the boundary, so the predicate
  stays with the C caller as it does in C generally: read
  `subetha_condvar_generation`, check your own predicate, and pass that
  generation to `subetha_condvar_wait`, which returns at once when a
  notify has already moved past it. That is what stops a notify landing
  between the check and the wait from being lost, and it is the same
  shape as a futex's expected-value argument. `notify_one` and
  `notify_all` report how many they woke, destroying the handle releases
  every parked caller, and `subetha_condvar_unlink` removes both backing
  files.
- The counting semaphore (`subetha_semaphore_*`, handle kind 37): a
  bounded number of permits any number of processes take and give back. A
  permit is a token `subetha_semaphore_release` returns, and
  `subetha_semaphore_held` reports how many are out, which is where a
  leak shows. `subetha_semaphore_acquire` waits up to a `timeout_ms` and
  registers among the waiters while it does, so a release from any
  process still advances the wakeup generation the Rust waiters watch;
  destroying the semaphore's handle releases every caller waiting on it
  and gives back every permit it issued. `subetha_semaphore_unlink`
  removes all three backing files.
- The reader-writer lock (`subetha_rwlock_*`, handle kind 35): many
  readers or one writer across processes, with writer priority. A hold is
  a token `subetha_rwlock_unlock` gives back, and destroying the lock's
  handle gives back every hold it issued. The waiting acquires take a
  `timeout_ms` and poll, since
  a release on this lock signals nobody, and destroying the lock's handle
  releases every caller waiting on it, so a destroy and
  `subetha_shutdown` both complete while an acquire is outstanding. In
  `subetha-cxc`, `SharedRWLock` gains `register_waiting_writer`,
  `unregister_waiting_writer` and `try_write_lock_registered`, which
  `write_lock` is built on; a writer waiting through the ABI
  registers for the length of its wait and so takes the lock ahead of
  readers arriving after it. Code 126.
- The epoch table (`subetha_epochs_*`, handle kind 32), the first of the
  C ABI's coordination tier. A scan takes a pin and reads one fixed view
  of a store while writers keep running; a compound write takes a ticket
  so everything it stamps becomes visible at once. Both are tokens:
  `subetha_pin_release` releases a pin and `subetha_ticket_publish`
  publishes a ticket, which is what the Rust guards do on drop, and
  destroying the table's handle gives back every one it issued.
  `subetha_epochs_dead_tickets` and `_free_dead_ticket` recover an epoch
  whose writer died mid-compound. In `subetha-cxc`, `SharedEpochs` gains
  `claim_pin` / `release_pin` and `claim_ticket` / `publish_ticket`,
  which `PinGuard` and `EpochTicket` are built on, so the claim
  protocol has one implementation rather than a second copy behind the
  ABI. Codes 124 and 125.
- The frame region (`subetha_frame_region_*`, handle kind 31): fixed-size
  blocks in a file for payloads too large to travel inside a ring slot,
  which is the region an adaptive ring builds for itself, reached
  directly. Create, open, reset, alloc, free, write, read, stats and
  unlink. Any process may free any block. The free list runs through the
  blocks themselves, so a free overwrites the first four bytes of the one
  it takes, and both the C test and the cross-process peer are built
  around that. In `subetha-cxc`, `FrameRegion::read_block` copies into a
  caller's slice, beside `read_block_into`, which appends to a `Vec`.
- The shared cell (`subetha_cell_*`, handle kind 30) at a value size the
  caller declares, recorded in the region, so a four-byte cell and a Rust
  `SharedCell<u32>` are one region and a handle declaring another size is
  refused. In `subetha-cxc`, `RawCell`.
- The shared linked list (`subetha_list_*`, handle kind 28) at an element
  layout the caller declares: push and pop at both ends, get, set,
  removal from the middle by the node's index, and a walk from either
  end. In `subetha-cxc`, `RawLinkedList` over `RawRegion`, whose
  `element_ptr`, `allocate_with` and `free_slot` let a caller touch part
  of an element in place.
- The shared atomics (`subetha_atomic_u32_*`, `subetha_atomic_u64_*`,
  `subetha_atomic_bool_*`, handle kind 27): load, store, swap, the fetch
  operations and compare-exchange, each in a sequentially-consistent form
  and an `_explicit` form taking a `SUBETHA_ORDER_` value, with the
  orderings an operation cannot have refused rather than panicked on.
- `subetha_ring_options` carries `frame_block` and `frame_blocks`, the frame
  region geometry an adaptive ring is created with, and `shm_sddl`, the
  security descriptor its shared-memory regions and wakers are created
  with; a shared-memory ring is created and opened through the secured
  constructors.
- `subetha-ffi-tests` carries `c/workloads.c` and `tests/workloads.rs`:
  fifteen workloads shaped like the programs that use the library, driven
  through the C ABI in every locale and mode their shape allows, each
  printing what it measured, with `SUBETHA_FFI_SOAK_SECS` repeating them
  for a soak. Ten spawn every role as a real child process, and nine are
  shaped like the programs that use the rings.
- The other six are shaped like the programs that use the shared state,
  the coordination and the transports, every role in C and a real child
  process: a content-addressed blob store (a hash map from an fnv hash to
  an arena reference, writers storing overlapping sets, readers on the
  arena read-only), a content index rebuilt generation by generation
  under an owner lease (a vec of records over an arena, reset at a fresh
  path, flushed, published through a counter, read read-only), a
  multi-version index (a versioned map changed by writers one at a time
  under a lock, as the tree beneath it requires, while readers scan under
  pins from the shared epoch table and no lock), a graph store on a frame
  region (chains of edge pages with a version word per page, writers
  appending and pruning, readers walking and re-reading a page caught
  mid-write), a record store under one reader-writer lock (ids in the
  strategy-switching set, records in a hash map, the set migrated under
  the write hold), and a cluster stream (sealed Sens-O-Matic streams from
  several sender processes into one receiver, more than one per process,
  the accounting closed against the kernel's own drop report). Twelve
  cells, strict and managed; every cell checks content from seeds and
  removes its backings through the ABI's own unlink calls.
- The line-log workload's writer ends on an end slot the harness pushes
  once every producer has returned, and on nothing else, so a producer
  starved on a loaded box cannot resume after the writer has gone and
  fill the ring with whole lines nobody wrote. Every test opens by
  closing whatever an earlier panicked test left, so one cause is one
  failure rather than the next test's shutdown reporting the leak as its
  own.
- The suite's `sleep_us` waits on a high-resolution waitable timer on
  Windows rather than on `Sleep`, whose granularity is the scheduler
  tick: at the default tick that host charges about 11 ms for any wait
  of a millisecond or less, so a poll asked for in microseconds cost
  tens of times what it asked and a loop bounded by a count of polls
  spanned far longer than the count was chosen for. Fourteen poll loops
  across nine cells reach the wire through that one helper. The stream
  cell's receiver now polls about 240 times a cycle where it polled 19,
  and its strict mode runs 133 soak passes with no operation over a
  second, where before it failed at 36 cycles and 205 of its 240 sender
  operations sat past 100 ms.
- The stream receiver stays past the last stream for as long as a sender
  is allowed to wait on one. It is the only thing that acknowledges a
  stream and it acknowledges only from inside its poll loop, so a
  shorter stay ends the only source of acknowledgment while a sender is
  still owed one, and what reaches that sender is indistinguishable from
  a peer that stopped responding. `STREAM_LINGER_MS` is defined as
  `STREAM_FINISH_MS` rather than as its own number, so the two cannot
  drift apart.
- The stream receiver polls at the rate its decoder is driven: 200
  microseconds in strict mode, where the polling thread drives the
  decoder itself, and 2 milliseconds in managed mode, where a sidecar
  thread drives it and the polling thread only carries away what that
  sidecar has already delivered. A poll faster than the sidecar finds
  nothing and spends the core the sidecar needs, which is why the cell
  passed in strict and failed in managed once the timer above took the
  poll from 11 milliseconds to half a millisecond. The empty-queue wait
  is what changes; a poll that finds an item processes it and continues
  with no wait, so a cycle's 1200 items still cost 1200 fast iterations.
  The cell now passes both modes on that host, 38 strict soak passes and
  39 managed, with no operation over a second, while the box carried
  another agent's build at normal priority and the gate ran at idle.
- `subetha-cxc`: `RawTreiberStack` and `RawDeque`, the Treiber stack and
  the Chase-Lev deque at a slot size chosen at run time, sharing their
  region layout and protocol with the typed `SharedTreiberStack` and
  `SharedDeque`; the headers carry the element alignment and a layout
  tag (`align_of::<T>()` and zero from the typed types).
- `subetha-ffi-tests`: a C suite compiled by the host's own compiler with
  every warning an error and run from `cargo test`, including a C consumer
  in a second process draining a ring through the file-backed wakers, and
  one such peer per family: thirty-two cross-process peers, each a real
  second process.
- `cargo run -p xtask -- ffi-install --prefix <dir>` lays out `include/`, `lib/`,
  `bin/` on Windows, a pkg-config file and a CMake package config for
  `find_package(subetha)`; `cargo run -p xtask -- ffi-package-gate` builds
  and runs a CMake consumer against that tree, reports a missing `cmake`
  or `pkg-config` as SKIPPED, and compiles the same consumer directly with
  the host's C compiler, shared and static. The package config sends the
  options in rustc's `native-static-libs` list, such as MSVC's
  `/defaultlib:msvcrt`, to `INTERFACE_LINK_OPTIONS`, and names as
  libraries only the entries that are libraries, so the static consumer
  links.
- `AdaptiveRing::unlink` removes every backing a prefix names, including
  backings grown past the creation hint, and reports the removed, missing
  and failed counts.
- `C_ABI_TIERS.md` at the repository root states the order in which the C
  surface ships.
- `subetha_ring_options` carries `max_holders` and `last_holder`, an opt-in
  at creation for a file-backed adaptive ring to be removed by whichever
  process lets go of it last: the holders are counted in a region of
  their own beside the peer directory, a holder whose process died is
  reaped by probing the pid, and the last release removes the backings
  together with the wakers and the notifier record the ABI keeps beside
  them, into one report. `SUBETHA_LAST_HOLDER_KEEP` with zero holders is
  a ring that counts no holders and removes nothing of its own; asking to
  unlink with `max_holders` zero is refused rather than given a number
  nobody chose. Anonymous and
  shared-memory rings refuse the option by name. In `subetha-cxc`,
  `AdaptiveRing::with_last_holder` and the `ring_holders` module.
- The C ABI's transports, tier 4. Sens-O-Matic (`subetha_sens_*`, handle
  kind 51): the unified sender and receiver that switch erasure code on
  measured loss, and the `_rlc_` and `_rs_` halves that pin one code and
  report zero switches because they cannot switch; all six share `send`,
  `poll`, `flush`, `local_port` and `read_stats`. Strict mode makes the
  poll the pump, on the calling thread; managed mode owns the thread
  that drives the decoder. With the `tls` feature, `subetha_sens_sender_tls`
  and `subetha_sens_receiver_tls` seal every item under TLS 1.3 and take
  the certificate and key as DER bytes, `subetha_sens_self_signed_cert`
  mints a pair, and `subetha_sens_tls_available` says whether the build
  carries the path; without the feature every entry point exists and
  answers `SUBETHA_E_NOT_SUPPORTED` naming it. The TCP and QUIC bridges
  (`subetha_tcp_bridge_*`, `subetha_quic_bridge_*`, kinds 49 and 50)
  carry items between a ring here and a peer there, a client or a server
  half each, with `run` under a timeout and `read_stats`; the QUIC bridge
  takes its certificate as DER bytes and `subetha_quic_self_signed_cert`
  mints one; `subetha_transports_available` answers for the pair, which a
  build carries or not independently of the sealed Sens path. The blocking
  bridge (`subetha_blocking_tcp_bridge_*`, kind 72) carries a blocking
  SPSC ring the same way, parking on the ring instead of yielding, so an
  idle bridge costs no CPU and a consumer that stops draining ends the
  run with an error rather than a discard; it rides the same feature and
  the same `run` and `read_stats` shape. Virtual
  endpoints (`subetha_endpoint_*`, kind 63): a registry of named targets,
  each a local locale-ring handle or a remote address, read together with
  the generation it was read at, and `_still_valid` in place of the
  lifetime borrow the Rust pinned endpoint takes. The quality-of-service
  policy (`subetha_qos_*`, kind 64), each field its own atomic and no
  call that writes the whole policy at once.
- `subetha_sens_finish(handle, timeout_ms, out_acked)`: wait up to the
  deadline for the far end to acknowledge everything a sending half sent,
  retransmitting what it has not, the way the Rust `finish` does, so a C
  sender ends a stream rather than closing on a tail that no repair
  follows. `subetha_sens_stats` carries `unroutable`,
  `preauth_dropped`, `unopened` and `handshake_failures`, a unified
  receiver's own accounting of what reached the process and was not
  delivered, so a stream that never opens shows where it stopped rather
  than as a zero drop count.
- What a strict-mode transport loses in the kernel is reported as far as
  the host allows, and the report says which case it is.
  `subetha_sens_stats` carries `missed`, the datagrams the far end never
  reported receiving, which only a unified sender learns, and
  `kernel_dropped`, the datagrams this host's kernel dropped for a full
  receive buffer, each paired with a report of `SUBETHA_DROPS_EXACT`,
  `SUBETHA_DROPS_OCCURRED` or `SUBETHA_DROPS_UNKNOWN`. Linux counts per
  socket through the `SO_RXQ_OVFL` ancillary message; FreeBSD's `SO_RERROR`
  turns an overflow into an error on receive, which says that it happened
  and not how often; Windows moves no counter. A zero paired with anything
  but `EXACT` is not a claim that none were lost. In `subetha-cxc`,
  `dgram::DropReport` and `DropTally`, `kernel_drops()` on the demux
  socket and every receiver, and `missed()` on the unified sender.
- The C ABI's probabilistic and specialist structures, tier 5. The Bloom
  filter, the blocked Bloom filter, the count-min sketch and HyperLogLog
  (`subetha_bloom_*`, `subetha_blocked_bloom_*`, `subetha_cms_*`,
  `subetha_hll_*`, kinds 52, 56, 53 and 66), each with a `_suggest` that
  sizes it from a target; the histogram with `_percentile` and
  `_bucket_for`, the reservoir sampler with `_snapshot`, the rate limiter
  with `_try_acquire` and `_acquire_wait`, and the topology map recording
  fan-out and fan-in (kinds 54, 57, 55 and 58); the versioned chain, map
  and slab and the laned map (kinds 59, 61, 60 and 62), epoch-versioned
  structures whose reader pins an epoch and whose writer sweeps or voids
  one once every reader has left it, the map and the slab compiled at
  three size classes named by `SUBETHA_VERSIONED_*` and runtime-sized
  otherwise; the bit vector (kind 65), whose `set`, `clear` and `toggle`
  write what the bit was into `was`, so `set` is a claim between
  processes; NaN-boxed values (`subetha_nan_*`), one `uint64_t` the
  caller holds and the only family with no handle, no mode and no need
  for `subetha_init`, with a two-level tagged form
  (`subetha_nan_from_tagged`, `_as_tagged` and `_is_tagged`) carrying an
  index and a tag at a tag width the caller picks, which
  `subetha_nan_type` reports as `SUBETHA_NAN_RESERVED` because it reads
  only the first level; the LRU cache (kind 67), where `get` leaves recency
  alone and `get_and_touch` promotes and a miss leaves the value buffer
  as it was; the directed graph (kind 68), out-edges walked with
  `first_edge` and `next_edge` to `SUBETHA_GRAPH_NIL` and node removal
  not offered because every edge pointing at a node lives in another
  node's chain; the time-point tile (kind 69), sixteen lanes with
  `visible_mask` answering all of them in one call; the content-prefix
  pointer (`subetha_umbra_*`), sixteen bytes the caller holds and
  resolves against a region handle, with `_prefix_eq` named for what it
  is; the strategy-switching set (kind 70), moved between a vector and a
  hash map by `_migrate` alone, its strategy word kept in a state file
  beside the two backings so a migration through one handle is what every
  process reads, mapped by every handle and read on every operation as
  the typed `SharedUniversal` keeps it, its open taking no strategy and
  keeping the one in force
  (`subetha_universal_open(base_path, capacity, element_size, mode,
  out)`), its migration publishing by compare-exchange over the word it
  read so a word that moved is reported rather than overwritten, and its
  stats carrying the stamp a reader compares across an operation; and the
  cascade tower (kind 71), at a
  depth set by the levels the caller supplies, `get` refusing at the
  first level of a path that no longer agrees rather than resolving
  whatever now sits at its end.
- In `subetha-cxc`, six cores at element sizes chosen at run time, so a
  C caller reaches the generic structures without a fixed instantiation
  per type: `RawLruCache` over `RawHashMap` and `RawLinkedList`, every
  read checking that the slot the map named still holds the key;
  `RawGraph` over two `RawRegion`s; `RawUmbraPointer`, a 16-byte value;
  `RawTimePointTile`, refusing version 0 and clearing a removed lane's
  version before its occupancy; `RawKTower`, its depth taken from the
  levels supplied; and `RawUniversal`, one word carrying strategy,
  generation and version, published after the elements reach the target
  backing, a no-op migration burning no version.
- `subetha-cxc`: `SharedArray`, a flat read-only alternative to
  `SharedSlab` and `SharedVec` for a table written once and read from
  many processes. One 64-byte header carries the magic, the stride, the
  length and a sealed flag; the elements follow at their own stride with
  no per-slot lock or version, so a `u8` costs one byte rather than the
  64 a slab slot takes. `create`, `fill_from`, `set`, `seal`, which
  flushes before it sets the flag, `open_read_only`, which maps the file
  without write access, `get` and `as_slice`. The slab and vec module
  docs state what a slot costs and what that cost buys, with the packing
  that recovers it on the published crate: a slot-sized array such as
  `[u8; 56]` keeps the seqlock at 1.14 times the payload rather than 64.
- `subetha-cxc`: `residue_fec`, a residue-number erasure code recovered
  by the Chinese remainder theorem rather than by a linear system, behind
  the off-by-default `residue-fec` feature. It is correct and its tests
  pass, and it is not what a caller wants: it encodes about 210 times
  slower than the RLC and recovers about 135 times slower, costs slightly
  more on the wire, and is a block code, so a loss waits for the rest of
  its block where the sliding window does not. The one property that sets
  it apart from a random linear code, generating no coefficient at all,
  the published taps have as well. It sits behind a feature rather than
  in the published surface because the optimizations its own notes call
  for would change its moduli and its chunk mapping, which a public
  module is a promise not to do.
- `subetha-cxc`: `region_file`, the one place a file-backed region is
  created, opened, opened read-only and removed, so a removal means the
  same thing on every platform; the ring prefixes, the raw primitives and
  the shared primitives all open through it.
- `subetha-cxc`: `ring_trace`, compiled only into a debug build, records
  what happens to a ring's ownership - the morph a push landed across,
  the peer counts and claimed bitmaps a shape decision was taken on, the
  producers and consumers arriving, leaving and being reaped, and the
  slot events behind each pop - and a failure dumps the whole trace held
  for that ring.
- Interoperability vectors a second implementation checks itself against:
  `crates/subetha-cxc/vectors/rlc.txt` for the sliding-window RLC and
  `vectors/rs.txt` for the block code, each written out by a test that
  fails when the code stops reproducing it.
- Managed mode's shape sidecar scans every
  `SUBETHA_SCAN_INTERVAL_DEFAULT_US` microseconds, 250, when a caller
  names none in `subetha_ring_options::scan_interval_us`. The cadence is
  managed mode's added round-trip latency rather than a tuned optimum:
  measured on Linux and FreeBSD from 250 to 10000 microseconds, a
  request and its response each wait for a scan, so a round trip costs
  exactly one interval and a streaming caller costs nothing, with no knee
  anywhere in that range.

### Changed

- The sliding-window RLC takes its coefficients from a published table
  of sixty-four constants rather than from a generator that derives them
  per repair. This changes the wire and 0.3.0 does not interoperate with
  0.2.x on the RLC repair path. A repair names its generator in the
  high nibble of `dt` - free, because the density occupies 0..=15 - so a
  decoder reads the choice off the stream rather than from its own
  configuration. A repair naming generator 0, which is what 0.2.x sends,
  is counted by `RlcDecoder::refused_repairs` and dropped, because its
  coefficients cannot be reproduced here and an equation with the wrong
  coefficients does not fail to solve, it solves to bytes that were never
  sent. `RlcEncoder` selects no generator: there is one.
  `rlc_fec::refused_repairs()` tallies those refusals for the process
  beside each decoder's own count, so a peer still sending 0.2.x repairs
  is visible without a decoder to hand.

  A fixed generator suffices because the code is convolutional -
  successive repairs cover windows that have shifted, so their equations
  differ though the generator does not, and the per-repair coefficient
  was doing work the window's own motion already did.

  What it costs was measured against the per-repair generator 0.2.x
  sends, on one wire stream per trial with the same dropped positions
  dealt to each: across random loss from 2% to 20% and bursts of 2, 3 and
  5, the taps came within 0.15 percentage points everywhere and were
  ahead in two regimes. What the table can be is bounded too: over four
  coding geometries the shipped table reaches the ceiling set by which
  repairs cover which symbols, so no table of any values recovers more.
  `benches/generator_recovery.rs` keeps the shipped code's recovery
  profile across those regimes, and `benches/tap_search.rs` the ranking
  and the controls that prove the ranking sees a bad table.

- `repair_key` is a repair sequence number: it names the repair in logs
  and traces and does not feed the coefficients, which come from the
  window.

- The published tap table holds one tap per position the widest coding
  window can hold - sixty-four, the controller's own maximum - and the
  density threshold selects a class spread across the window rather than
  a prefix of it: `dt` of 15 takes every position and `dt` of 0 one in
  sixteen, at every depth. A table shorter than the window caps how far a
  repair reaches, however wide the window is, and widening the window is
  the controller's answer to a long burst, so the table length is
  asserted against that maximum and the two constants cannot drift apart
  in silence.

- `AdaptiveRing` walks every backing it has been in, for the life of the
  ring, rather than remembering one previous shape. A producer resolves
  the shape tag and pushes some instructions later, so a backing can
  receive a push after it stops being current, and two morphs in a row
  would otherwise strand whatever landed in the first. There are four
  shapes, so never forgetting one is a bounded walk. A morph is not
  deferred behind an undrained backing and `RingError::StaleBacklog` is
  not returned; the variant stays as a reserved code.

- `HeartbeatTable::open` checks the file's length before mapping, so a
  caller declaring more slots than the file holds gets `LayoutMismatch`
  naming the disagreement rather than an `IoError` from mapping past the
  end of the file, which is how `OwnerLease::open` and the ring families
  answer it.

- The RLC decoder's row scale runs on the GF(2^8) SIMD ladder
  (`fec::gf_mul_auto`), beside the multiply-add sites around it, rather
  than a byte at a time: a symbol is up to a full datagram and a window
  reaches 64 pivots, so the scale is the same order of work as the
  accumulate steps it sits beside. The field and the coefficients are
  untouched, so the interoperability vectors are unchanged.

- A debug build of the SPSC ring claims the tail advance with a
  compare-exchange rather than a plain store, so a second reader on a
  core that admits one panics naming the consumer, the backing and the
  ring's recent history instead of leaving a duplicate to be inferred
  from what a consumer received. A release build keeps the plain store
  the single-reader contract allows.

### Fixed

- The shared hash map's walk (`subetha_hashmap_next`, `RawHashMap::next_entry`,
  `SharedHashMap::snapshot`) handed back a slot on its state alone, and an
  insert claims its slot before it writes the payload and publishes the
  hash, so a walk racing an insert copied the zeroed payload of an entry
  still being written. The blob-store workload's reader met it on every
  host once the cell soaked: a reference of zero, a 0-byte blob that did
  not hash to its key. A walk now passes over a slot claimed and not yet
  published, as a lookup for that key already waited on it.
- A capacity morph on `CapacityAdaptiveRing` and `CapacityBroadcastRing`
  pruned every drained backing from the stale list, while a producer
  pushes into the active backing of the state it loaded: a push delayed
  across two morphs landed in a backing no state named, no consumer's
  stale walk reached it, and a consumer waiting on a count spun forever.
  Seen once in ten parallel runs of the ring tests on a loaded Linux host.
  A drained entry now leaves the list only when the list is its last
  holder, so a backing a loaded snapshot still names stays reachable for
  the push that snapshot carries.
- The peer directory's liveness probe on Unix passed a slot's pid word to
  `kill` as a `pid_t` as it stood, so a word of `u32::MAX` became `-1`,
  which names every process the caller may signal and answers alive, and
  the dead-peer reaper kept such a slot forever. A value that does not
  fit a positive pid is now dead without a probe. The test storing
  `u32::MAX` as a crashed holder's pid had failed on Linux and FreeBSD
  every time and passed on Windows, whose probe refuses the value.
- A peer published its claim to the directory's bitmap before it
  published to the count the shape policy reads, so a shape decision
  taken in that window saw fewer peers than had registered and could
  choose a shape narrower than the peer set - a single-producer shape
  under several producers, which costs one item. A claim now raises the
  count before taking the bit and puts it back when no slot is free, so
  the count can only overstate, and a shape too wide for its peers
  carries them correctly. A contract ceiling checked against this count
  can now refuse a registration that a race would previously have
  admitted past the declared bound; refusing names an error, admitting
  broke the contract silently.
- A consumer arriving into a lower-numbered slot took the single-reader
  role off a consumer that was inside a pop on a Lamport backing. The
  role is checked before the pop and is not atomic with it, so the
  displaced reader kept copying while its replacement started and both
  took the same item. An incumbent that still holds its slot now keeps
  the role, and only a designation naming a slot nobody holds moves; an
  arrival cannot displace a walk in flight, and a departure cannot
  either, because the consumer that leaves is the one that would have
  been walking.
- Two threads could hold one consumer slot at once, putting two readers
  on a Lamport core that admits one and handing both the same item. A
  claim takes the directory's bitmap bit before storing its pid, so a
  recycled slot briefly reads as claimed by the sentinel its release
  wrote; the dead-peer reaper excluded pid zero, which is what a
  producer release writes, but not that sentinel, which is what a
  consumer release writes, and so freed a claim whose holder was
  running.
- A single-reader shape served consumer slot 0 alone, but slots come
  from the directory's lowest-free-bit claim, so a consumer that
  outlived the others kept whatever slot it had claimed and read empty
  on a ring that was not empty. The reader is now the lowest claimed
  consumer slot, re-derived on every topology sync.
- A ring could be left owned by a consumer that had departed: the
  rebalance assigns from a snapshot of the claimed slots and can hand a
  ring to one that is already unregistering, then ask that owner for a
  handoff no absent owner will apply. Recovery existed only through the
  crash takeover, which probes a pid and so runs on every 1024th empty
  scan, and a consumer that drains to the first empty and stops never
  reached it. An owner that released its slot needs no pid probe, so the
  scan reclaims such a ring at once.

### Documentation

- A tutorial for arriving with a C or C++ toolchain: the install layout
  per platform, linking through CMake, pkg-config or the compiler
  directly, a first ring, and the three separate calls a failure is read
  with.
- The cross-process waker and the lazy value carry a direct-Rust figure
  beside their through-the-ABI one, so what is recorded is the cost of
  the boundary rather than the cost of the operation.
- `SENS_O_MATIC_WIRE.md` at the repository root, the normative wire
  specification for the Sens-O-Matic transport, read from the source: the
  packet types, the RLC and Reed-Solomon headers field by field, the
  control frames and their variable-length encoding, connection
  identity, path validation and admission, the published tap table and
  the density nibble, and what version 1 does not interoperate with. Two
  of its hand-written tables are held against the code by tests, and the
  README and the wiki point at it as normative.
- The C ABI acceptance gate in the README: fifteen workloads driven
  through `subetha.h`, ten of them spawning every role as a real child
  process, and the 37 cells `SUBETHA_FFI_SOAK_SECS` soaks. It carries
  what the three legs measured at 240 seconds a cell, and states that
  the three hosts are two machines: Linux and FreeBSD are guests on one
  processor, so three operating systems is what three legs buy and the
  wall times are not independent of one another.
- Every published performance figure is recomputed from the data checked
  in beside it. Eleven stated ranges did not contain their own
  measurements, each because a bound had been rounded the wrong way: a
  range summarizing measurements takes its floor down and its ceiling
  up, and rounding both alike narrows the interval past the data while
  reading like ordinary tidying. The pinned-shape span is 36.8 to 114.9
  ns, where two documents had given 37 to 115 and 49 to 121 for the same
  quantity and neither contained it; the same correction runs through
  the per-platform leaderboard, its summary list, the block
  Reed-Solomon and TCP tail latencies, and the two wiki pages that
  carried them. A bridge document attributed a one-way figure to the
  leaderboard that the leaderboard does not hold, and the conclusion
  resting on it now states the measured relationship rather than an
  equality. A sentence pointing at a crate README that has never existed
  is gone. What was already exact is most of it: every ratio claim, every
  cell of the bridge tables, the ordering-mode table, and the in-process
  comparison including the claims that name a winner.
- The install lines name the version the crates are published at.
- Every comment and document in the workspace states what the code is, in
  US spelling, without emphasis capitals and without narrating what
  changed.

## [0.2.9] - 2026-09-05

### Added

- Counters for every failure the production paths used to discard:
  `send_failures()`, `demux_send_failures()` and `handshake_failures()` on
  the unified endpoint; `sens_rlc::socket_buffer_refusals()` for kernels
  that refuse the 4 MiB socket buffers; `results_dropped()` on the
  scheduler; `permit_release_overflows()` on the semaphore;
  `morph_refusals()` and `mode_refusals()` on the adaptive ring;
  `session_service_errors()` and `trace_sends_skipped()` on the RS bridge;
  `DgramSock::rearm_failures()`; `unattributed_frames()` and
  `malformed_frames()` on the RLC receiver, forwarded by the unified
  receiver as `rlc_unattributed_frames()` / `rlc_malformed_frames()`.

### Changed

- `ShardedUdpSender::send_item` returns `io::Result<()>` and reports
  `BrokenPipe` naming a shard whose thread has ended, where it used to
  drop the item. Callers handle the result.
- Every file-backed `create` reports a region that exists at a smaller
  size as its own `LayoutMismatch`, the answer `open` already gave for the
  same file (36 call sites across 35 types).
- The cached clock's readers take the clock directly when its updater
  thread could not be spawned, and the refusal is reported.
- Drain loops in the TCP and QUIC bridges, the locale migration, the
  capacity rings and the event log match `Empty` by name and return or
  record any other pop error; waker waits match their timeout by name and
  propagate the rest; the TCP bridge's closing barrier treats reset,
  abort, broken pipe and unexpected EOF as the receiver's close and
  returns any other read error.

### Fixed

- `mmf_attach::create_or_attach` refuses a non-empty file shorter than the
  requested size at once, with `SizeMismatch` naming the path and both
  sizes; it used to wait five seconds for a creator that would never
  finish. An empty file is still waited on.
- The RLC receiver handed any frame without a connection id to the most
  recently admitted session, which rebound its peer address to the
  frame's source without validation, so a one-byte datagram from anyone
  re-pointed that session's control traffic. Such frames are counted and
  reach no session. A `PATH_RESPONSE` answering a migration challenge is
  routed to the window whose id it carries, so an older peer's migration
  validates while a newer peer is live.
- The scheduler dropped a computed result on a full ring with no trace;
  the LRU cache could lose an entry between its list and its map on a
  refused re-insert; a graph edge or list node whose slot the region
  refused to free was leaked; the RS receiver discarded a session's
  service and feedback errors; an io_uring receive slot that failed to
  re-arm vanished; the net-events watcher could fail to spawn and read as
  a stable path. Each now reports or counts.
- `SharedAsyncPointer`'s speculative races no longer swallow a worker's
  panic: the plain variants continue it on the caller's thread, and the
  resilient variant counts the dead so `AllWorkersDied` is a measured
  statement.
- A listening receiver's RS index never trails its delivery frontier
  (`listen_tls` refuses `CodePolicy::Auto`); that invariant is a debug
  assertion at the branch that relies on it.

### Documentation

- Every new counter on its primitive's page; the listener's send-failure
  accounting; the RLC routing rule for frames that name no window.

## [0.2.8] - 2026-08-29

### Added

- `SharedHashMap::insert_if_absent` places a key only when it is absent
  and returns the value found otherwise; `compare_exchange` swaps a value
  only when the stored bytes match, under the slot's seqlock, with
  `KeyAbsent` for a key that has no entry.
- `SharedEpochs` tickets: `begin()` reserves one epoch for a compound
  write across every structure sharing the table, `now()` reports the
  highest epoch every lower ticket has published, and the reclaim horizon
  is bounded by the same value, so entries stamped from one ticket are
  seen by a scan all or none. A ticket whose process died is reported by
  `dead_tickets()`; each structure voids what it stamped there
  (`void_epoch`) and `free_dead_ticket()` releases the slot last.
  `VersionedBTreeMap::insert_at` / `remove_at` stamp a ticket's epoch.
- `SharedVersionedSlab<T, D>`: a `SharedSlab` of epoch-stamped version
  chains sharing an epoch table, so the records an index names stay
  readable at the version a scan pinned (`set_at`, `retire_at`, `get_at`,
  `void_epoch`). A push into a full chain drops only versions no pin can
  reach and refuses with `Pinned` when every version is still reachable.
- `LanedVersionedMap`: one versioned index across n single-writer lanes
  sharing one epoch table, so n statements write at once where one mutex
  used to serialize them. `claim_lane` takes a free lane, `claim_lane_for`
  the lane a key already lives in (`LaneBusy` otherwise), removing a key
  held by another lane reports `KeyInAnotherLane`, and an ordered read
  merges the lanes with a heap trusted up to the smallest lane cursor.
  `HolderTable::try_claim_slot` claims one named slot.
- Benches: a whole-store scan across writer lanes, four writers across
  four lanes against the same four behind a mutex (5.18x ahead), and the
  versioned slab's push and pinned read.

### Changed

- The `SharedEpochs` table layout carries a new magic for the ticket
  region; a table laid out by 0.2.7 or earlier is refused rather than read
  with the region missing.

### Fixed

- `SharedHashMap` published a claimed slot's hash before its payload,
  letting a prober plant the same key in a second slot; the payload lands
  first and hash 0 marks a slot still forming. A writer whose tombstone
  claim was stolen re-probes instead of claiming the empty slot beyond it,
  and concurrent updates to one key take the seqlock by CAS.

### Documentation

- Pages for the versioned slab and the laned map.

## [0.2.7] - 2026-08-29

### Added

- `UnifiedSensReceiver::listen_tls(local, cfg, tls, peers)`: a TLS
  listener that is up before any peer dials and serves any number of
  senders, each with its own keys. Handshakes run on the demux thread
  through a per-peer `HandshakeMachine`, and items open with per-peer
  packet numbers bound to the session tag. `handshake_refusals`,
  `handshake_failures`, `tls_preauth_dropped` and `tls_unopened` count
  what is turned away or cannot open. `CodePolicy::Auto` is refused for a
  listener, since the switch boundary is per endpoint.
- `connect_tls_named` asserts a chosen server name,
  `client_config_trusting` accepts a CA root or several leaves, and
  `self_signed_cert_for` issues a certificate for chosen names.

### Fixed

- A single-peer TLS receiver delivered one item and left its sender on a
  full window: opening the session moved the receiver's keys into it, so
  every later sealed frame reached routing still sealed. Receiver and
  session now share the keys.

## [0.2.6] - 2026-08-29

### Fixed

- Windows: an ICMP port-unreachable drawn by a departed peer surfaced as
  `WSAECONNRESET` on the receiver's next receive and displaced a datagram
  belonging to whichever peer was next; one peer delivered 112 of 150
  items under load. `SIO_UDP_CONNRESET` is disabled on every Sens-O-Matic
  socket (`dgram::quiet_icmp_connreset`, a no-op off Windows): 278 socket
  error episodes in one failing run became 0 across 40.
- Tests across the suite wait for the observable event rather than a
  duration, and senders that stop early report what they sent, so a
  loaded host no longer reads as the transport losing data.

### Documentation

- Every lock page states what a holder that dies does to a waiter:
  `SharedRWLock` spins without bound and only `reset` recovers it, the
  blocking primitives point at their bounded variants, and all point at
  `OwnerLease` for a resource whose holder might die.

## [0.2.5] - 2026-08-28

### Added

- `VersionedBTreeMap`: a `SharedBTreeMap` of `Versioned<V>` beside a
  `SharedEpochs`. A delete stamps a death epoch instead of removing the
  entry, a scan pins an epoch and sees what was current then, and an
  entry goes once no pin can reach it. Reinserting a key whose tombstone a
  live pin can still reach is refused as `RebornUnderPin`.
- `HolderTable`: a fixed array of claimable slots stamped with the holding
  process, over memory the caller has mapped; the peer directory's
  consumer slots, the epoch table's pins and a `SharedArc`'s holders are
  the same structure. `SharedEpochs` holds one, byte-for-byte compatible
  with 0.2.4.
- `SharedArc`: a value in shared memory kept alive by the processes
  holding it, bounded by `ShmValue` rather than `Copy` so atomics qualify;
  `LastHolder` decides whether the last release unlinks the backing.
- Benches for all three against the alternatives a caller would reach
  for, with the numbers in the docs including where each primitive loses.

## [0.2.4] - 2026-08-28

### Added

- `ShmFile` takes the creating process's SDDL security descriptor
  (`AdaptiveRing::create_shmfs_secured` / `open_shmfs_secured`), carried
  on `BackingId::Shm` so the peer directory, ordering region and payload
  region take the same one. An unparseable descriptor fails the create;
  the crate supplies no default.
- `SharedSlab`: fixed capacity, a caller-chosen index, and records larger
  than a cache line under the per-slot seqlock. Re-exported from the crate
  root.
- `SharedEpochs`: the counter a writer stamps a superseded version with
  and the pin table a scan holds against reclamation, both in the mapping
  so a reclaimer in another process sees the pins; `reap_dead_pins` frees
  a crashed scanner's slot.
- `SUBETHA_RING_DEBUG` reports every shape transition, the MPMC ownership
  decisions and the table a consumer scanned when it found nothing to
  pop; `ownership_snapshot` exposes ring ownership.

### Fixed

- A single-reader ring shape is served to consumer 0 alone; a second
  consumer registering while the ring was still Mpsc drained the same core
  and took the same items twice. A morph blocked by a stale backlog is held
  as pending and applied by the pop that clears the backlog.
- A pinner reserves its slot before reading the epoch, so a reclaimer's
  horizon cannot pass a pin that appears an instant later.

## [0.2.3] - 2026-08-28

### Added

- `SharedBTreeMap::range`: a bounded ordered query resumed by key, which
  bounds the seqlock retry behind it.

### Documentation

- The `subetha-cxc` crate root states the producer and consumer counts
  each queue primitive supports; `SharedDeque` with two producers and a
  merge-ordered ring consumer without a drainer lease both deadlock by
  contract.

## [0.2.2] - 2026-08-25

### Fixed

- `BackingId::Shm` carries the shm namespace to every region an
  `AdaptiveRing` names (backings, peer directory, per-producer rings,
  ordering region, payload region), so a ring shared between Windows
  sessions reaches all of them rather than each side succeeding against a
  separate copy.

## [0.2.1] - 2026-08-25

### Added

- `ShmNamespace::Machine` names one shared-memory region for every session
  on a Windows host, for a service in session 0 and its interactive
  clients. A create without `SeCreateGlobalPrivilege` fails with the OS
  error rather than falling back.

### Fixed

- Ring takeover fires only on a released claim or a claimed slot whose
  process is gone: a released consumer slot looked like one never claimed,
  and the takeover probe stole rings from a live consumer draining through
  `try_claim_ring`.

## [0.2.0] - 2026-08-23

Breaking: `StringRef` packs its offset in 40 bits and its length in 24
(1 TiB arenas, 16 MiB strings), and an arena region is tagged with the
layout its refs are packed under, so a region written under the previous
layout is refused. A `^0.1` requirement does not resolve to this line.

### Added

- Block-RS telemetry matching the RLC side: per-gate ingest refusal
  counts, armed admission challenges and their age, the block-id ranges
  refused and retransmitted, the last DATA read off the wire, a
  transmit-side probe, the liveness probe's own resend path, the inbound
  demux queue depth and recovery backlog, and counts for feedback,
  challenge answers and code-switch announcements no send path could
  deliver. `EPOCH_OFFSET` is public.
- Both demux readers count datagrams no routing arm claims.
- The RLC receiver counts a session that could not be serviced.
- A bench of the decoder ingest path.

### Changed

- The multi-peer RS receive path drains everything that has arrived per
  poll instead of one datagram, matching the single-peer burst reads; a
  second session no longer slows the receiver below a peer's send rate.

### Fixed

- `SharedVec::push_back` published a slot's index before its payload, so a
  reader could take the zeroed region as a value; a reservation allocator
  publishes `len` only after every earlier reservation has landed.
- Block-RS: a control frame carries no epoch, and the session inferred as
  its owner rebound its peer to the sender, so a completed window acked a
  restarted sender's blocks and delivery stopped one block short for good.
  Only a datagram that names a session may move where that session sends.
- The recovery queue no longer resends datagrams for blocks the peer has
  acked, and a NAK for a block the sender no longer holds is counted and
  named instead of discarded.

## [0.1.12] - 2026-08-22

### Changed

- `SharedStringArena` reports a bad capacity as `LayoutMismatch` from
  `create`, `open` and `reset` alike instead of asserting in two of them.
- `FrameRegion::create` and `SharedAtomicBool::create` obtain an existing
  region rather than truncating it, completing the sweep of every
  file-backed `create` outside the owner-exclusive set.

## [0.1.11] - 2026-08-22

### Added

- `UnifiedSensSender::finish_within(deadline)`; `finish()` keeps its
  default. `demux_stale_for` and `demux_errors` report a reader thread
  that is alive but deaf.

### Changed

- Attach-on-create for `FrameRing`, `SharedUniversal`, `SpscRingCore` and
  the constructors built on it, `DirectFileRing`, `PubSubRing`,
  `SharedVersionedChain` and the hugepage region; `DirectFileRing::open`
  requires the exact file size.

### Fixed

- Linux did not compile from 0.1.6 through 0.1.10: sharing the receiver
  socket behind an `Arc` required `DgramSock: Sync`, which the io_uring
  and wire backends denied. Both hold their state under a mutex.
- `TaskPool::shutdown` set its flag outside the queue lock, so a worker
  between its check and its park missed the notify and `join` never
  returned.
- The unified endpoint fed back to the last speaker only, starving every
  other sender's loss estimate; every peer heard from within the last 30 s
  is fed.
- `net_events` reads the egress MTU on FreeBSD and macOS, so an MTU drop
  registers as a path event there.
- `SharedStringArena::intern_bytes` refuses an offset past `u32::MAX`
  instead of narrowing it into another string's bytes.
- Rustdoc builds with `-Dwarnings` on every platform.

## [0.1.10] - 2026-08-22

### Added

- Telemetry for placing a stall: `tx_probe` / `rlc_tx_probe` (packed, on
  the wire, acked), `route_probe` (active code, RS backlog, items
  accepted, `send_item` entries), `ctrl_probe` / `rlc_ctrl_probe` and
  `session_control` / `rlc_session_control` on both ends of the ARQ
  backchannel, `demux_alive`, `demux_probe`, `queue_seam_probe`,
  `pump_types`, `rlc_path_validations` / `rlc_path_validation_failures`,
  `UnifiedSensSender::local_addr` and `rlc_session_peer`.
  `SUBETHA_SEND_TRACE` and `SUBETHA_WAKE_TRACE` print ledgers on stderr.

### Changed

- Twenty-four more primitives obtain an existing region on `create` and
  truncate only on `reset`: hash map, leader election, once cell, atomics,
  cell, fence clock, handle table, rate limiter, time point, histogram,
  bit vector, Bloom and blocked Bloom filters, count-min sketch,
  HyperLogLog, reservoir sampler, broadcast ring, Treiber stack, vec,
  string arena, topology map, B-tree map and region. The deque family,
  `SharedRing` and the ordering types keep truncate-on-create by design.

### Fixed

- Mesh starvation, three causes: an admission challenge resends at most
  once per 50 ms per candidate, the unified demux thread answers
  `PATH_CHALLENGE` at wire latency, and an ACK goes out when the delivery
  frontier advances and at most once per 10 ms otherwise. Verified 4/4 on
  a four-node quorum gate.
- The reactor parked a future whose ring was already non-empty when items
  were published between its registration and the loop's first read; a
  parked future with a non-empty ring is fired regardless of head history.
  Verified 80/80 under CPU-starved load.

## [0.1.9] - 2026-08-22

### Added

- `session_frontier(cid)` / `rlc_session_frontier(cid)`: one window's
  `(delivered_through, highest_seen)`.

### Changed

- `SharedRWLock::create` attaches to an existing lock and `reset` is the
  explicit truncation, so racing creators cannot clear a writer flag a
  live holder owns. The same contract for `CrossProcessWaker`,
  `OwnerLease`, `HeartbeatTable`, `SharedCondvar`'s generation region and
  `BlockingRWLock`'s wakeup region.

### Fixed

- One session's send error no longer aborts servicing the sessions after
  it or returns the tick's already-decoded items inside an `Err`.

## [0.1.8] - 2026-08-22

### Added

- `SharedRWLock::create_or_open` elects one creator and has the others
  attach; `mmf_attach::create_or_attach` is the shared election, applied
  to the peer directory.
- `live_rlc_sessions`, `live_rs_sessions` and `session_refusals` on the
  unified receiver.

### Fixed

- A latched `ConnectionReset` (Windows, ICMP from a dead peer) ended the
  whole RLC receive drain, so peers queued behind it were never read; the
  drain reads past it, bounded by `MAX_DRAIN_RESETS`.
- The RLC connection id was drawn from the wall clock and repeated (484
  distinct values in 1000 draws); it mixes the invariant TSC, the pid and
  the port.

## [0.1.7] - 2026-08-22

### Fixed

- A unified receiver's block-RS half served one peer, because
  `with_multi_peer` was unreachable through the unified constructor; a
  demux socket routes by session epoch on its own.

## [0.1.6] - 2026-08-22

Breaking on the wire: the block-RS DATA header grows from 9 to 13 bytes to
carry a session epoch, so 0.1.6 RS endpoints do not interoperate with
earlier ones.

### Added

- A decode window per peer on both codes: RLC sessions keyed by connection
  id and RS sessions keyed by epoch, with `poll_from()` returning
  `(peer, item)` on the RLC, RS and unified receivers. `with_multi_peer()`
  on the RS receiver; the connected single-peer fast path stays the
  default.
- New connection ids are challenged before a window opens; the first id a
  receiver sees is admitted outright, and a TLS receiver refuses a second.
- `with_session_ceiling(max)` bounds live windows and pending challenges,
  and `session_refusals()` counts peers turned away. Unbounded by default.

### Changed

- `AutoIpc::capacity(n)` rounds to a power of two as its docs said;
  `capacity(100).build_channel()` used to panic.

### Fixed

- Block-RS survives a peer restart: every DATA datagram carries the
  session epoch, adoption is gated on a nonce challenge, feedback is
  scoped to its session, the heartbeat announces the epoch, and the
  shared-socket receive path keeps the source address.
- Feedback falls back to an addressed send when a connected socket
  carries a latched `ConnectionReset`.
- `SharedAsyncPointer::get_or_lazy` waits for a publisher mid-initialization
  instead of panicking on the read.

## [0.1.5] - 2026-08-21

### Added

- `take_session_changed()` and `session_adoption_counts()` on the RLC and
  unified receivers.
- The `subetha-e2e` driver binary (workspace member, not published):
  scenarios that cross a real process boundary, including failover of a
  killed child, the ring boundary, flush visibility across processes, a
  scheduler round trip, session restart, a forged session id refused, and
  receiver restart.
- The `iceoryx-bench` feature gates the iceoryx2 bench contender so
  `cargo test` compiles without libclang.

### Fixed

- A restarted RLC peer draws a fresh connection id and was discarded for
  good; the receiver challenges the address with a nonce and adopts the
  session when it returns. The path-validation frames are routed by name
  on the shared socket, which also revives address migration under the
  unified endpoint, inert since that endpoint landed.
- A receiver joining a stream in progress anchors its frontier on the
  first source id observed instead of waiting at zero.

### Removed

- The in-process integration tests the e2e gate subsumes.

## [0.1.4] - 2026-08-02

### Added

- `SharedVec::open_read_only` and `SharedStringArena::open_read_only` map
  without write access; writes return `ReadOnly`.
- `SharedVec::for_each` / `for_each_range` walk the vec without allocating
  a snapshot.

### Documentation

- The `with_frames` path for offset frames above 8 KB, proven across a
  process boundary with matching arguments on both sides.

## [0.1.3] - 2026-07-23

### Fixed

- Offset-class frames (payloads above the inline budget) did not cross a
  process boundary on the file or shm locale, because the payload region
  was a process-private mapping. It rides the ring's own locale
  (`<prefix>.frames.bin` or `<prefix>_frames`) with a CAS-guarded init,
  created lazily on the first offset frame.

## [0.1.2] - 2026-07-23

### Added

- `AdaptiveRing::open_shmfs` attaches to a populated shm region without
  re-laying it out; `create_shmfs` re-initialized the region and wiped
  another process's snapshot.

### Documentation

- The unified Sens-O-Matic endpoint (auto-switching RLC and RS with TLS on
  both codes) positioned as the untrusted-WAN default; the polymorphic
  substrate documented at five axes; README prose passes.

## [0.1.1] - 2026-07-06

### Fixed

- Crate metadata and README URLs point at the renamed `SubEtha` repository
  and Pages site; the published 0.1.0 froze the pre-rename paths. No code
  change.

## [0.1.0] - 2026-07-06

Initial release: MMF-backed cross-process IPC for Rust, one byte layout
serving cross-thread, cross-process, disk-persistent and cross-host
deployment.

- `subetha-cxc`: `Channel<T>`, `AdaptiveIpc<T>`, `AutoIpc`, the MMF
  dispatcher and about forty MMF-backed primitives across the Locale x
  Protocol x Shape x Capacity x Ordering axes (`AdaptiveRing`,
  `OrderingRegion`, the capacity and locale adaptive rings, `PubSubRing`,
  `VirtualEndpoint`, `QosPolicy`, `RingContract`), the Sens-O-Matic
  reliable-UDP transports (block Reed-Solomon and sliding-window RLC, FEC
  plus ARQ, TLS on the RLC code) with the unified auto-switching endpoint,
  the QUIC and TCP bridges and the raw-L2 wire socket behind features,
  and the OS-specific rings (`DirectFileRing`, fd handoff,
  `KernelAsyncRing`, hugepage and superpage regions, vsock).
- `subetha-core`: handshake header, observation ring, marshal trait,
  axis-signature catalog, CPUID helpers.
- `subetha-sidecar`: per-NUMA scan thread, policy, `SidecarBox`,
  `AdaptiveInstance`.
- `subetha-pointers`: Umbra, Bloom, KStep, KTower, SelfDesc, Versioned +
  HLC, Cardinality, CHERI capability and RaspBatch pointer types.
- `subetha`: the umbrella crate re-exporting the four.
- The Hugo wiki and the measured six-platform performance record.

[Unreleased]: https://github.com/Variably-Constant/SubEtha/compare/0.6.0...HEAD
[0.6.0]: https://github.com/Variably-Constant/SubEtha/compare/0.5.1...0.6.0
[0.5.1]: https://github.com/Variably-Constant/SubEtha/compare/0.5.0...0.5.1
[0.5.0]: https://github.com/Variably-Constant/SubEtha/compare/0.4.1...0.5.0
[0.4.1]: https://github.com/Variably-Constant/SubEtha/compare/0.4.0...0.4.1
[0.4.0]: https://github.com/Variably-Constant/SubEtha/commit/0.4.0
[0.3.3]: https://github.com/Variably-Constant/SubEtha/commit/0.3.3
[0.3.2]: https://github.com/Variably-Constant/SubEtha/commit/0.3.2
[0.3.1]: https://github.com/Variably-Constant/SubEtha/commit/0.3.1
[0.3.0]: https://github.com/Variably-Constant/SubEtha/commit/0.3.0
[0.2.9]: https://github.com/Variably-Constant/SubEtha/commit/38a10f3
[0.2.8]: https://github.com/Variably-Constant/SubEtha/commit/6ca3d22
[0.2.7]: https://github.com/Variably-Constant/SubEtha/commit/aec1f18
[0.2.6]: https://github.com/Variably-Constant/SubEtha/commit/597c23a
[0.2.5]: https://github.com/Variably-Constant/SubEtha/commit/7d1fc25
[0.2.4]: https://github.com/Variably-Constant/SubEtha/commit/e73a185
[0.2.3]: https://github.com/Variably-Constant/SubEtha/commit/0cd7018
[0.2.2]: https://github.com/Variably-Constant/SubEtha/commit/c301474
[0.2.1]: https://github.com/Variably-Constant/SubEtha/commit/a8e96f0
[0.2.0]: https://github.com/Variably-Constant/SubEtha/commit/7a41964
[0.1.12]: https://github.com/Variably-Constant/SubEtha/commit/6827089
[0.1.11]: https://github.com/Variably-Constant/SubEtha/commit/f9fc6e3
[0.1.10]: https://github.com/Variably-Constant/SubEtha/commit/b08e3f8
[0.1.9]: https://github.com/Variably-Constant/SubEtha/commit/234f370
[0.1.8]: https://github.com/Variably-Constant/SubEtha/commit/536a9dc
[0.1.7]: https://github.com/Variably-Constant/SubEtha/commit/16ca524
[0.1.6]: https://github.com/Variably-Constant/SubEtha/commit/64d4c51
[0.1.5]: https://github.com/Variably-Constant/SubEtha/commit/04f5477
[0.1.4]: https://github.com/Variably-Constant/SubEtha/commit/7d6a890
[0.1.3]: https://github.com/Variably-Constant/SubEtha/commit/a7bc859
[0.1.2]: https://github.com/Variably-Constant/SubEtha/commit/de5f6d3
[0.1.1]: https://github.com/Variably-Constant/SubEtha/commit/0a5e48e
[0.1.0]: https://github.com/Variably-Constant/SubEtha/commit/9a91f03
