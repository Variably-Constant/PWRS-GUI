---
title: "Shared Versioned Index"
weight: 27
---

# SharedVersionedIndex&lt;K, V&gt;

An ordered map in shared memory that any number of threads and processes
write at once, keeping every version of a key that a pinned reader can
still need. A key holds its versions newest first behind one atomic
word. A reader shows a pin from the index's
[Shared Epochs](../../coordination-types/shared-epochs/) table and sees,
for each key, the newest version born at or before the pin, or one its
own statement wrote. Nothing in it takes a lock: one compare-and-swap
decides each write, and a writer that loses is refused rather than made
to wait.

## Files

`create(path, epochs_path, max_pins)` obtains the index at `path` and
its epoch table at `epochs_path`, making either that is absent and
attaching to either that is present; `open` attaches to both. The main
file holds the header, the head of the skip list and one write list per
ticket slot. Keys, versions and write-list entries live in chunk files
beside it, named after it (`<path>.keys0`, `<path>.tall0`,
`<path>.versions0`, `<path>.writes0` and on), each made the first time
the index needs it, so the index grows while other processes read it.
`create_with_chunks` sets the slots one chunk holds, 2^4 to 2^20, and
2^16 by default; an index is opened at the chunk size it was made with.
The structures of one store name one epoch table, so one ticket stamps
them all.

## Writes

- `write(key, value, &ticket, &pin)` makes `value` current for `key` in
  the statement holding `ticket`, whose snapshot is `pin`; `None`
  deletes. It builds only on the version the pin sees: a key whose
  newest version was born after the pin by another epoch, published or
  not, refuses it with `IndexError::Conflict { epoch }`, and the
  statement takes back what it wrote and runs again. Of writers racing
  on one key exactly one wins, and of writers racing to create a key
  exactly one version lands.
- A second write under the same ticket replaces its own version.
  Deleting a version the ticket wrote leaves a deletion when that
  version superseded a value, and otherwise puts the key back as it was
  before the ticket wrote it.
- `void(&key, epoch)` takes back `epoch`'s version of `key`, and the
  version it superseded is current again. An epoch's version is the
  newest on its key until the epoch publishes, since every pin sits
  below an open ticket. Once a later version supersedes it, it is
  history a reader may already have seen, and `void` answers `false`.

## Reads

- `get_at(&key, &pin, own)` answers the value current at the pin, or the
  one the ticket `own` wrote.
- `range_at(low, high, limit, &pin, own)` answers the rows in key order
  and, when `limit` versions examined stopped the walk, the last key
  examined, to resume from with `Bound::Excluded`; `None` means the
  range is exhausted. A key's versions are never split across two calls,
  every call examines at least one key, and bounds that cross read as no
  rows.
- `changed_since(low, high, since, own, &pin)` says whether any key in
  the range gained a version after `since` from an epoch other than
  `own`'s, a deletion included: what a statement validates for each
  range it scanned before it publishes.
- `version_count()` counts the versions held, deletions included, and
  `live_count()` the keys whose newest version holds a value.

## Reclamation

`sweep()` drops every version superseded at or below the epoch table's
reclaim horizon, removes every key whose newest version is a deletion at
or below it, and answers how many versions holding a value went. What it
unlinks is retired, not freed: a slot is handed out again only once the
horizon has passed the epoch it was retired at, when no pin that could
still be reading it is left. Every read and write takes a pin from the
index's own epoch table for that reason. A pin from a table in another
file is not counted in the horizon and does not keep what it reads in
place.

## A writer that dies

Before a write installs a version it records the key on the list kept
for its ticket's slot. `void_epoch(epoch)` walks that list, so the epoch
of a ticket whose process died, which `dead_tickets` names, is taken
back key by key, at the cost of what that epoch wrote, before
`free_dead_ticket` releases the slot. Until then every pin sits below the
dead ticket, so no reader sees what it wrote. A writer that dies at any
point leaves the list readable: a key's first level is linked by one
compare-and-swap, and the levels above it are shortcuts.

## Errors

| Error | When |
|---|---|
| `Conflict { epoch }` | the key's newest version was born after the pin by another epoch |
| `Full` | a pool has no free slot after reclaiming what the horizon allows |
| `LayoutMismatch(Mismatch)` | the files hold an index of another key or value layout, chunk size or ticket count |
| `InvalidConfig(&str)` | a chunk size outside 2^4 to 2^20, or a ticket from another epoch table |
| `Epochs(EpochError)` | the epoch table's own error |
| `IoError(io::Error)` | the operating system refused to open, size or map a file |

## Keys and values

`K` and `V` are stored as their bytes, so they must be plain data that
holds no pointer or reference, and every process attached to one index
must agree on their types and on `K`'s ordering.

## Verified

The module's unit tests, over chunks of 64 slots so the index crosses
chunk files: a write is seen by pins at or after its epoch; earlier pins
keep their version across an update, a delete and a reinsert; a
statement reads its own writes to a key and in a range, its deletion
included, while another reader at its pin does not; a version or a
deletion born after the pin is a conflict naming its epoch; a second
write under one ticket replaces its own version; `void` takes a version
back and revives what it superseded; eight threads racing on a live key
admit exactly one, and eight racing to create a key land exactly one
version in each of 200 rounds; a range resumed three versions at a time
covers every key; bounds that cross read as no rows; `changed_since`
reports another epoch's change and not the statement's own; a sweep
drops only what no held pin can see; a deleted key is removed by a sweep
and can be written again; `void_epoch` takes back everything an epoch
wrote; a second attach reads the first one's writes and conflicts with
them; 300 keys rewritten and swept six times keep every live key; three
writers, two readers and a sweeper running at once never show a reader
another key's value, a value past its pin, or two views from one pin; a
version retired while a pin that could step over it is held is not
handed out again until that pin is gone, a test that fails when the pool
ignores the horizon. Across processes, four racer processes on one live
key admit exactly one writer in each of 20 rounds, four racing to create
keys land exactly one version in each of 50 rounds, and a writer process
that exits without publishing has its epoch named by `dead_tickets`,
seen by no reader, and taken back whole by `void_epoch`.
