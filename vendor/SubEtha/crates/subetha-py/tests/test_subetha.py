"""The Python surface, checked from Python.

The Rust tests cannot reach any of this: a pyclass needs an interpreter to
be a class at all, so what the binding actually presents to Python is only
observable from here.

The test that matters most is the zero-copy one. A `memoryview` over a
region is supposed to be the mapping rather than a copy of it, and the
only way to tell those apart is to change the bytes through Rust and look
for the change through the view.
"""

import gc
import os
import subprocess
import sys
import threading
import time
import uuid

import pytest

import subetha


@pytest.fixture
def scratch(tmp_path):
    return lambda name: os.fspath(tmp_path / name)


def test_an_atomic_holds_its_value(scratch):
    atom = subetha.Atomic(scratch("counter"), init=7)
    assert atom.load() == 7
    atom.store(9)
    assert atom.load() == 9
    assert atom.fetch_add(3) == 9
    assert atom.load() == 12


def test_a_second_handle_sees_the_same_file(scratch):
    path = scratch("shared")
    first = subetha.Atomic(path, init=1)
    first.fetch_add(41)
    second = subetha.Atomic.open(path)
    assert second.load() == 42, "the two handles are the same bytes on disk"


def test_a_batch_is_the_same_as_the_calls_it_replaces(scratch):
    one_at_a_time = subetha.Atomic(scratch("a"), init=0)
    for _ in range(100):
        one_at_a_time.fetch_add(2)

    batched = subetha.Atomic(scratch("b"), init=0)
    batched.fetch_add_many(100, 2)

    assert batched.load() == one_at_a_time.load() == 200


def test_an_unknown_ordering_is_refused(scratch):
    atom = subetha.Atomic(scratch("counter"))
    with pytest.raises(ValueError) as caught:
        atom.load("sideways")
    assert "sideways" in str(caught.value)


@pytest.mark.parametrize("order", ["relaxed", "acquire", "seq_cst"])
def test_the_orderings_that_a_load_accepts(scratch, order):
    atom = subetha.Atomic(scratch(f"counter-{order}"), init=5)
    assert atom.load(order) == 5


def test_an_atomic_is_a_context_manager(scratch):
    with subetha.Atomic(scratch("counter"), init=3) as atom:
        assert atom.load() == 3


def test_a_region_round_trips_a_slot(scratch):
    region = subetha.Region(scratch("region"), capacity=8, slot_size=16)
    assert region.capacity == 8
    index = region.allocate(b"x" * 16)
    assert region.get(index) == b"x" * 16
    region.set(index, b"y" * 16)
    assert region.get(index) == b"y" * 16


def test_a_memoryview_is_the_mapping_and_not_a_copy(scratch):
    region = subetha.Region(scratch("region"), capacity=4, slot_size=8)
    index = region.allocate(b"\x00" * 8)
    view = memoryview(region)

    # Written through Rust, read through the view. A copy taken at
    # memoryview() time would still show the old bytes here.
    region.set(index, b"\xab" * 8)
    assert b"\xab" * 8 in bytes(view), "the view did not see a write made after it was taken"

    del view


def test_a_memoryview_covers_every_slot(scratch):
    slots, slot_size = 4, 8
    region = subetha.Region(scratch("region"), capacity=slots, slot_size=slot_size)
    view = memoryview(region)
    assert len(view) >= slots * slot_size, "the view is shorter than the slots it covers"
    del view


def test_a_region_is_a_context_manager(scratch):
    with subetha.Region(scratch("region"), capacity=2, slot_size=4) as region:
        assert region.capacity == 2


# The rings. Full and empty are answers here rather than failures: a push
# that does not fit returns False and a pop with nothing to take returns
# None, so ordinary flow control does not go through exceptions.


def test_a_ring_round_trips_in_order(scratch):
    ring = subetha.SpscRing(scratch("ring"), capacity=8)
    assert ring.push(b"first") is True
    assert ring.push(b"second") is True
    assert ring.pop().startswith(b"first")
    assert ring.pop().startswith(b"second")


def test_an_empty_ring_pops_none(scratch):
    ring = subetha.SpscRing(scratch("ring"), capacity=4)
    assert ring.pop() is None


def test_a_full_ring_refuses_rather_than_raising(scratch):
    ring = subetha.SpscRing(scratch("ring"), capacity=2)
    accepted = [ring.push(bytes([i])) for i in range(4)]
    assert accepted[0] is True
    assert False in accepted, "a ring of two slots took four items"


def test_an_item_longer_than_a_slot_is_refused(scratch):
    ring = subetha.SpscRing(scratch("ring"), capacity=4)
    with pytest.raises(ValueError):
        ring.push(b"x" * (ring.payload_size + 1))


def test_push_many_reports_what_it_took(scratch):
    ring = subetha.SpscRing(scratch("ring"), capacity=4)
    took = ring.push_many([bytes([i]) for i in range(10)])
    assert took == 4, "it should stop at the first refusal and say so"
    assert len(ring.pop_many(100)) == 4


def test_a_batch_carries_the_same_items_as_the_calls_it_replaces(scratch):
    one = subetha.SpscRing(scratch("a"), capacity=64)
    many = subetha.SpscRing(scratch("b"), capacity=64)
    items = [bytes([i]) * 4 for i in range(20)]

    for item in items:
        one.push(item)
    many.push_many(items)

    assert one.pop_many(100) == many.pop_many(100)


def test_push_buffer_takes_items_packed_end_to_end(scratch):
    ring = subetha.SpscRing(scratch("ring"), capacity=16)
    packed = b"".join(bytes([i]) * 8 for i in range(5))
    assert ring.push_buffer(packed, 8) == 5
    out = ring.pop_many(5)
    assert len(out) == 5
    assert out[0].startswith(bytes([0]) * 8)
    assert out[4].startswith(bytes([4]) * 8)


def test_push_buffer_refuses_a_zero_item_length(scratch):
    ring = subetha.SpscRing(scratch("ring"), capacity=4)
    with pytest.raises(ValueError):
        ring.push_buffer(b"xxxx", 0)


def test_pop_buffer_returns_the_items_and_the_count(scratch):
    ring = subetha.SpscRing(scratch("ring"), capacity=8)
    ring.push_many([bytes([i]) * 4 for i in range(3)])
    packed, taken = ring.pop_buffer(8)
    assert taken == 3
    assert len(packed) == taken * ring.payload_size


def test_a_second_handle_on_a_ring_sees_the_same_items(scratch):
    path = scratch("ring")
    writer = subetha.SpscRing(path, capacity=8)
    writer.push(b"across")
    reader = subetha.SpscRing.open(path, capacity=8)
    assert reader.pop().startswith(b"across")


def test_a_broadcast_ring_gives_every_consumer_every_item(scratch):
    ring = subetha.BroadcastRing(scratch("bcast"), capacity=8)
    first = ring.register_consumer()
    second = ring.register_consumer()
    assert ring.push(b"to everyone") is True

    assert ring.recv(first).startswith(b"to everyone")
    assert ring.recv(second).startswith(b"to everyone"), "the second consumer missed it"
    assert ring.recv(first) is None, "an item should be delivered once per consumer"


def test_a_broadcast_ring_keeps_its_wakers_beside_its_file(scratch):
    # A waiter in any process, a C one included, parks on these files, and
    # a push or receive from here wakes it through them.
    path = scratch("bcast-wakers")
    ring = subetha.BroadcastRing(path, capacity=8)
    assert os.path.exists(path + ".cwaker.bin")
    assert os.path.exists(path + ".pwaker.bin")
    assert ring.capacity == 8


def test_a_broadcast_consumer_lags_and_catches_up(scratch):
    ring = subetha.BroadcastRing(scratch("bcast"), capacity=8)
    consumer = ring.register_consumer()
    ring.push_many([bytes([i]) for i in range(3)])
    assert ring.lag(consumer) == 3
    assert len(ring.recv_many(consumer, 10)) == 3
    assert ring.lag(consumer) == 0


def test_broadcast_lag_says_none_when_the_id_names_no_consumer(scratch):
    """Zero means caught up, and it must mean only that.

    An id nobody holds and an id past the table are both "no such
    consumer", and answering either with a number puts them in the same
    bucket as a reader that is up to date.
    """
    ring = subetha.BroadcastRing(scratch("bcast"), capacity=8)
    consumer = ring.register_consumer()
    ring.push_many([bytes([i]) for i in range(3)])
    assert ring.lag(consumer) == 3

    ring.unregister_consumer(consumer)
    assert ring.lag(consumer) is None

    # Whatever the producer does next, an id nobody holds stays None
    # rather than drifting further behind.
    ring.push(b"after")
    assert ring.lag(consumer) is None

    assert ring.lag(10_000) is None


def test_broadcast_lag_is_zero_for_a_consumer_that_missed_everything(scratch):
    """The case the documentation warns about, pinned as behavior.

    A consumer registers at the head, so one registered after a burst is
    caught up by this measure and saw none of it. `producer_position` at
    the moment of registration is what tells a caller how much it will
    never see.
    """
    ring = subetha.BroadcastRing(scratch("bcast"), capacity=8)
    ring.push_many([bytes([i]) for i in range(3)])

    missed = ring.producer_position
    late = ring.register_consumer()

    assert missed == 3, "three items were published before anyone registered"
    assert ring.lag(late) == 0, "caught up with the head, having read nothing"
    assert ring.recv(late) is None, "and there is nothing there to read"


def test_a_broadcast_producer_can_wait_for_its_readers(scratch):
    """The barrier answers with what arrived rather than hanging.

    This is what a producer does about the loss `lag` cannot report:
    publish only once the readers are registered, because a consumer
    starts at the head and everything before it is gone.
    """
    ring = subetha.BroadcastRing(scratch("bcast"), capacity=8)

    began = time.monotonic()
    assert ring.wait_for_consumers(1, timeout=0.05) == 0, "nobody registered"
    assert time.monotonic() - began >= 0.04, "it must wait, not give up at once"

    ring.register_consumer()
    assert ring.wait_for_consumers(1, timeout=0.05) == 1

    with pytest.raises(ValueError):
        ring.wait_for_consumers(1, timeout=0)


def test_a_late_reader_releases_the_broadcast_barrier(scratch):
    """And having waited, the producer's first item is not lost."""
    ring = subetha.BroadcastRing(scratch("bcast"), capacity=8)
    taken = []

    def join_late():
        time.sleep(0.02)
        taken.append(ring.register_consumer())

    reader = threading.Thread(target=join_late)
    reader.start()
    assert ring.wait_for_consumers(1, timeout=5.0) == 1
    reader.join()

    assert ring.push(b"first") is True
    assert ring.recv(taken[0]).startswith(b"first")


def test_a_broadcast_ring_counts_its_consumers(scratch):
    ring = subetha.BroadcastRing(scratch("bcast"), capacity=4)
    consumer = ring.register_consumer()
    assert ring.active_consumers == 1
    ring.unregister_consumer(consumer)
    assert ring.active_consumers == 0


# The Vyukov ring: any number of producers and consumers, each item taken
# by exactly one of them. A waiter parks on the wakers beside the ring, so
# a push or pop through any handle wakes it.


def vyukov_shm_name():
    """A shared-memory name no other test asks for, short enough for the
    platforms that limit one."""
    return f"subetha_vy_{os.getpid()}_{uuid.uuid4().hex[:8]}"


def wait_on_a_thread(call):
    """Start `call` on a thread once it is about to wait, and hand back the
    thread and the list its answer lands in."""
    ready = threading.Event()
    answer = []

    def run():
        ready.set()
        try:
            answer.append(call())
        except OSError as e:
            answer.append(e)

    thread = threading.Thread(target=run)
    thread.start()
    ready.wait()
    time.sleep(0.2)
    return thread, answer


def test_a_vyukov_ring_hands_items_back_in_order_padded_to_a_slot(scratch):
    ring = subetha.VyukovRing(scratch("vyukov"), capacity=8)
    assert ring.payload_size == 56
    assert ring.push_many([b"one", b"two", b"three"]) == 3
    assert ring.approx_len == 3
    assert ring.pop() == b"one".ljust(ring.payload_size, b"\0")
    assert [item.rstrip(b"\0") for item in ring.pop_many(10)] == [b"two", b"three"]
    assert ring.pop() is None


def test_a_full_vyukov_ring_refuses_rather_than_raising():
    ring = subetha.VyukovRing.anon(2)
    assert ring.push(b"a") is True
    assert ring.push(b"b") is True
    assert ring.push(b"c") is False
    assert ring.push_many([b"c", b"d"]) == 0


def test_a_vyukov_ring_refuses_what_it_cannot_hold(scratch):
    ring = subetha.VyukovRing.anon(4)
    with pytest.raises(ValueError):
        ring.push(bytes(ring.payload_size + 1))
    for capacity in (0, 1, 3, 6):
        with pytest.raises(ValueError):
            subetha.VyukovRing.anon(capacity)
    with pytest.raises(ValueError):
        subetha.VyukovRing(scratch("odd"), capacity=3)
    with pytest.raises(ValueError):
        ring.pop_for(timeout=-1)


def test_a_vyukov_ring_attaches_to_the_ring_it_finds(scratch):
    path = scratch("vyukov")
    first = subetha.VyukovRing(path, capacity=8)
    assert os.path.exists(path + ".cwaker.bin")
    assert os.path.exists(path + ".pwaker.bin")
    first.push(b"kept")
    second = subetha.VyukovRing(path, capacity=8)
    assert second.pop().startswith(b"kept"), "a second constructor must attach, not start over"
    assert subetha.VyukovRing.open(path, capacity=8).capacity == 8
    with pytest.raises(subetha.LayoutMismatch):
        subetha.VyukovRing.open(path, capacity=16)
    with pytest.raises(subetha.LayoutMismatch):
        subetha.VyukovRing(path, capacity=16)
    with pytest.raises(OSError):
        subetha.VyukovRing.open(scratch("absent"), capacity=8)


def test_a_vyukov_pop_wakes_on_a_push_through_another_handle(scratch):
    # The two handles share nothing but the files, so the wake can only
    # travel through the wakers beside the ring.
    path = scratch("vyukov")
    waiting = subetha.VyukovRing(path, capacity=4)
    pushing = subetha.VyukovRing.open(path, capacity=4)
    waiter, answer = wait_on_a_thread(lambda: waiting.pop_for(timeout=30.0))
    assert pushing.push(b"wake") is True
    waiter.join(timeout=30.0)
    assert answer and answer[0] is not None, "the wait ran out instead of waking"
    assert answer[0].startswith(b"wake")


def test_a_vyukov_push_wakes_on_a_pop_through_another_handle(scratch):
    path = scratch("vyukov")
    waiting = subetha.VyukovRing(path, capacity=2)
    popping = subetha.VyukovRing.open(path, capacity=2)
    assert waiting.push_many([b"a", b"b"]) == 2
    pusher, answer = wait_on_a_thread(lambda: waiting.push_for(b"late", timeout=30.0))
    assert popping.pop().startswith(b"a")
    pusher.join(timeout=30.0)
    assert answer == [True], "the wait ran out instead of waking"
    assert [item.rstrip(b"\0") for item in popping.pop_many(4)] == [b"b", b"late"]


def test_a_vyukov_wait_answers_none_or_false_when_its_time_runs_out():
    ring = subetha.VyukovRing.anon(2)
    began = time.monotonic()
    assert ring.pop_for(timeout=0.05) is None
    assert time.monotonic() - began >= 0.04, "it must wait, not give up at once"
    assert ring.push_many([b"a", b"b"]) == 2
    assert ring.push_for(b"c", timeout=0.05) is False
    assert ring.pop_for() is not None, "an item that is there is taken without waiting"


def test_closing_a_vyukov_handle_ends_its_waits_and_no_one_elses(scratch):
    path = scratch("vyukov")
    closing = subetha.VyukovRing(path, capacity=4)
    other = subetha.VyukovRing.open(path, capacity=4)
    waiter, answer = wait_on_a_thread(lambda: closing.pop_for(timeout=30.0))
    closing.close()
    waiter.join(timeout=30.0)
    assert len(answer) == 1 and isinstance(answer[0], OSError), answer
    with pytest.raises(OSError):
        closing.pop_for(timeout=5.0)
    assert other.pop_for(timeout=0.05) is None, "a wait through another handle carries on"
    other.push(b"still")
    assert closing.pop_for(timeout=5.0).startswith(b"still"), "an item that is there is still taken"


def test_vyukov_stats_count_what_went_through():
    ring = subetha.VyukovRing.anon(4)
    assert ring.push_many([b"a", b"b", b"c"]) == 3
    assert ring.pop() is not None
    assert ring.stats() == {
        "capacity": 4,
        "producer_seq": 3,
        "consumer_seq": 1,
        "approx_len": 2,
        "waiters_full": 0,
    }


def test_a_vyukov_slot_a_dead_producer_claimed_is_found_and_healed(scratch):
    # A producer that dies between claiming a slot and publishing it leaves
    # the claim counted and the slot's sequence unmoved. Writing the claim
    # counter, the header's second cache line, into a fresh ring's file
    # leaves exactly that behind for slot 0.
    path = scratch("vyukov")
    ring = subetha.VyukovRing(path, capacity=4)
    assert ring.next_stuck_slot() is None
    assert ring.heal_stuck_slot(0) is False, "a position nobody claimed is not stuck"
    del ring
    gc.collect()
    with open(path, "r+b") as f:
        f.seek(64)
        f.write((1).to_bytes(8, sys.byteorder))

    ring = subetha.VyukovRing.open(path, capacity=4)
    assert ring.next_stuck_slot() == 0
    assert ring.next_stuck_slot(start=1) is None
    assert ring.pop() is None, "a consumer cannot drain past an unpublished slot"
    assert ring.heal_stuck_slot(0) is True
    assert ring.next_stuck_slot() is None
    assert ring.pop() == bytes(ring.payload_size), "a slot on its first lap holds zeros"


def test_unlinking_a_vyukov_ring_removes_its_file_and_wakers(scratch):
    path = scratch("vyukov")
    ring = subetha.VyukovRing(path, capacity=4)
    ring.push(b"gone")
    ring.flush()
    ring.wake_all()
    del ring
    gc.collect()
    assert subetha.VyukovRing.unlink(path) == {"removed": 3, "missing": 0}
    assert not os.path.exists(path)
    assert not os.path.exists(path + ".cwaker.bin")
    assert subetha.VyukovRing.unlink(path) == {"removed": 0, "missing": 3}


def test_a_vyukov_ring_in_shared_memory_wakes_across_handles():
    name = vyukov_shm_name()
    made = subetha.VyukovRing.create_shm(name, 4)
    opened = subetha.VyukovRing.open_shm(name, 4)
    assert made.push(b"over") is True
    assert opened.pop().startswith(b"over")
    waiter, answer = wait_on_a_thread(lambda: opened.pop_for(timeout=30.0))
    assert made.push(b"wake") is True
    waiter.join(timeout=30.0)
    assert answer and answer[0] is not None, "the wait ran out instead of waking"
    assert answer[0].startswith(b"wake")


def test_a_vyukov_shm_namespace_is_one_the_library_knows():
    with pytest.raises(ValueError):
        subetha.VyukovRing.create_shm(vyukov_shm_name(), 4, namespace="galaxy")
    with pytest.raises(ValueError):
        subetha.VyukovRing.create_shm(vyukov_shm_name(), 4, namespace="appcontainer")
    with pytest.raises(OSError):
        subetha.VyukovRing.open_shm(vyukov_shm_name(), 4)


def test_a_vyukov_ring_is_a_context_manager():
    with subetha.VyukovRing.anon(4) as ring:
        assert ring.push(b"inside") is True
    assert ring.pop().startswith(b"inside"), "leaving the block closes nothing"
    assert "VyukovRing" in repr(ring)


# The MPSC pool and MPMC grid. The producers are deliberately bound to
# one thread, which is what the Rust type says, so the classes are
# unsendable rather than pretending otherwise.


def test_an_mpsc_pool_drains_every_producer(scratch):
    producers, consumer = subetha.mpsc_pool(scratch("pool"), producers=3, capacity=8)
    assert len(producers) == 3
    assert consumer.producers == 3

    for i, producer in enumerate(producers):
        producer.push(bytes([i]) * 4)

    drained = consumer.pop_many(10)
    assert len(drained) == 3, "one item from each producer should arrive"
    assert {item[0] for item in drained} == {0, 1, 2}


def test_an_mpsc_pool_needs_a_producer(scratch):
    with pytest.raises(ValueError):
        subetha.mpsc_pool(scratch("pool"), producers=0, capacity=8)


def test_an_mpmc_grid_shares_the_rings_out(scratch):
    producers, consumers = subetha.mpmc_grid(
        scratch("grid"), producers=4, consumers=2, capacity=8
    )
    assert len(producers) == 4
    assert len(consumers) == 2
    assert sum(c.rings for c in consumers) == 4, "every ring belongs to a consumer"


def test_an_mpmc_grid_refuses_more_consumers_than_producers(scratch):
    with pytest.raises(ValueError):
        subetha.mpmc_grid(scratch("grid"), producers=1, consumers=2, capacity=8)


def test_an_mpmc_grid_needs_a_consumer(scratch):
    with pytest.raises(ValueError):
        subetha.mpmc_grid(scratch("grid"), producers=2, consumers=0, capacity=8)


# Shared state.


def test_a_cell_holds_a_value_and_steps_its_version(scratch):
    cell = subetha.Cell(scratch("cell"), value_size=8)
    before = cell.version
    cell.set(b"12345678")
    assert cell.get() == b"12345678"
    assert cell.version > before, "a write should move the version"


def test_a_cell_seen_from_a_second_handle(scratch):
    path = scratch("cell")
    writer = subetha.Cell(path, value_size=4)
    writer.set(b"abcd")
    reader = subetha.Cell.open(path, value_size=4)
    assert reader.get() == b"abcd"


def test_a_cell_counts_each_write_once_and_load_answers_the_version(scratch):
    cell = subetha.Cell(scratch("cell"), value_size=4)
    assert cell.load() == (0, bytes(4)), "a fresh cell holds zeros at version 0"
    assert cell.set(b"abcd") == 1
    assert cell.load() == (1, b"abcd")
    assert cell.version == 1


def test_a_cell_compare_and_set_lands_only_on_the_version_it_saw(scratch):
    path = scratch("cell")
    a = subetha.Cell(path, value_size=4)
    b = subetha.Cell.open(path, value_size=4)
    seen, _ = a.load()
    assert b.compare_and_set(seen, b"won!") == (True, seen + 1)
    assert a.compare_and_set(seen, b"lost") == (False, seen + 1), "the loser learns the winner's version"
    assert a.load() == (seen + 1, b"won!")


def test_a_vec_appends_reads_and_pops(scratch):
    vec = subetha.Vec(scratch("vec"), capacity=8, writers=1, element_size=4)
    assert len(vec) == 0
    assert vec.push(b"aaaa") == 0
    assert vec.push(b"bbbb") == 1
    assert len(vec) == 2
    assert vec.get(0) == b"aaaa"
    assert vec.pop() == b"bbbb"
    assert len(vec) == 1


def test_a_full_vec_answers_none_rather_than_raising(scratch):
    vec = subetha.Vec(scratch("vec"), capacity=2, writers=1, element_size=2)
    assert vec.push(b"aa") is not None
    assert vec.push(b"bb") is not None
    assert vec.push(b"cc") is None, "a full vec should answer None"


def test_a_vec_reads_a_range_in_one_call(scratch):
    vec = subetha.Vec(scratch("vec"), capacity=8, writers=1, element_size=2)
    vec.push_many([bytes([i]) * 2 for i in range(5)])
    packed = vec.read_range(1, 3)
    assert packed == b"\x01\x01\x02\x02\x03\x03"


def test_a_vec_read_range_stops_at_the_end_of_what_is_live(scratch):
    vec = subetha.Vec(scratch("vec"), capacity=8, writers=1, element_size=2)
    vec.push_many([b"aa", b"bb"])
    assert vec.read_range(0, 100) == b"aabb", "it should stop rather than run off the end"


def test_a_vec_writes_a_range_in_one_call(scratch):
    vec = subetha.Vec(scratch("vec"), capacity=8, writers=1, element_size=2)
    vec.push_many([b"..", b"..", b".."])
    assert vec.write_range(0, b"xxyyzz") == 3
    assert vec.read_range(0, 3) == b"xxyyzz"


def test_a_vec_refuses_a_partial_element_in_write_range(scratch):
    vec = subetha.Vec(scratch("vec"), capacity=4, writers=1, element_size=4)
    vec.push(b"....")
    with pytest.raises(ValueError):
        vec.write_range(0, b"xyz")


def test_a_map_inserts_reads_and_removes(scratch):
    m = subetha.HashMap(scratch("map"), capacity=16, key_size=4, value_size=8)
    assert m.insert(b"key1", b"value___") == "inserted"
    assert m.insert(b"key1", b"other___") == "updated"
    assert m.get(b"key1") == b"other___"
    assert b"key1" in m
    assert len(m) == 1
    assert m.remove(b"key1") == b"other___"
    assert m.get(b"key1") is None
    assert b"key1" not in m


def test_a_map_answers_none_for_an_absent_key(scratch):
    m = subetha.HashMap(scratch("map"), capacity=8, key_size=2, value_size=2)
    assert m.get(b"no") is None
    assert m.remove(b"no") is None


def test_a_map_looks_up_many_keys_in_one_call(scratch):
    m = subetha.HashMap(scratch("map"), capacity=16, key_size=2, value_size=2)
    m.insert_many([(b"aa", b"11"), (b"bb", b"22")])
    assert m.get_many([b"aa", b"zz", b"bb"]) == [b"11", None, b"22"]


def test_a_slab_reads_and_writes_by_index(scratch):
    slab = subetha.Slab(scratch("slab"), capacity=8, element_size=4)
    assert slab.capacity == 8
    slab.set(3, b"abcd")
    assert slab.get(3) == b"abcd"


def test_a_slab_slot_version_moves_on_a_write(scratch):
    slab = subetha.Slab(scratch("slab"), capacity=4, element_size=2)
    before = slab.slot_version(0)
    slab.set(0, b"xy")
    assert slab.slot_version(0) > before, "the slot's write count should step"


def test_a_slab_reads_a_range_in_one_call(scratch):
    slab = subetha.Slab(scratch("slab"), capacity=8, element_size=2)
    slab.write_range(0, b"aabbcc")
    assert slab.read_range(0, 3) == b"aabbcc"


def test_a_slab_refuses_a_partial_element(scratch):
    slab = subetha.Slab(scratch("slab"), capacity=4, element_size=4)
    with pytest.raises(ValueError):
        slab.write_range(0, b"xyz")


def test_a_slab_index_past_the_end_raises(scratch):
    slab = subetha.Slab(scratch("slab"), capacity=2, element_size=2)
    with pytest.raises(IndexError):
        slab.get(99)


def test_a_btree_keeps_its_keys_in_order(scratch):
    tree = subetha.BTreeMap(scratch("tree"), capacity=64, key_size=2, value_size=2)
    for key in [b"cc", b"aa", b"bb"]:
        tree.insert(key, key)
    assert tree.first() == (b"aa", b"aa"), "the smallest key should come first"
    assert tree.last() == (b"cc", b"cc"), "the largest key should come last"


def test_a_btree_insert_reports_what_was_there(scratch):
    tree = subetha.BTreeMap(scratch("tree"), capacity=32, key_size=2, value_size=2)
    assert tree.insert(b"aa", b"11") is None, "nothing was there before"
    assert tree.insert(b"aa", b"22") == b"11", "it should hand back the old value"
    assert tree.get(b"aa") == b"22"


def test_a_btree_removes_and_forgets(scratch):
    tree = subetha.BTreeMap(scratch("tree"), capacity=32, key_size=2, value_size=2)
    tree.insert(b"aa", b"11")
    assert b"aa" in tree
    assert tree.remove(b"aa") == b"11"
    assert b"aa" not in tree
    assert tree.remove(b"aa") is None


def test_an_empty_btree_has_no_first_or_last(scratch):
    tree = subetha.BTreeMap(scratch("tree"), capacity=16, key_size=2, value_size=2)
    assert tree.first() is None
    assert tree.last() is None


def test_a_btree_needs_a_capacity(scratch):
    with pytest.raises(ValueError):
        subetha.BTreeMap(scratch("tree"), capacity=0, key_size=2, value_size=2)


def test_an_arena_interns_and_resolves(scratch):
    arena = subetha.Arena(scratch("arena"), capacity_bytes=4096)
    ref = arena.intern("hello")
    assert isinstance(ref, int), "a reference is eight bytes, not the string"
    assert arena.get(ref) == "hello"


def test_an_arena_reference_crosses_to_another_handle(scratch):
    path = scratch("arena")
    writer = subetha.Arena(path, capacity_bytes=4096)
    ref = writer.intern("shared between processes")
    reader = subetha.Arena.open(path, capacity_bytes=4096)
    assert reader.get(ref) == "shared between processes"


def test_an_arena_interns_many_in_one_call(scratch):
    arena = subetha.Arena(scratch("arena"), capacity_bytes=4096)
    refs = arena.intern_many(["one", "two", "three"])
    assert len(refs) == 3
    assert arena.get_many(refs) == ["one", "two", "three"]


def test_an_arena_tracks_what_it_has_used(scratch):
    arena = subetha.Arena(scratch("arena"), capacity_bytes=4096)
    before = arena.used_bytes
    arena.intern("some bytes")
    assert arena.used_bytes > before
    assert arena.used_bytes + arena.remaining_bytes <= arena.capacity_bytes


def test_an_arena_holds_bytes_that_are_not_text(scratch):
    arena = subetha.Arena(scratch("arena"), capacity_bytes=1024)
    ref = arena.intern_bytes(b"\xff\xfe\x00")
    assert arena.get_bytes(ref) == b"\xff\xfe\x00"


def test_a_list_pushes_and_pops_at_both_ends(scratch):
    lst = subetha.LinkedList(scratch("list"), capacity=8, element_size=2)
    lst.push_back(b"bb")
    lst.push_front(b"aa")
    lst.push_back(b"cc")
    assert len(lst) == 3
    assert lst.pop_front() == b"aa"
    assert lst.pop_back() == b"cc"
    assert lst.pop_front() == b"bb"
    assert lst.pop_front() is None


def test_a_list_node_index_stays_valid_until_removed(scratch):
    lst = subetha.LinkedList(scratch("list"), capacity=8, element_size=2)
    first = lst.push_back(b"aa")
    lst.push_back(b"bb")
    assert lst.get(first) == b"aa"
    assert lst.remove(first) == b"aa"
    assert len(lst) == 1


def test_a_list_nobody_died_in_reclaims_nothing(scratch):
    lst = subetha.LinkedList(scratch("list"), capacity=8, element_size=2)
    lst.push_back(b"aa")
    assert lst.reclaim() == 0, "no process was killed mid-operation"
    assert lst.pop_front() == b"aa", "reclaiming leaves a live node alone"


def test_a_notifier_wakes_on_a_signal(scratch):
    notifiers = subetha.NotifierSet(scratch("notify"))
    watcher = notifiers.attach()
    assert notifiers.attached >= 1
    notifiers.signal()
    assert watcher.wait(timeout=2.0) is True, "a signaled notifier should wake"


def test_a_notifier_times_out_when_nothing_signals(scratch):
    notifiers = subetha.NotifierSet(scratch("notify"))
    watcher = notifiers.attach()
    watcher.drain()
    assert watcher.wait(timeout=0.05) is False, "a quiet notifier should time out"


def test_a_notifier_wakes_from_another_thread(scratch):
    import threading

    notifiers = subetha.NotifierSet(scratch("notify"))
    watcher = notifiers.attach()
    watcher.drain()

    timer = threading.Timer(0.05, notifiers.signal)
    timer.start()
    try:
        # Only passes if the wait released the interpreter: otherwise the
        # timer thread never runs and this times out.
        assert watcher.wait(timeout=10.0) is True
    finally:
        timer.cancel()


def test_a_notifier_hands_out_its_native_object(scratch):
    notifiers = subetha.NotifierSet(scratch("notify"))
    watcher = notifiers.attach()
    # A file descriptor on Unix, an event handle on Windows. Both are
    # integers here; what an event loop can do with it differs.
    assert isinstance(watcher.native, int)
    assert isinstance(watcher.index, int)


def test_a_notifier_refuses_a_negative_timeout(scratch):
    notifiers = subetha.NotifierSet(scratch("notify"))
    watcher = notifiers.attach()
    with pytest.raises(ValueError):
        watcher.wait(timeout=-1.0)


def test_a_shared_arc_holds_a_value_and_counts_its_holders(scratch):
    arc = subetha.SharedArc(scratch("arc"), value=b"shared payload", keep_on_last=True)
    assert arc.get() == b"shared payload"
    assert arc.holders >= 1
    assert arc.value_size == len(b"shared payload")


def test_a_second_holder_attaches_to_the_same_value(scratch):
    path = scratch("arc")
    first = subetha.SharedArc(path, value=b"once written", keep_on_last=True)
    second = subetha.SharedArc.open(path, value_bytes=first.value_size, keep_on_last=True)
    assert second.get() == b"once written"
    assert second.holders >= 2, "both holders should be counted"


def test_a_shared_arc_reads_and_writes_part_of_its_value(scratch):
    arc = subetha.SharedArc(scratch("arc"), value=bytes(32), keep_on_last=True)
    arc.write_at(8, b"field")
    assert arc.read_at(8, 5) == b"field"
    assert arc.read_at(0, 4) == bytes(4), "the rest should be untouched"


def test_a_shared_arc_needs_a_holder(scratch):
    with pytest.raises(ValueError):
        subetha.SharedArc(scratch("arc"), value=b"x", max_holders=0)


def test_one_process_takes_leadership_and_knows_it(scratch):
    election = subetha.LeaderElection(scratch("leader"))
    assert election.leader is None, "nobody should lead before anyone claims"
    assert election.try_claim() is True
    assert election.am_i_leader() is True
    assert election.leader == os.getpid()


def test_a_second_claimant_is_refused_while_the_leader_lives(scratch):
    election = subetha.LeaderElection(scratch("leader"))
    election.try_claim(pid=1111)
    # A different pid asking while 1111 is fresh should be told no.
    assert election.try_claim(pid=2222) is False
    assert election.leader == 1111


def test_a_leader_that_steps_down_frees_the_role(scratch):
    election = subetha.LeaderElection(scratch("leader"))
    election.try_claim(pid=1111)
    assert election.step_down(pid=1111) is True
    assert election.try_claim(pid=2222) is True, "the role should be free at once"
    assert election.leader == 2222


def test_beating_as_a_former_leader_reports_the_loss(scratch):
    election = subetha.LeaderElection(scratch("leader"))
    election.try_claim(pid=1111)
    election.step_down(pid=1111)
    election.try_claim(pid=2222)
    # The answer a former leader needs: you are not it any more.
    assert election.beat(pid=1111) is False


def test_the_term_moves_when_the_role_changes_hands(scratch):
    election = subetha.LeaderElection(scratch("leader"))
    election.try_claim(pid=1111)
    first = election.term
    election.step_down(pid=1111)
    election.try_claim(pid=2222)
    assert election.term > first, "a new leader should carry a new term"


def test_a_quiet_leader_is_taken_over_once_the_epoch_passes_its_grace(scratch):
    election = subetha.LeaderElection(scratch("leader"))
    assert election.try_claim(pid=1111) is True
    start = election.global_epoch
    # 2222 is the higher id, so only a quiet leader's takeover lets it in.
    assert election.try_claim(pid=2222, grace_epochs=2) is False
    assert election.tick_epoch() == start + 1, "a tick answers the epoch it reached"
    assert election.tick_epoch() == start + 2
    assert election.global_epoch == start + 2
    assert election.try_claim(pid=2222, grace_epochs=2) is False, "two epochs behind is within a grace of two"
    election.tick_epoch()
    assert election.try_claim(pid=2222, grace_epochs=2) is True, "three behind is past it"
    assert election.leader == 2222


def test_a_lazy_value_is_claimed_once_and_read_by_the_rest(scratch):
    path = scratch("lazy")
    producer = subetha.LazyValue(path, value_bytes=4)
    reader = subetha.LazyValue.open(path, value_bytes=4)
    assert producer.claim() is True
    assert reader.claim() is False, "a claim held by a live process stands"
    assert reader.get() is None, "nothing is published yet"
    assert producer.publish(b"done") is True
    assert reader.get() == b"done"
    assert reader.claim() is False, "a published value is not claimed again"


def test_a_lazy_value_claim_whose_process_is_gone_is_taken_over(scratch):
    value = subetha.LazyValue(scratch("lazy-gone"), value_bytes=4)
    # A pid this high is not a live process on any of the gate hosts.
    assert value.claim(pid=0x7FFF_FFF0) is True
    assert value.get() is None, "nothing is published under the dead claim"
    assert value.claim() is True, "the next claim takes over the dead one"
    assert value.publish(b"done") is True
    assert value.get() == b"done"


def test_a_holder_table_claims_publishes_and_releases(scratch):
    table = subetha.HolderTable(scratch("holders"), capacity=4)
    slot = table.claim(payload=42)
    assert slot is not None
    assert table.payload(slot) == 42
    assert table.live == 1
    table.release(slot)
    assert table.live == 0
    assert table.payload(slot) is None


def test_a_full_holder_table_answers_none(scratch):
    table = subetha.HolderTable(scratch("holders"), capacity=2)
    first = table.claim(1)
    second = table.claim(2)
    assert first is not None and second is not None
    assert table.claim(3) is None, "a full table should answer None"
    table.release(first)
    assert table.claim(3) is not None, "a released slot should come back"


def test_a_holder_slot_can_be_reserved_then_published(scratch):
    table = subetha.HolderTable(scratch("holders"), capacity=2)
    slot = table.reserve()
    assert slot is not None
    table.publish(slot, 99)
    assert table.payload(slot) == 99


def test_a_holder_slot_past_the_capacity_raises_index_error(scratch):
    table = subetha.HolderTable(scratch("holders"), capacity=2)
    for call in (lambda: table.publish(2, 7), lambda: table.payload(2), lambda: table.release(2)):
        with pytest.raises(IndexError, match="slot 2 is past the 2 the table holds"):
            call()


def test_a_holder_payload_the_table_keeps_for_itself_raises_value_error(scratch):
    table = subetha.HolderTable(scratch("holders"), capacity=2)
    for payload in (0, 2**64 - 1):
        with pytest.raises(ValueError):
            table.claim(payload)
    assert table.live == 0, "a refused claim holds no slot"
    slot = table.reserve()
    with pytest.raises(ValueError):
        table.publish(slot, 0)


def test_a_heartbeat_table_hands_out_slots_and_beats(scratch):
    table = subetha.Heartbeat(scratch("beat"), capacity=4)
    slot = table.register()
    table.beat(slot)
    seen = table.snapshot(slot)
    assert seen is not None
    assert seen["pid"] == os.getpid(), "the slot should carry this process"
    table.unregister(slot)


def test_a_heartbeat_slot_that_nobody_holds_reads_as_none(scratch):
    table = subetha.Heartbeat(scratch("beat"), capacity=4)
    assert table.snapshot(3) is None


def test_a_heartbeat_table_that_is_full_raises(scratch):
    table = subetha.Heartbeat(scratch("beat"), capacity=1)
    table.register()
    with pytest.raises(OSError):
        table.register()


def test_a_heartbeat_global_epoch_moves_when_ticked(scratch):
    table = subetha.Heartbeat(scratch("beat"), capacity=2)
    before = table.global_epoch
    after = table.tick_global_epoch()
    assert after > before


def test_a_heartbeat_slot_past_the_capacity_raises_index_error(scratch):
    table = subetha.Heartbeat(scratch("beat"), capacity=2)
    for call in (lambda: table.beat(2), lambda: table.snapshot(2), lambda: table.unregister(2)):
        with pytest.raises(IndexError, match="slot 2 is past the 2 the table holds"):
            call()


def test_a_barrier_opens_when_the_only_participant_arrives(scratch):
    table = subetha.Heartbeat(scratch("beat"), capacity=4)
    slot = table.register()
    table.beat(slot)
    barrier = subetha.EpochBarrier(scratch("barrier"), table)
    assert barrier.live_peers >= 1
    assert barrier.wait(epoch=0, timeout=5.0) is True


def test_a_barrier_times_out_when_someone_never_arrives(scratch):
    table = subetha.Heartbeat(scratch("beat"), capacity=4)
    for _ in range(2):
        table.beat(table.register())
    barrier = subetha.EpochBarrier(scratch("barrier"), table)
    # Two live peers, one arrival: the barrier should not open, and it
    # should say so rather than waiting for ever.
    assert barrier.wait(epoch=0, timeout=0.1) is False


def test_a_barrier_reports_its_epoch_and_arrivals_together(scratch):
    table = subetha.Heartbeat(scratch("beat"), capacity=4)
    table.beat(table.register())
    barrier = subetha.EpochBarrier(scratch("barrier"), table)
    epoch, arrived = barrier.snapshot()
    assert epoch == barrier.current_epoch
    assert isinstance(arrived, int)


def test_a_barrier_refuses_a_timeout_that_is_not_positive(scratch):
    table = subetha.Heartbeat(scratch("beat"), capacity=2)
    table.beat(table.register())
    barrier = subetha.EpochBarrier(scratch("barrier"), table)
    with pytest.raises(ValueError):
        barrier.wait(epoch=0, timeout=0.0)


def test_a_condvar_returns_at_once_when_the_predicate_already_holds(scratch):
    cv = subetha.Condvar(scratch("cv"))
    assert cv.wait_for(lambda: True, timeout=5.0) is True


def test_a_condvar_times_out_rather_than_waiting_for_ever(scratch):
    cv = subetha.Condvar(scratch("cv"))
    assert cv.wait_for(lambda: False, timeout=0.05) is False, "a timeout should answer False"


def test_a_condvar_wakes_when_the_predicate_becomes_true(scratch):
    import threading

    cv = subetha.Condvar(scratch("cv"))
    state = {"ready": False}

    def make_it_true():
        state["ready"] = True
        cv.notify_all()

    timer = threading.Timer(0.05, make_it_true)
    timer.start()
    try:
        assert cv.wait_for(lambda: state["ready"], timeout=10.0) is True
    finally:
        timer.cancel()


def test_a_condvar_steps_its_generation_on_notify(scratch):
    cv = subetha.Condvar(scratch("cv"))
    before = cv.generation
    cv.notify_all()
    assert cv.generation > before, "a notify should move the generation"


def test_a_condvar_reraises_what_the_predicate_raised(scratch):
    cv = subetha.Condvar(scratch("cv"))

    class Boom(Exception):
        pass

    def angry():
        raise Boom

    # It must not park for ever on a predicate it cannot evaluate, and it
    # must not report the failure as a satisfied condition.
    with pytest.raises(Boom):
        cv.wait_for(angry, timeout=5.0)


def test_a_condvar_refuses_a_timeout_that_is_not_positive(scratch):
    cv = subetha.Condvar(scratch("cv"))
    with pytest.raises(ValueError):
        cv.wait_for(lambda: True, timeout=0.0)


def test_a_capacity_ring_carries_items(scratch):
    ring = subetha.CapacityRing(scratch("cap"), capacity=8)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    assert ring.send(producer, b"before") is True
    assert ring.recv(consumer).startswith(b"before")


def test_a_capacity_ring_keeps_its_wakers_beside_its_files(scratch):
    # A waiter in any process, a C one included, parks on these files, and
    # a send or receive from here wakes it through them.
    path = scratch("cap-wakers")
    ring = subetha.CapacityRing(path, capacity=8)
    assert os.path.exists(path + ".cwaker.bin")
    assert os.path.exists(path + ".pwaker.bin")
    assert ring.capacity == 8


def test_a_capacity_ring_resizes_without_losing_what_is_in_flight(scratch):
    ring = subetha.CapacityRing(scratch("cap"), capacity=4)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    ring.send_many(producer, [b"one", b"two"])

    ring.morph_to(16)
    assert ring.capacity == 16

    # The items were written to the old backing. The whole point of the
    # morph is that they are still readable.
    taken = ring.recv_many(consumer, 10)
    assert len(taken) == 2, "a resize must not drop what was already in the ring"
    assert taken[0].startswith(b"one")
    assert taken[1].startswith(b"two")


def test_a_capacity_ring_works_after_a_resize(scratch):
    ring = subetha.CapacityRing(scratch("cap"), capacity=4)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    ring.morph_to(32)
    assert ring.send(producer, b"after") is True
    assert ring.recv(consumer).startswith(b"after")


def test_a_capacity_ring_refuses_a_capacity_that_is_not_a_power_of_two(scratch):
    with pytest.raises(ValueError):
        subetha.CapacityRing(scratch("cap"), capacity=6)
    ring = subetha.CapacityRing(scratch("cap2"), capacity=4)
    with pytest.raises(ValueError):
        ring.morph_to(6)


def test_a_capacity_ring_prewarms(scratch):
    ring = subetha.CapacityRing(scratch("cap"), capacity=4)
    ring.prewarm(64)
    # Prewarming builds the backing but does not switch to it.
    assert ring.capacity == 4
    assert ring.warm_capacity == 64
    ring.morph_to(64)
    assert ring.capacity == 64
    assert ring.warm_hits == 1, "the morph should have taken the prewarmed backing"


def test_a_capacity_ring_gives_back_a_prewarmed_backing(scratch):
    ring = subetha.CapacityRing(scratch("cap"), capacity=4)
    ring.prewarm(64)
    ring.clear_warm()
    assert ring.warm_capacity is None
    # The morph still works, it just has to build the backing itself.
    ring.morph_to(64)
    assert ring.capacity == 64
    assert ring.warm_hits == 0


def test_a_resize_counts_what_was_read_from_the_old_backing(scratch):
    ring = subetha.CapacityRing(scratch("cap"), capacity=4)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    ring.send_many(producer, [b"one", b"two"])
    ring.morph_to(16)
    assert ring.recv_many(consumer, 10)
    assert ring.stale_pops == 2, "both items came from the superseded backing"


def test_a_capacity_ring_is_unstamped_unless_asked(scratch):
    ring = subetha.CapacityRing(scratch("cap"), capacity=8)
    assert ring.stamped is False
    assert ring.ordering_mode is None


def test_a_stamped_capacity_ring_reports_its_ordering(scratch):
    ring = subetha.CapacityRing(scratch("cap"), capacity=8, stamped=True)
    assert ring.stamped is True
    assert ring.ordering_mode == "unordered"
    assert ring.inversions == 0
    ring.set_ordering_mode("merge_by_stamp")
    assert ring.ordering_mode == "merge_by_stamp"
    ring.set_ordering_mode("merge_strict")
    assert ring.ordering_mode == "merge_strict"


def test_a_stamped_capacity_ring_still_carries_items(scratch):
    ring = subetha.CapacityRing(scratch("cap"), capacity=8, stamped=True)
    ring.set_ordering_mode("merge_by_stamp")
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    ring.send_many(producer, [b"one", b"two"])
    taken = ring.recv_many(consumer, 10)
    assert len(taken) == 2
    assert taken[0].startswith(b"one")
    assert taken[1].startswith(b"two")


def test_a_capacity_ring_refuses_an_ordering_mode_it_does_not_have(scratch):
    ring = subetha.CapacityRing(scratch("cap"), capacity=8, stamped=True)
    with pytest.raises(ValueError):
        ring.set_ordering_mode("whatever_seems_right")


def test_a_capacity_ring_can_be_opened_by_another_holder(scratch):
    path = scratch("cap")
    ring = subetha.CapacityRing(path, capacity=8)
    producer = ring.register_producer()
    ring.send(producer, b"across")

    second = subetha.CapacityRing.open(path, capacity=8)
    consumer = second.register_consumer()
    assert second.recv(consumer).startswith(b"across")


def test_a_loss_kind_tells_noise_from_a_full_queue(scratch):
    which = subetha.LossKind()
    # A steady spacing establishes what normal looks like.
    for _ in range(40):
        which.observe_spacing(1000.0)

    # A gap at the normal spacing is noise, not a queue.
    assert which.classify(1, 1000.0) == "wireless"
    # A gap far wider than normal is a queue that filled.
    assert which.classify(1, 8000.0) == "congestion"
    assert 0.0 <= which.congestion_share <= 1.0


def test_a_loss_kind_refuses_a_spacing_that_is_not_one(scratch):
    which = subetha.LossKind()
    with pytest.raises(ValueError):
        which.observe_spacing(-1.0)
    with pytest.raises(ValueError):
        which.observe_delay(float("inf"))


def test_loss_bursts_says_nothing_until_it_has_seen_enough(scratch):
    runs = subetha.LossBursts()
    assert runs.mean_run_length is None, "None is not zero: nothing measured yet"
    assert runs.samples == 0


def test_loss_bursts_finds_runs_where_there_are_runs(scratch):
    runs = subetha.LossBursts()
    # Twenty clean, then five lost together, over and over.
    pattern = [False] * 20 + [True] * 5
    for _ in range(40):
        runs.observe_many(pattern)

    assert runs.samples == 40 * len(pattern)
    assert runs.mean_run_length is not None
    assert runs.mean_run_length > 1.0, "losses arriving together is a run"


def test_loss_bursts_sees_single_losses_as_single(scratch):
    runs = subetha.LossBursts()
    pattern = [False] * 9 + [True]
    for _ in range(100):
        runs.observe_many(pattern)

    assert runs.mean_run_length is not None
    # Isolated losses give runs near one, clearly shorter than the
    # clustered case above.
    assert runs.mean_run_length < 3.0


def test_timing_reports_jitter_and_spacing(scratch):
    beat = subetha.Timing(window=32)
    # A steady arrival every millisecond with the receiving clock a
    # fixed amount ahead.
    for n in range(64):
        beat.observe(sent=n * 1000, received=n * 1000 + 500_000)

    assert beat.samples > 0
    assert beat.spacing > 0, "the spacing between arrivals is measured"
    assert beat.jitter >= 0
    # A steady stream is not a queue filling.
    assert abs(beat.trend_debiased) < 1000


def test_timing_sees_a_delay_that_is_climbing(scratch):
    beat = subetha.Timing(window=64)
    # Each item arrives a little later than the last relative to when it
    # was sent, which is a queue filling.
    for n in range(64):
        beat.observe(sent=n * 1000, received=n * 1000 + 500_000 + n * 200)

    assert beat.trend > 0, "a delay climbing must read as climbing"


def test_timing_refuses_a_window_of_one(scratch):
    with pytest.raises(ValueError):
        subetha.Timing(window=1)


def test_timing_refuses_a_receive_time_that_goes_back(scratch):
    beat = subetha.Timing(window=8)
    beat.observe(sent=0, received=1_000)
    beat.observe(sent=1_000, received=2_000)
    with pytest.raises(ValueError):
        beat.observe(sent=2_000, received=1_500)
    assert beat.samples == 2, "a refused item is not counted"
    # A send time may go back, for an item that arrived out of order.
    beat.observe(sent=1_500, received=2_000)
    assert beat.samples == 3


def test_a_round_trip_shape_starts_with_nothing_to_say(scratch):
    shape = subetha.RoundTripShape()
    assert shape.two_groups is None
    assert shape.samples == 0
    assert 0.0 <= shape.wireless_confidence <= 1.0


def test_a_round_trip_shape_takes_a_run_of_readings(scratch):
    shape = subetha.RoundTripShape()
    # Two clusters: quick ones, and ones that waited for a retry.
    quick = [5000.0] * 60
    retried = [45000.0] * 40
    assert shape.observe_many(quick + retried) == 100
    assert shape.samples == 100
    assert 0.0 <= shape.wireless_confidence <= 1.0


def test_a_round_trip_shape_refuses_a_reading_that_is_not_one(scratch):
    shape = subetha.RoundTripShape()
    with pytest.raises(ValueError):
        shape.observe(-5.0)


def test_periodicity_finds_nothing_in_a_flat_stream(scratch):
    beat = subetha.Periodicity()
    for n in range(200):
        beat.observe(1000.0, n * 1000)
    # Nothing spikes, so there is no beat and it says so rather than
    # inventing one.
    assert beat.period is None or beat.period[1] >= 0


def test_capacity_says_nothing_before_it_has_probes(scratch):
    path = subetha.Capacity(probe_bytes=1400)
    assert path.link_capacity is None
    assert path.available is None
    assert path.samples == (0, 0)


def test_capacity_takes_pairs_and_trains(scratch):
    path = subetha.Capacity(probe_bytes=1400)
    for n in range(20):
        base = n * 10_000.0
        path.observe_pair(0, base)
        path.observe_pair(1, base + 120.0)
    for n in range(20):
        path.observe_train(200_000.0 + n * 150.0)

    pairs, train = path.samples
    assert pairs > 0
    assert train > 0
    path.reset()
    assert path.samples == (0, 0)


def test_capacity_refuses_a_probe_of_no_bytes(scratch):
    with pytest.raises(ValueError):
        subetha.Capacity(probe_bytes=0)


def test_a_forecast_follows_what_it_is_fed(scratch):
    ahead = subetha.Forecast()
    for _ in range(20):
        ahead.observe(bytes=125_000, seconds=1.0)

    # A steady 125,000 bytes a second reads as exactly that, in bytes a
    # second, and the forecast is a lower bound beneath it.
    assert ahead.mean_rate == 125_000
    assert 0 < ahead.next_rate < ahead.mean_rate


def test_a_forecast_refuses_an_interval_of_no_time(scratch):
    ahead = subetha.Forecast()
    with pytest.raises(ValueError):
        ahead.observe(bytes=100, seconds=0.0)


def test_path_changes_sees_a_route_that_moved(scratch):
    route = subetha.PathChanges()
    assert route.last is None

    for _ in range(30):
        route.observe(ttl=54, congestion_mark=0, hops=10)
    steady = route.route_movement

    # The hop count changes, which is the route moving under the traffic.
    for _ in range(30):
        route.observe(ttl=50, congestion_mark=0, hops=14)

    assert route.last == (50, 0, 14)
    assert route.route_movement >= steady


def test_path_changes_counts_what_was_marked(scratch):
    route = subetha.PathChanges()
    for n in range(40):
        route.observe(ttl=54, congestion_mark=3 if n % 2 else 0, hops=10)
    assert 0.0 <= route.marked_share <= 1.0


def test_a_tiny_bloom_never_forgets_what_was_added(scratch):
    seen = subetha.TinyBloom()
    added = [f"k{n}".encode() for n in range(subetha.TinyBloom.suggested_capacity)]
    seen.insert_many(added)
    assert all(key in seen for key in added), "a bloom filter never says no wrongly"


def test_a_tiny_bloom_fits_in_one_number(scratch):
    seen = subetha.TinyBloom([b"one", b"two"])
    carried = seen.bits
    assert isinstance(carried, int)

    # That number is the whole filter: rebuilt from it, it answers the same.
    rebuilt = subetha.TinyBloom.from_bits(carried)
    assert b"one" in rebuilt
    assert rebuilt.bits == carried
    assert rebuilt.set_bits == seen.set_bits


def test_an_empty_tiny_bloom_says_no_to_everything(scratch):
    seen = subetha.TinyBloom()
    assert seen.bits == 0
    assert seen.set_bits == 0
    assert b"anything" not in seen


def test_a_tiny_bloom_says_what_it_will_cost(scratch):
    few = subetha.TinyBloom.false_positive_rate(4)
    many = subetha.TinyBloom.false_positive_rate(32)
    assert 0.0 <= few < many <= 1.0, "more keys in the same bits means more wrong yeses"


def test_a_fine_bloom_holds_more_than_a_tiny_one(scratch):
    assert subetha.FineBloom.suggested_capacity > subetha.TinyBloom.suggested_capacity
    seen = subetha.FineBloom()
    added = [f"k{n}".encode() for n in range(subetha.FineBloom.suggested_capacity)]
    seen.insert_many(added)
    assert all(key in seen for key in added)


def test_a_clock_orders_two_events_at_the_same_reading(scratch):
    first = subetha.Clock(physical=100, logical=0)
    # The physical time did not move, so the count steps instead.
    second = first.advance(100)
    assert second.physical == 100
    assert second.logical > first.logical
    assert first < second


def test_a_clock_follows_the_physical_time_when_it_moves(scratch):
    first = subetha.Clock(physical=100, logical=5)
    later = first.advance(200)
    assert later.physical == 200
    assert later.logical == 0, "a new physical reading starts the count again"


def test_a_clock_taken_now_is_a_real_reading(scratch):
    taken = subetha.Clock.now()
    assert taken.physical > 0
    assert taken.logical == 0


def test_merging_a_clock_orders_the_received_event_after_its_cause(scratch):
    sender = subetha.Clock(physical=500, logical=3)
    receiver = subetha.Clock(physical=100, logical=0)
    # The receiver's own clock is behind, so the merge takes the
    # sender's reading and steps past it.
    merged = receiver.merge(sender, 100)
    assert merged > sender, "what was received must order after what caused it"


def test_a_causal_clock_tells_before_from_concurrent(scratch):
    start = subetha.CausalClock()
    mine = start.tick(0)
    theirs = start.tick(1)

    assert start.happened_before(mine)
    assert mine.compare(start) == "after"
    assert mine.concurrent_with(theirs), "neither caused the other"
    assert mine.compare(theirs) == "concurrent"


def test_a_causal_clock_merge_takes_the_higher_of_each(scratch):
    mine = subetha.CausalClock().tick(0).tick(0)
    theirs = subetha.CausalClock().tick(1)
    merged = mine.merge(theirs)

    assert merged.count(0) == 2
    assert merged.count(1) == 1
    assert mine.happened_before(merged)
    assert theirs.happened_before(merged)


def test_a_causal_clock_is_equal_to_itself(scratch):
    one = subetha.CausalClock().tick(3)
    assert one.compare(one) == "equal"
    assert one.counts[3] == 1
    assert len(one.counts) == subetha.CausalClock.nodes


def test_a_causal_clock_refuses_a_participant_it_does_not_count(scratch):
    clock = subetha.CausalClock()
    with pytest.raises(ValueError):
        clock.tick(subetha.CausalClock.nodes)
    with pytest.raises(ValueError):
        clock.count(subetha.CausalClock.nodes)


def test_a_clock_refuses_to_step_its_count_past_the_largest(scratch):
    top = subetha.Clock(physical=5, logical=2**64 - 1)
    # No reading at the same physical time orders after this one.
    with pytest.raises(OverflowError):
        top.advance(5)
    with pytest.raises(OverflowError):
        subetha.Clock(physical=5, logical=0).merge(top, 0)
    assert top.advance(6) == subetha.Clock(physical=6, logical=0)


def test_a_fence_clock_refuses_to_step_its_count_past_the_largest(scratch):
    clock = subetha.FenceClock(scratch("fence-top"), 2)
    slot = clock.register()
    # A physical part past the shared clock keeps every step on the count.
    future = 2**63 - 1
    with pytest.raises(OverflowError):
        clock.merge(slot, future, 2**64 - 1)
    assert clock.get_local(slot) == (0, 0), "a refused merge leaves the clock as it was"
    assert clock.merge(slot, future, 2**64 - 2) == (future, 2**64 - 1)
    with pytest.raises(OverflowError):
        clock.tick(slot)
    assert clock.get_local(slot) == (future, 2**64 - 1)


def test_a_fence_clock_slot_past_the_capacity_raises_index_error(scratch):
    clock = subetha.FenceClock(scratch("fence-past"), 2)
    calls = (
        lambda: clock.tick(2),
        lambda: clock.merge(2, 1, 0),
        lambda: clock.get_local(2),
        lambda: clock.unregister(2),
    )
    for call in calls:
        with pytest.raises(IndexError, match="slot 2 is past the 2 the clock holds"):
            call()


def test_a_channel_carries_items(scratch):
    chan = subetha.Channel(scratch("chan"), capacity=64)
    assert chan.send(b"first") is True
    assert chan.recv() == b"first"
    assert chan.recv() is None, "empty is an answer, not a failure"


def test_a_channel_carries_a_run_in_one_crossing(scratch):
    chan = subetha.Channel(scratch("chan"), capacity=64)
    assert chan.send_many([b"one", b"two", b"three"]) == 3
    assert chan.recv_many() == [b"one", b"two", b"three"]


def test_a_channel_keeps_the_length_of_a_short_item(scratch):
    chan = subetha.Channel(scratch("chan"), capacity=64)
    chan.send(b"ab\x00\x00")
    assert chan.recv() == b"ab\x00\x00"


def test_a_channel_fills_up_and_says_so(scratch):
    chan = subetha.Channel(scratch("chan"), capacity=2)
    sent = 0
    while chan.send(b"x"):
        sent += 1
        if sent > 100:
            break
    assert sent <= 100, "a full channel must eventually answer False"
    assert chan.send(b"x") is False


def test_a_channel_refuses_an_item_that_does_not_fit(scratch):
    chan = subetha.Channel(scratch("chan"), capacity=64)
    with pytest.raises(ValueError):
        chan.send(b"x" * (subetha.Channel.max_item_size + 1))


def test_a_channel_waits_for_an_item_and_gives_up(scratch):
    chan = subetha.Channel(scratch("chan"), capacity=64)
    started = time.monotonic()
    assert chan.recv_for(timeout=0.1) is None, "a timeout answers None"
    assert time.monotonic() - started >= 0.05, "and it really waited"


def test_a_channel_wait_returns_as_soon_as_something_arrives(scratch):
    chan = subetha.Channel(scratch("chan"), capacity=64)

    def send_shortly():
        time.sleep(0.05)
        chan.send(b"late")

    sender = threading.Thread(target=send_shortly)
    sender.start()
    # The interpreter is free while this waits, which is what lets the
    # thread above run at all.
    assert chan.recv_for(timeout=10.0) == b"late"
    sender.join(timeout=10)


def test_a_channel_is_shared_between_handles(scratch):
    path = scratch("chan")
    first = subetha.Channel(path, capacity=64)
    first.send(b"across")
    second = subetha.Channel.open(path, capacity=64)
    assert second.recv() == b"across"


def test_a_channel_capacity_rounds_up_and_an_open_naming_it_attaches(scratch):
    path = scratch("chan")
    first = subetha.Channel(path, capacity=1000)
    first.send(b"rounded")
    second = subetha.Channel.open(path, capacity=1000)
    assert second.recv() == b"rounded"
    assert subetha.Channel.open(path, capacity=1024).recv() is None


def test_a_channel_open_names_what_it_could_not_attach_to(scratch):
    path = scratch("chan")
    with pytest.raises(FileNotFoundError):
        subetha.Channel.open(path, capacity=64)
    made = subetha.Channel(path, capacity=64)
    with pytest.raises(subetha.LayoutMismatch, match="bytes where"):
        subetha.Channel.open(path, capacity=128)
    assert made.send(b"still here") is True


def test_an_adaptive_queue_carries_items(scratch):
    queue = subetha.AdaptiveQueue(scratch("adapt"), capacity=64)
    assert queue.send(b"first") is True
    assert queue.recv() == b"first"
    assert queue.recv() is None


def test_an_adaptive_queue_starts_as_a_ring(scratch):
    queue = subetha.AdaptiveQueue(scratch("adapt"), capacity=64)
    assert queue.shape == "ring", "streaming traffic starts on the ring"
    assert queue.shape_generation == 0


def test_an_adaptive_queue_can_be_moved_by_hand(scratch):
    queue = subetha.AdaptiveQueue(scratch("adapt"), capacity=64)
    before = queue.shape_generation

    queue.change_shape_to("work_stealing")
    assert queue.shape == "work_stealing"
    assert queue.shape_generation > before, "a change must be visible to a holder"

    # And it still carries items in the new shape.
    queue.send(b"after")
    assert queue.recv() == b"after"


def test_an_adaptive_queue_refuses_a_shape_it_does_not_have(scratch):
    queue = subetha.AdaptiveQueue(scratch("adapt"), capacity=64)
    with pytest.raises(ValueError):
        queue.change_shape_to("hash_map")


def test_an_adaptive_queue_watches_the_traffic(scratch):
    queue = subetha.AdaptiveQueue(scratch("adapt"), capacity=1024)
    for _ in range(20):
        queue.send_many([b"a", b"b", b"c", b"d"])

    average, batched = queue.traffic
    assert average >= 1, "the sizes it was sent are counted"
    assert 0.0 <= batched <= 1.0


def test_an_adaptive_queue_rounds_its_capacity_and_answers_how_much_of_a_batch_went_in(scratch):
    # Three rounds up to four, which a batch of ten overfills.
    queue = subetha.AdaptiveQueue(scratch("adapt"), capacity=3)
    sent = queue.send_many([bytes([n]) for n in range(10)])
    assert 0 < sent < 10, f"a queue of four takes part of ten, took {sent}"
    assert queue.send_many([b"more"]) == 0, "a full queue takes nothing"
    assert queue.recv_many(16) == [bytes([n]) for n in range(sent)]


def test_an_adaptive_queue_decides_for_itself_whether_to_move(scratch):
    queue = subetha.AdaptiveQueue(scratch("adapt"), capacity=1024)
    # An answer either way is fine; what matters is that asking is safe
    # and that what comes back names a shape it actually has.
    moved = queue.maybe_change_shape()
    assert moved is None or moved in ("ring", "work_stealing")
    assert queue.shape in ("ring", "work_stealing")


def test_an_adaptive_queue_keeps_the_ordering_it_was_given(scratch):
    queue = subetha.AdaptiveQueue(
        scratch("adapt"), capacity=64, ordering="global_fifo"
    )
    assert queue.ordering == "global_fifo"
    assert queue.inversions == 0
    queue.ordering = "per_producer"
    assert queue.ordering == "per_producer"


def test_an_adaptive_queue_refuses_an_ordering_it_does_not_have(scratch):
    with pytest.raises(ValueError):
        subetha.AdaptiveQueue(scratch("adapt"), capacity=64, ordering="whenever")


def test_an_adaptive_queue_refuses_a_threshold_that_is_not_a_rate(scratch):
    with pytest.raises(ValueError):
        subetha.AdaptiveQueue(scratch("adapt"), capacity=64, auto_order=-1.0)


def test_an_adaptive_queue_waits_and_gives_up(scratch):
    queue = subetha.AdaptiveQueue(scratch("adapt"), capacity=64)
    started = time.monotonic()
    assert queue.recv_for(timeout=0.1) is None
    assert time.monotonic() - started >= 0.05


def test_a_work_queue_gives_the_owner_its_own_work_back(scratch):
    queue = subetha.WorkQueue(scratch("work"), capacity=64)
    assert queue.push(b"one") is True
    assert queue.push(b"two") is True
    # The owner takes the most recent first, which is the cheap end.
    assert queue.pop() == b"two"
    assert queue.pop() == b"one"
    assert queue.pop() is None


def test_a_work_queue_is_stolen_from_the_other_end(scratch):
    queue = subetha.WorkQueue(scratch("work"), capacity=64)
    queue.push_many([b"first", b"second", b"third"])
    # A thief takes the oldest, so it does not fight the owner.
    assert queue.steal() == b"first"


def test_a_thief_reaches_a_queue_somebody_else_owns(scratch):
    path = scratch("work")
    owner = subetha.WorkQueue(path, capacity=64)
    owner.push_many([b"one", b"two"])

    thief = subetha.WorkQueue.steal_from(path)
    taken = thief.steal_many()
    assert taken, "a thief must reach the owner's work"
    assert all(item in (b"one", b"two") for item in taken)


def test_a_thief_attaching_where_no_queue_is_hears_the_file_is_not_there(scratch):
    with pytest.raises(FileNotFoundError):
        subetha.WorkQueue.steal_from(scratch("absent"))


def test_a_work_queue_with_nothing_in_it_answers_nothing(scratch):
    queue = subetha.WorkQueue(scratch("work"), capacity=64)
    assert queue.pop() is None
    assert queue.steal() is None
    assert queue.steal_many() == []


def test_a_work_queue_refuses_an_item_that_does_not_fit(scratch):
    queue = subetha.WorkQueue(scratch("work"), capacity=64)
    with pytest.raises(ValueError):
        queue.push(b"x" * (subetha.WorkQueue.max_item_size + 1))


def test_a_kv_map_says_whether_a_key_was_new(scratch):
    index = subetha.KvMap(scratch("kv"), capacity=256)
    assert index.insert(1, 10) is True, "the key was not there before"
    assert index.get(1) == 10
    assert index.insert(1, 20) is False, "this replaced what it held"
    assert index.get(1) == 20


def test_a_kv_map_reads_several_keys_at_once(scratch):
    index = subetha.KvMap(scratch("kv"), capacity=256)
    index.insert_many([(1, 10), (3, 30)])
    assert index.get_many([1, 2, 3]) == [10, None, 30]
    assert 1 in index
    assert 2 not in index
    assert len(index) == 2


def test_an_atomic_subtracts(scratch):
    counter = subetha.Atomic(scratch("counter"), init=10)
    assert counter.fetch_sub(3) == 10, "the answer is what it was before"
    assert counter.load() == 7


def test_an_atomic_taken_below_zero_wraps_to_the_top(scratch):
    counter = subetha.Atomic(scratch("counter"), init=0)
    counter.fetch_sub(1)
    assert counter.load() == 2**64 - 1, "these are unsigned, so below zero wraps"


def test_an_atomic_works_on_its_bits(scratch):
    bits = subetha.Atomic(scratch("bits"), init=0b1010)
    assert bits.fetch_or(0b0101) == 0b1010
    assert bits.load() == 0b1111
    assert bits.fetch_and(0b1100) == 0b1111
    assert bits.load() == 0b1100
    assert bits.fetch_xor(0b1111) == 0b1100
    assert bits.load() == 0b0011


def test_an_atomic_swaps_in_one_step(scratch):
    value = subetha.Atomic(scratch("value"), init=1)
    assert value.swap(99) == 1
    assert value.load() == 99


def test_a_32_bit_atomic_counts_and_wraps_at_its_width(scratch):
    path = scratch("u32")
    counter = subetha.AtomicU32(path, init=2**32 - 2)
    assert counter.fetch_add(1) == 2**32 - 2
    assert counter.fetch_add(1) == 2**32 - 1
    assert counter.load() == 0, "past thirty-two bits wraps round"
    assert counter.compare_exchange(0, 7) == 0
    assert subetha.AtomicU32.open(path).load() == 7, "a second handle sees the same value"
    assert counter.swap(3) == 7
    assert counter.fetch_or(0b100) == 3 and counter.load() == 7
    assert counter.fetch_and(0b110) == 7 and counter.load() == 6
    assert counter.fetch_xor(0b011) == 6 and counter.load() == 5
    assert counter.fetch_sub(5) == 5 and counter.load() == 0


def test_a_32_bit_atomic_refuses_a_64_bit_file(scratch):
    path = scratch("wide")
    subetha.Atomic(path, init=1)
    with pytest.raises(subetha.LayoutMismatch) as caught:
        subetha.AtomicU32.open(path)
    assert "the file's width is 8 where 4 was asked for" in str(caught.value), str(caught.value)


def test_a_flag_is_set_once_and_seen_by_every_handle(scratch):
    path = scratch("flag")
    flag = subetha.AtomicBool(path)
    assert flag.load() is False
    assert flag.swap(True) is False, "the caller that sees False come back set it"
    assert flag.swap(True) is True
    assert subetha.AtomicBool(path).load() is True, "a second create attaches and keeps the value"
    assert subetha.AtomicBool.open(path).load() is True
    flag.store(False)
    assert flag.load() is False


def test_a_load_or_store_refuses_an_order_it_cannot_take(scratch):
    for atomic in (subetha.Atomic(scratch("orders64")), subetha.AtomicU32(scratch("orders32"))):
        for order in ("release", "acq_rel"):
            with pytest.raises(ValueError, match=f"a load cannot be {order}; use relaxed, acquire or seq_cst"):
                atomic.load(order)
        for order in ("acquire", "acq_rel"):
            with pytest.raises(ValueError, match=f"a store cannot be {order}; use relaxed, release or seq_cst"):
                atomic.store(1, order)
        assert atomic.load() == 0, "a refused store writes nothing"
    flag = subetha.AtomicBool(scratch("orders-flag"))
    with pytest.raises(ValueError, match="a load cannot be release"):
        flag.load("release")
    with pytest.raises(ValueError, match="a store cannot be acquire"):
        flag.store(True, "acquire")
    assert flag.swap(True, "acq_rel") is False, "a read-modify-write takes every order"


def test_reset_remakes_an_atomic_whatever_the_file_held(scratch):
    path = scratch("remade")
    held = subetha.Atomic(path, init=41)
    held.fetch_add(1)
    # Windows refuses to remake a file any handle still maps, so the last
    # one goes first; each reset below is the only handle when it runs.
    del held
    assert subetha.Atomic.reset(path, init=5).load() == 5
    assert subetha.AtomicU32.reset(path, init=9).load() == 9, "reset lays the file out at its own width"
    assert subetha.AtomicU32.open(path).load() == 9
    assert subetha.AtomicBool.reset(path, init=True).load() is True
    assert subetha.AtomicBool.open(path).load() is True


def test_a_compare_exchange_answers_what_it_found(scratch):
    value = subetha.Atomic(scratch("value"), init=5)
    # It matched, so the new value went in and the answer is what was
    # expected.
    assert value.compare_exchange(5, 6) == 5
    assert value.load() == 6

    # It did not match, so nothing changed and the answer is what is
    # actually there, which is what a caller retries against.
    assert value.compare_exchange(5, 7) == 6
    assert value.load() == 6


def test_a_compare_exchange_loop_reaches_its_answer(scratch):
    value = subetha.Atomic(scratch("value"), init=0)
    seen = value.load()
    while True:
        found = value.compare_exchange(seen, seen + 10)
        if found == seen:
            break
        seen = found
    assert value.load() == 10


needs_tcp_bridge = pytest.mark.skipif(
    "tcp" not in subetha.transports,
    reason="this wheel was built without the tcp-bridge feature",
)


@needs_tcp_bridge
def test_a_tcp_bridge_carries_a_ring_to_another_ring(scratch):
    sending = subetha.Ring(scratch("bridge-out"), capacity=64)
    receiving = subetha.Ring(scratch("bridge-in"), capacity=64)
    producer = sending.register_producer()
    consumer = receiving.register_consumer()

    server = subetha.TcpBridgeServer(receiving, ("127.0.0.1", 0))
    sending.send_many(producer, [b"one", b"two", b"three"])

    # The reading end waits for the sending end to finish, so one of
    # them has to be on another thread.
    arrived = []
    reader = threading.Thread(target=lambda: arrived.append(server.accept_one()))
    reader.start()

    client = subetha.TcpBridgeClient(sending, server.local_addr)
    client.run(items=3)
    reader.join(timeout=30)

    assert not reader.is_alive(), "the reading end must finish once the items are in"
    assert arrived == [3]
    taken = receiving.recv_many(consumer, 10)
    assert [bytes(item).rstrip(b"\x00") for item in taken] == [b"one", b"two", b"three"]


@needs_tcp_bridge
def test_a_tcp_bridge_says_where_it_listens(scratch):
    receiving = subetha.Ring(scratch("bridge-in"), capacity=64)
    server = subetha.TcpBridgeServer(receiving, ("127.0.0.1", 0))
    host, port = server.local_addr
    assert host == "127.0.0.1"
    assert port != 0


@needs_tcp_bridge
def test_a_tcp_bridge_refuses_an_address_that_is_not_one(scratch):
    sending = subetha.Ring(scratch("bridge-out"), capacity=64)
    with pytest.raises((ValueError, OSError)):
        subetha.TcpBridgeClient(sending, ("no.such.host.invalid", 9))


needs_quic_bridge = pytest.mark.skipif(
    "quic" not in subetha.transports,
    reason="this wheel was built without the quic-bridge feature",
)


@needs_quic_bridge
def test_a_quic_bridge_carries_a_ring_to_another_ring(scratch):
    cert, key = subetha.generate_self_signed_cert("subetha.test")
    sending = subetha.Ring(scratch("quic-out"), capacity=64)
    receiving = subetha.Ring(scratch("quic-in"), capacity=64)
    producer = sending.register_producer()
    consumer = receiving.register_consumer()

    server = subetha.QuicBridgeServer(receiving, ("127.0.0.1", 0), cert, key)
    sending.send_many(producer, [b"one", b"two", b"three"])

    arrived = []
    reader = threading.Thread(target=lambda: arrived.append(server.accept_one()))
    reader.start()

    client = subetha.QuicBridgeClient(
        sending, server.local_addr, cert, "subetha.test"
    )
    client.run(items=3)
    reader.join(timeout=30)

    assert not reader.is_alive(), "the reading end must finish once the items are in"
    assert arrived == [3]
    taken = receiving.recv_many(consumer, 10)
    assert [bytes(item).rstrip(b"\x00") for item in taken] == [b"one", b"two", b"three"]


@needs_quic_bridge
def test_a_quic_certificate_comes_back_as_two_pieces(scratch):
    cert, key = subetha.generate_self_signed_cert("subetha.test")
    assert isinstance(cert, bytes) and cert
    assert isinstance(key, bytes) and key
    assert cert != key


@needs_quic_bridge
def test_a_quic_bridge_refuses_a_certificate_it_cannot_read(scratch):
    sending = subetha.Ring(scratch("quic-out"), capacity=64)
    with pytest.raises(OSError):
        subetha.QuicBridgeClient(
            sending, ("127.0.0.1", 9), b"not a certificate", "subetha.test"
        )


def test_the_wheel_says_which_transports_it_was_built_with(scratch):
    assert "sens" in subetha.transports, "the link is always built"
    for name in subetha.transports:
        assert name in ("sens", "tcp", "quic")


ITEM = 256


def drain(reader, wanted, tries=400):
    """Poll until `wanted` items have come back, or the tries run out.

    A link like this does not hand back one item per call. It answers
    with whatever it could rebuild that time round, so a reader loops.
    """
    taken = []
    for _ in range(tries):
        taken.extend(reader.poll())
        if len(taken) >= wanted:
            break
        time.sleep(0.005)
    return taken


def test_a_link_carries_items_across_the_loopback(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    writer = subetha.SensSender(("127.0.0.1", 0), reader.local_addr, max_item_size=ITEM)

    sent = [bytes([n]) * ITEM for n in range(8)]
    assert writer.send_many(sent) == 8

    taken = drain(reader, 8)
    assert taken == sent, "everything sent must come back, in order"


def test_a_link_hands_back_nothing_when_nothing_was_sent(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    assert reader.poll() == [], "an empty answer is ordinary, not a fault"
    assert reader.alive is True


def test_a_link_says_where_it_listens(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    host, port = reader.local_addr
    assert host == "127.0.0.1"
    assert port != 0, "the system picked a port, and it is readable"


def test_a_link_carries_an_item_shorter_than_the_maximum(scratch):
    # The length travels with the item, so a short one arrives short
    # rather than padded out.
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    writer = subetha.SensSender(("127.0.0.1", 0), reader.local_addr, max_item_size=ITEM)
    writer.send(b"short")
    assert drain(reader, 1) == [b"short"]


def test_a_link_refuses_an_item_larger_than_the_maximum(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    writer = subetha.SensSender(("127.0.0.1", 0), reader.local_addr, max_item_size=ITEM)
    assert writer.max_item_size == ITEM
    assert reader.max_item_size == ITEM
    with pytest.raises(ValueError):
        writer.send(b"x" * (ITEM + 1))


def test_a_link_sends_none_of_a_run_when_one_does_not_fit(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    writer = subetha.SensSender(("127.0.0.1", 0), reader.local_addr, max_item_size=ITEM)
    with pytest.raises(ValueError):
        writer.send_many([b"fine", b"x" * (ITEM + 1), b"also fine"])
    assert reader.poll() == [], "a refused run must not have sent part of itself"


def test_a_link_starts_on_the_sliding_code(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    writer = subetha.SensSender(("127.0.0.1", 0), reader.local_addr, max_item_size=ITEM)
    assert writer.code == "rlc", "the quicker code is where a clean link starts"
    assert reader.code == "rlc"
    assert writer.switches == 0


def test_a_link_can_be_pinned_to_the_block_code(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM, code="rs")
    writer = subetha.SensSender(
        ("127.0.0.1", 0), reader.local_addr, max_item_size=ITEM, code="rs"
    )
    assert writer.code == "rs"
    assert reader.code == "rs"

    # The block code groups items in eights, so a run has to fill whole
    # groups before any of it comes out.
    sent = [bytes([n]) * ITEM for n in range(16)]
    writer.send_many(sent)
    assert drain(reader, 16) == sent


def test_the_block_code_holds_a_run_shorter_than_a_group(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM, code="rs")
    writer = subetha.SensSender(
        ("127.0.0.1", 0), reader.local_addr, max_item_size=ITEM, code="rs"
    )
    writer.send_many([bytes([n]) * ITEM for n in range(4)])
    assert drain(reader, 1, tries=40) == [], "a part group waits for the rest of it"


def test_a_link_refuses_a_code_it_does_not_have(scratch):
    with pytest.raises(ValueError):
        subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM, code="turbo")


def test_a_link_refuses_an_address_that_is_not_one(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    with pytest.raises((ValueError, OSError)):
        subetha.SensSender(
            ("127.0.0.1", 0), ("no.such.host.invalid", 9), max_item_size=ITEM
        )
    assert reader.alive is True


def test_a_link_counts_what_went_on_the_wire(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    writer = subetha.SensSender(("127.0.0.1", 0), reader.local_addr, max_item_size=ITEM)
    writer.send_many([bytes([n]) * ITEM for n in range(8)])
    drain(reader, 8)

    datagrams_out, _ = writer.datagrams
    assert datagrams_out >= 8, "the wire carries the items and the extra besides"
    measured = writer.loss
    assert measured is None or 0.0 <= measured <= 1.0


def test_a_link_reports_no_loss_measurement_before_it_has_one(scratch):
    # None is not zero. Nothing has been measured yet, which is a
    # different answer from measured and nothing lost.
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    writer = subetha.SensSender(("127.0.0.1", 0), reader.local_addr, max_item_size=ITEM)
    assert writer.loss is None


def test_a_link_says_which_sender_an_item_came_from(scratch):
    reader = subetha.SensReceiver(("127.0.0.1", 0), max_item_size=ITEM)
    writer = subetha.SensSender(("127.0.0.1", 0), reader.local_addr, max_item_size=ITEM)
    writer.send(b"x" * ITEM)

    tagged = []
    for _ in range(400):
        tagged.extend(reader.poll_from())
        if tagged:
            break
        time.sleep(0.005)

    assert tagged, "the item must arrive"
    tag, item = tagged[0]
    assert item == b"x" * ITEM
    assert isinstance(tag, int)


def test_a_tower_reaches_a_value_by_its_path(scratch):
    tower = subetha.Tower(
        scratch("tower"),
        capacity=64,
        value_size=8,
        levels=[(scratch("tower-top"), 64)],
    )
    assert tower.depth == 2
    path = tower.append(b"12345678")
    assert len(path) == 2
    assert tower.get(path) == b"12345678"
    assert len(tower) == 1


def test_a_tower_one_deep_is_a_path_of_one(scratch):
    tower = subetha.Tower(scratch("tower"), capacity=64, value_size=4, levels=[])
    assert tower.depth == 1
    path = tower.append(b"abcd")
    assert len(path) == 1
    assert tower.get(path) == b"abcd"


def test_a_tower_refuses_a_path_a_level_no_longer_agrees_with(scratch):
    # This is what a tower is for. A bare index into the bottom level
    # would answer with whatever now sits there.
    tower = subetha.Tower(
        scratch("tower"),
        capacity=64,
        value_size=4,
        levels=[(scratch("tower-top"), 64)],
    )
    stale = tower.append(b"aaaa")
    # Rewrite the top place the stale path goes through.
    tower.insert_at_top(stale[0], b"bbbb")

    with pytest.raises(ValueError):
        tower.get(stale)


def test_a_tower_refuses_a_path_of_the_wrong_length(scratch):
    tower = subetha.Tower(
        scratch("tower"),
        capacity=64,
        value_size=4,
        levels=[(scratch("tower-top"), 64)],
    )
    with pytest.raises(ValueError):
        tower.get([0])


def test_a_tower_refuses_a_value_of_the_wrong_size(scratch):
    tower = subetha.Tower(scratch("tower"), capacity=64, value_size=4, levels=[])
    assert tower.value_size == 4
    with pytest.raises(ValueError):
        tower.append(b"too long for this")


def test_a_tower_stores_and_reads_a_run_in_one_crossing(scratch):
    tower = subetha.Tower(
        scratch("tower"),
        capacity=64,
        value_size=4,
        levels=[(scratch("tower-top"), 64)],
    )
    paths = tower.append_many([b"aaaa", b"bbbb", b"cccc"])
    assert len(paths) == 3
    assert tower.get_many(paths) == [b"aaaa", b"bbbb", b"cccc"]


def test_a_tower_is_shared_between_handles(scratch):
    path, top = scratch("tower"), scratch("tower-top")
    first = subetha.Tower(path, capacity=64, value_size=4, levels=[(top, 64)])
    reached_by = first.append(b"abcd")

    second = subetha.Tower.open(path, capacity=64, value_size=4, levels=[(top, 64)])
    assert second.get(reached_by) == b"abcd"


def test_a_qos_policy_starts_where_it_was_asked_to(scratch):
    wants = subetha.QosPolicy(
        durability="persistent", reliability="reliable", keep_last=None, max_latency=0.5
    )
    assert wants.durability == "persistent"
    assert wants.reliability == "reliable"
    assert wants.keep_last is None, "None means keep everything there is room for"
    assert wants.max_latency == 0.5
    assert wants.ordering == "per_producer", "ordering is set on its own, never guessed"


def test_a_qos_policy_can_be_changed_after_it_is_made(scratch):
    wants = subetha.QosPolicy()
    wants.durability = "transient"
    wants.reliability = "reliable"
    wants.keep_last = 16
    wants.max_latency = 0.25
    wants.ordering = "global_fifo"

    assert wants.durability == "transient"
    assert wants.reliability == "reliable"
    assert wants.keep_last == 16
    assert wants.max_latency == 0.25
    assert wants.ordering == "global_fifo"


def test_the_named_qos_policies_differ_where_they_should(scratch):
    assert subetha.QosPolicy.streaming().durability == "volatile"
    assert subetha.QosPolicy.streaming().reliability == "best_effort"
    assert subetha.QosPolicy.reliable_pubsub().reliability == "reliable"
    assert subetha.QosPolicy.persistent_log().durability == "persistent"
    assert subetha.QosPolicy.persistent_log().keep_last is None


def test_a_qos_policy_refuses_a_setting_it_does_not_have(scratch):
    with pytest.raises(ValueError):
        subetha.QosPolicy(durability="forever")
    with pytest.raises(ValueError):
        subetha.QosPolicy(reliability="mostly")
    wants = subetha.QosPolicy()
    with pytest.raises(ValueError):
        wants.ordering = "whatever_arrives"
    with pytest.raises(ValueError):
        wants.max_latency = -1.0


def test_a_qos_snapshot_says_where_the_bytes_should_live(scratch):
    wants = subetha.QosPolicy(durability="persistent")
    taken = wants.snapshot()
    assert taken.durability == "persistent"
    # Bytes that must outlive the process belong in a file.
    assert taken.recommends_locale_change("anon") == "file"
    assert taken.recommends_locale_change("file") is None, "already where it should be"


def test_a_qos_snapshot_says_when_the_ordering_must_change(scratch):
    wants = subetha.QosPolicy()
    wants.ordering = "global_fifo"
    taken = wants.snapshot()
    assert taken.recommends_ordering_change("per_producer") == "global_fifo"
    assert taken.recommends_ordering_change("global_fifo") is None


def test_a_qos_snapshot_does_not_move_when_the_policy_does(scratch):
    wants = subetha.QosPolicy(durability="volatile")
    taken = wants.snapshot()
    wants.durability = "persistent"
    assert taken.durability == "volatile", "a snapshot is one moment, not a view"
    assert wants.durability == "persistent"


def test_a_qos_snapshot_refuses_a_place_it_does_not_know(scratch):
    taken = subetha.QosPolicy().snapshot()
    with pytest.raises(ValueError):
        taken.recommends_locale_change("somewhere_else")


def test_a_topology_counts_who_sends_to_whom(scratch):
    seen = subetha.TopologyMap(scratch("topo"), participants=8)
    assert seen.participants == 8
    assert seen.total_sends == 0

    seen.record_send(0, 1)
    seen.record_send(0, 2)
    assert seen.total_sends == 2
    assert seen.fan_out(0) == 2, "one sender reaching two places"
    assert seen.fan_in(1) == 1
    assert seen.fan_out(1) == 0


def test_a_topology_records_a_run_in_one_crossing(scratch):
    seen = subetha.TopologyMap(scratch("topo"), participants=8)
    assert seen.record_many([(0, 1), (0, 2), (0, 3)]) == 3
    assert seen.fan_out(0) == 3
    assert seen.total_sends == 3


def test_a_topology_names_the_busiest_participants(scratch):
    seen = subetha.TopologyMap(scratch("topo"), participants=8)
    seen.record_many([(0, 1), (0, 2), (0, 3), (4, 1)])
    sender, reach = seen.busiest_sender
    assert sender == 0
    assert reach == 3
    receiver, reached_from = seen.busiest_receiver
    assert receiver == 1
    assert reached_from == 2


def test_a_topology_reads_one_sender_reaching_many_as_a_tree(scratch):
    seen = subetha.TopologyMap(scratch("topo"), participants=16)
    seen.record_many([(0, n) for n in range(1, 8)])
    assert seen.recommend() == "broadcast_tree"
    assert seen.broadcast_root == 0


def test_a_topology_reads_a_pair_as_point_to_point(scratch):
    seen = subetha.TopologyMap(scratch("topo"), participants=8)
    seen.record_many([(0, 1)] * 20)
    assert seen.recommend() == "point_to_point"


def test_a_topology_reads_many_reaching_many_as_a_mesh(scratch):
    seen = subetha.TopologyMap(scratch("topo"), participants=16)
    seen.record_many([(src, dst) for src in range(6) for dst in range(6) if src != dst])
    assert seen.recommend() == "all_to_all_mesh"


def test_publishing_a_recommendation_makes_every_holder_agree(scratch):
    path = scratch("topo")
    seen = subetha.TopologyMap(path, participants=16)
    seen.record_many([(0, n) for n in range(1, 8)])
    before = seen.recommendation_epoch

    published = seen.publish_recommendation()
    assert published == "broadcast_tree"
    assert seen.recommendation_epoch > before, "publishing must be visible as a step"

    other = subetha.TopologyMap.open(path, participants=16)
    assert other.published_recommendation() == "broadcast_tree"


def test_a_topology_recommendation_is_not_published_by_reading_it(scratch):
    seen = subetha.TopologyMap(scratch("topo"), participants=16)
    seen.record_many([(0, n) for n in range(1, 8)])
    assert seen.recommend() == "broadcast_tree"
    assert seen.recommendation_epoch == 0, "reading must not publish"


def test_a_topology_takes_its_own_thresholds(scratch):
    # A low threshold calls a small spread a tree; the default would not.
    strict = subetha.TopologyMap(scratch("strict"), 8, fan_out_threshold=2)
    strict.record_many([(0, 1), (0, 2)])
    assert strict.recommend() == "broadcast_tree"

    lax = subetha.TopologyMap(scratch("lax"), 8, fan_out_threshold=6)
    lax.record_many([(0, 1), (0, 2)])
    assert lax.recommend() != "broadcast_tree"


def test_a_topology_refuses_a_participant_it_does_not_have(scratch):
    seen = subetha.TopologyMap(scratch("topo"), participants=4)
    with pytest.raises(IndexError):
        seen.record_send(0, 99)


def test_a_graph_holds_nodes_and_the_edges_between_them(scratch):
    shape = subetha.Graph(scratch("graph"), max_nodes=16, max_edges=32)
    first = shape.add_node(100)
    second = shape.add_node(200)
    edge = shape.add_edge(first, second, 7)

    assert shape.node_count == 2
    assert shape.edge_count == 1
    assert shape.node_value(first) == 100
    assert shape.neighbors(first) == [(edge, second, 7)]
    assert shape.neighbors(second) == [], "the edge goes one way"


def test_a_graph_adds_many_in_one_crossing(scratch):
    shape = subetha.Graph(scratch("graph"), max_nodes=16, max_edges=32)
    nodes = shape.add_nodes([10, 20, 30])
    assert len(nodes) == 3
    edges = shape.add_edges([(nodes[0], nodes[1], 1), (nodes[0], nodes[2], 2)])
    assert len(edges) == 2
    assert shape.out_degree(nodes[0]) == 2


def test_a_graph_removes_an_edge_and_leaves_the_nodes(scratch):
    shape = subetha.Graph(scratch("graph"), max_nodes=16, max_edges=32)
    first = shape.add_node(1)
    second = shape.add_node(2)
    edge = shape.add_edge(first, second, 7)

    assert shape.remove_edge(first, edge) == 7
    assert shape.neighbors(first) == []
    assert shape.node_value(first) == 1
    assert shape.remove_edge(first, edge) is None


def test_a_graph_says_nothing_about_a_node_it_does_not_have(scratch):
    shape = subetha.Graph(scratch("graph"), max_nodes=16, max_edges=32)
    assert shape.node_value(99) is None
    assert shape.out_degree(99) is None
    assert shape.neighbors(99) == []


def test_a_graph_fills_up(scratch):
    shape = subetha.Graph(scratch("graph"), max_nodes=2, max_edges=2)
    shape.add_node(1)
    shape.add_node(2)
    assert shape.max_nodes == 2
    with pytest.raises(OSError):
        shape.add_node(3)


def test_a_graph_is_shared_between_handles(scratch):
    path = scratch("graph")
    first = subetha.Graph(path, max_nodes=16, max_edges=32)
    node = first.add_node(42)
    second = subetha.Graph.open(path, max_nodes=16, max_edges=32)
    assert second.node_value(node) == 42


def test_a_universal_set_holds_what_was_put_in(scratch):
    values = subetha.Universal(scratch("uni"), capacity=64)
    assert values.insert(7) is True, "a value the set did not hold is new"
    assert values.insert(7) is False, "and a set holds it once"
    assert 7 in values
    assert 8 not in values
    assert len(values) == 1
    assert values.snapshot() == [7]


def test_a_universal_set_starts_as_a_list(scratch):
    values = subetha.Universal(scratch("uni"), capacity=64)
    assert values.insert_many([1, 2, 3]) == 3
    assert values.strategy == "list", "a small set is quicker to walk than to hash"
    assert sorted(values.snapshot()) == [1, 2, 3]


def test_adding_a_run_counts_only_the_values_that_were_new(scratch):
    values = subetha.Universal(scratch("uni"), capacity=64)
    values.insert_many([1, 2, 3])
    assert values.insert_many([2, 3, 4, 4]) == 1
    assert sorted(values.snapshot()) == [1, 2, 3, 4]


def test_a_universal_set_can_be_moved_to_a_map(scratch):
    values = subetha.Universal(scratch("uni"), capacity=64)
    values.insert_many([1, 2, 3])
    before = values.version

    values.migrate_to("map")
    assert values.strategy == "map"
    assert values.version > before, "a move must be visible as a step"
    assert sorted(values.snapshot()) == [1, 2, 3], "and must carry everything over"
    assert 2 in values
    assert values.insert(2) is False, "a value carried across is not new on the map"


def test_a_universal_set_moved_where_it_already_is_does_nothing(scratch):
    values = subetha.Universal(scratch("uni"), capacity=64)
    values.insert(1)
    before = values.version
    values.migrate_to("list")
    assert values.version == before


def test_a_universal_set_generation_only_moves_on_a_wrap(scratch):
    # The generation is not the version; it steps only when the version
    # runs out of room, so a holder compares the pair.
    values = subetha.Universal(scratch("uni"), capacity=64)
    values.insert(1)
    values.migrate_to("map")
    assert values.version >= 1
    assert values.generation == 0


def test_a_universal_set_refuses_a_way_of_storing_it_does_not_have(scratch):
    values = subetha.Universal(scratch("uni"), capacity=64)
    with pytest.raises(ValueError):
        values.migrate_to("btree")


def test_a_universal_set_counts_what_it_has_been_asked(scratch):
    values = subetha.Universal(scratch("uni"), capacity=64)
    values.insert_many([1, 2])
    values.contains_many([1, 2, 3])
    inserts, lookups = values.op_counts
    assert inserts >= 2
    assert lookups >= 3


def test_clearing_a_universal_set_empties_it(scratch):
    values = subetha.Universal(scratch("uni"), capacity=64)
    values.insert_many([1, 2, 3])
    values.migrate_to("map")
    before = values.version
    values.clear()
    assert len(values) == 0
    assert 1 not in values
    assert values.version > before, "a clear steps the version as a migration does"
    assert values.strategy == "map", "and keeps the way of storing it had"
    assert values.insert(1) is True, "a value cleared away is new again"


def test_a_universal_set_is_shared_between_handles(scratch):
    path = scratch("uni")
    first = subetha.Universal(path, capacity=64)
    first.insert(9)
    second = subetha.Universal.open(path, capacity=64)
    assert 9 in second
    assert second.insert(9) is False, "the value another handle added is not new here"


def test_unlinking_a_universal_set_removes_its_state_and_both_tables(scratch):
    path = scratch("uni")
    values = subetha.Universal(path, capacity=64)
    values.insert_many([1, 2, 3])
    values.migrate_to("map")
    values.clear()
    del values
    gc.collect()
    assert subetha.Universal.unlink(path) == {"removed": 3, "missing": 0}
    assert not os.path.exists(path + ".state.bin")
    assert subetha.Universal.unlink(path) == {"removed": 0, "missing": 1}
    fresh = subetha.Universal(path, capacity=64)
    assert len(fresh) == 0, "a set made after the unlink starts empty"


def test_a_laned_map_writes_through_a_claimed_lane(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=4)
    assert index.lanes == 4
    with index.claim_lane() as lane:
        assert isinstance(lane.index, int)
        assert lane.insert(1, 10) is None
    assert index.get(1) == 10
    assert index.lane_of(1) is not None


def test_a_laned_map_gives_the_lane_back_at_the_end_of_a_block(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=2)
    with index.claim_lane():
        assert index.held_lanes == 1
    assert index.held_lanes == 0


def test_a_laned_map_runs_out_of_lanes(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=2)
    first = index.claim_lane()
    second = index.claim_lane()
    with pytest.raises(subetha.Contended):
        index.claim_lane()
    first.release()
    # A lane given back can be taken again.
    third = index.claim_lane()
    assert third.held is True
    second.release()
    third.release()


def test_a_laned_map_claims_the_lane_a_key_lives_in(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=4)
    with index.claim_lane() as lane:
        lane.insert(1, 10)
        first_lane = lane.index

    with index.claim_lane_for(1) as lane:
        assert lane.index == first_lane, "a key keeps its lane"
        assert lane.insert(1, 20) == 10


def test_claiming_a_lane_for_an_unknown_key_is_refused(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=4)
    with pytest.raises(KeyError):
        index.claim_lane_for(999)


def test_writing_a_key_through_the_wrong_lane_names_the_right_one(scratch):
    # A key belongs to one lane for its whole life, because a lane is a
    # separate tree. Removing it elsewhere would report a row gone that
    # no reader has stopped seeing, so it is refused by name.
    index = subetha.LanedMap(scratch("laned"), lanes=4)
    with index.claim_lane() as first:
        first.insert(1, 10)
        owning_lane = first.index

    claims = [index.claim_lane() for _ in range(index.lanes)]
    other = next(claim for claim in claims if claim.index != owning_lane)
    with pytest.raises(subetha.WrongLane):
        other.remove(1)
    for claim in claims:
        claim.release()


def test_a_laned_map_reads_without_claiming_a_lane(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=4)
    with index.claim_lane() as lane:
        lane.insert_many([(1, 10), (2, 20)])
    with index.claim_lane() as lane:
        lane.insert(3, 30)

    assert index.get_many([1, 2, 3, 4]) == [10, 20, 30, None]


def test_a_laned_map_scan_merges_every_lane_in_key_order(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=4)
    with index.claim_lane() as lane:
        lane.insert_many([(1, 10), (5, 50)])
    with index.claim_lane() as lane:
        lane.insert_many([(2, 20), (4, 40)])

    with index.pin() as reader:
        assert reader.scan() == [(1, 10), (2, 20), (4, 40), (5, 50)]
        assert reader.scan(low=2, high=4) == [(2, 20), (4, 40)]


def test_a_laned_map_scan_sees_one_unchanging_view(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=4)
    with index.claim_lane() as lane:
        lane.insert(1, 10)

    with index.pin() as reader:
        with index.claim_lane() as lane:
            lane.insert(2, 20)
        assert reader.get(2) is None, "written after the pin, so invisible to it"
        assert index.get(2) == 20


def test_a_laned_pin_keeps_its_map_alive(scratch):
    reader = subetha.LanedMap(scratch("laned"), lanes=2).pin()
    gc.collect()
    assert reader.scan() == []
    assert isinstance(reader.epoch, int)


def test_a_lane_claim_keeps_its_map_alive(scratch):
    lane = subetha.LanedMap(scratch("laned"), lanes=2).claim_lane()
    gc.collect()
    assert lane.insert(1, 10) is None
    assert isinstance(lane.index, int)


def test_a_lane_claim_given_back_refuses_to_write(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=2)
    lane = index.claim_lane()
    lane.release()
    assert lane.held is False
    with pytest.raises(ValueError):
        lane.insert(1, 10)
    # Twice is harmless.
    lane.release()


def test_a_laned_map_removes_and_sweeps(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=2)
    with index.claim_lane() as lane:
        lane.insert_many([(1, 10), (2, 20)])
    with index.claim_lane_for(1) as lane:
        assert lane.remove(1) == 10
    assert index.get(1) is None

    assert index.sweep() >= 1
    assert index.get(2) == 20


def test_a_laned_map_sweep_with_nothing_to_take_answers_zero(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=2)
    with index.claim_lane() as lane:
        lane.insert(1, 10)
    assert index.sweep() == 0


def test_a_laned_map_reports_lanes_nobody_is_holding_any_more(scratch):
    index = subetha.LanedMap(scratch("laned"), lanes=2)
    with index.claim_lane():
        # This process is alive and holding it, so nothing is reaped.
        assert index.reap_dead_claims() == 0
        assert index.held_lanes == 1


def test_a_laned_map_is_shared_between_handles(scratch):
    directory = scratch("laned")
    first = subetha.LanedMap(directory, lanes=2)
    with first.claim_lane() as lane:
        lane.insert(7, 70)

    second = subetha.LanedMap.open(directory, lanes=2)
    assert second.get(7) == 70


def versioned_index(scratch, name="vindex"):
    return subetha.VersionedIndex(scratch(name), scratch(name + "-epochs"), chunk_bits=6)


def test_a_versioned_index_keeps_every_version_a_pin_can_still_see(scratch):
    index = versioned_index(scratch)
    with index.pin() as pin, index.begin() as ticket:
        for key in range(10):
            index.write(key, key * 10, ticket, pin)
    with index.pin() as before:
        with index.pin() as pin, index.begin() as ticket:
            index.write(1, 11, ticket, pin)
        assert index.get_at(1, before) == 10, "a pin keeps the version it saw"
    with index.pin() as after:
        assert index.get_at(1, after) == 11
        assert index.get_at(99, after) is None


def test_a_version_born_after_the_pin_is_a_write_conflict_naming_its_epoch(scratch):
    index = versioned_index(scratch)
    with index.pin() as pin, index.begin() as ticket:
        index.write(1, 10, ticket, pin)
    stale = index.pin()
    with index.pin() as pin, index.begin() as ticket:
        won = ticket.epoch
        index.write(1, 11, ticket, pin)
    with index.begin() as late:
        with pytest.raises(subetha.WriteConflict) as refused:
            index.write(1, 12, late, stale)
        assert refused.value.epoch == won
        # The refused statement reads its own writes, and takes them back
        # before it publishes.
        index.write(2, 22, late, stale)
        assert index.get_at(2, stale, own=late) == 22
        assert index.get_at(2, stale) is None
        assert index.void_epoch(late.epoch) == 1
    stale.release()
    with index.pin() as now:
        assert index.get_at(1, now) == 11
        assert index.get_at(2, now) is None


def test_void_takes_one_version_back_and_revives_what_it_superseded(scratch):
    index = versioned_index(scratch)
    with index.pin() as pin, index.begin() as ticket:
        index.write(1, 10, ticket, pin)
        index.write(2, 20, ticket, pin)
    with index.pin() as pin, index.begin() as ticket:
        index.write(1, 11, ticket, pin)
        index.write(2, None, ticket, pin)
        assert index.get_at(2, pin, own=ticket) is None, "its own deletion"
        assert index.void(1, ticket.epoch) is True
        assert index.void(1, ticket.epoch) is False, "nothing left to take back"
        assert index.void(2, ticket.epoch) is True
        assert index.get_at(1, pin, own=ticket) == 10
    with index.pin() as now:
        assert index.get_at(2, now) == 20, "a voided deletion leaves the key live"


def test_a_range_resumed_past_its_cursor_reaches_every_key_once(scratch):
    index = versioned_index(scratch)
    with index.pin() as pin, index.begin() as ticket:
        for key in range(10):
            index.write(key, key + 100, ticket, pin)
    with index.pin() as pin:
        seen = []
        low = None
        pages = 0
        while True:
            rows, cursor = index.range_at(pin, low=low, limit=3)
            seen.extend(rows)
            pages += 1
            if cursor is None:
                break
            low = cursor + 1
        assert seen == [(key, key + 100) for key in range(10)]
        assert pages == 4, "three keys a page, then the last one"
        assert index.range_at(pin, low=2, high=4) == ([(2, 102), (3, 103), (4, 104)], None)
        assert index.range_at(pin, low=7, high=3) == ([], None), "crossed bounds read as no rows"


def test_changed_since_sees_another_epochs_change_and_not_its_own(scratch):
    index = versioned_index(scratch)
    with index.pin() as pin, index.begin() as ticket:
        index.write(5, 50, ticket, pin)
    since = index.now
    with index.pin() as pin, index.begin() as ticket:
        index.write(7, 70, ticket, pin)
    with index.pin() as pin:
        assert index.changed_since(pin, since, low=0, high=9)
        assert not index.changed_since(pin, since, low=8, high=9)
    with index.pin() as pin, index.begin() as ticket:
        mid = index.now
        index.write(9, 90, ticket, pin)
        assert not index.changed_since(pin, mid, low=8, high=9, own=ticket)
        assert index.changed_since(pin, mid, low=8, high=9)


def test_a_versioned_index_sweep_drops_only_what_no_pin_can_see(scratch):
    index = versioned_index(scratch)
    for value in (10, 20):
        with index.pin() as pin, index.begin() as ticket:
            index.write(1, value, ticket, pin)
    holding = index.pin()
    with index.pin() as pin, index.begin() as ticket:
        index.write(1, 30, ticket, pin)
    assert index.version_count() == 3
    assert index.sweep() == 1, "the version the held pin cannot see"
    assert index.get_at(1, holding) == 20
    holding.release()
    assert index.sweep() == 1
    assert index.version_count() == 1
    assert index.live_count() == 1


def test_a_dead_writers_epoch_is_never_seen_and_is_taken_back_whole(scratch):
    path, epochs_path = scratch("vindex"), scratch("vindex-epochs")
    index = subetha.VersionedIndex(path, epochs_path, chunk_bits=6)
    with index.pin() as pin, index.begin() as ticket:
        index.write(1, 10, ticket, pin)
    # The writer exits without running a finalizer, so its ticket is never
    # published: a statement that will never finish.
    writer = (
        "import os, sys, subetha\n"
        "index = subetha.VersionedIndex.open(sys.argv[1], sys.argv[2])\n"
        "pin = index.pin()\n"
        "ticket = index.begin()\n"
        "for key in range(20):\n"
        "    index.write(key, key + 5000, ticket, pin)\n"
        "os._exit(0)\n"
    )
    subprocess.run([sys.executable, "-c", writer, path, epochs_path], check=True)
    dead = index.dead_tickets()
    assert len(dead) == 1
    with index.pin() as pin:
        assert pin.epoch < dead[0], "a pin sits below the dead ticket"
        assert index.get_at(2, pin) is None
        assert index.get_at(1, pin) == 10
    assert index.void_epoch(dead[0]) == 20
    assert index.free_dead_ticket(dead[0]) is True
    with index.pin() as pin, index.begin() as ticket:
        index.write(50, 500, ticket, pin)
    with index.pin() as pin:
        assert pin.epoch > dead[0], "the dead epoch is in the past"
        assert index.get_at(2, pin) is None
        assert index.get_at(1, pin) == 10
    assert index.live_count() == 2


def test_a_published_ticket_and_a_released_pin_refuse_further_use(scratch):
    index = versioned_index(scratch)
    ticket = index.begin()
    pin = index.pin()
    assert ticket.held and pin.held
    ticket.publish()
    pin.release()
    assert not ticket.held and not pin.held
    with pytest.raises(ValueError):
        ticket.epoch
    with pytest.raises(ValueError):
        pin.epoch
    with index.pin() as live:
        with pytest.raises(ValueError):
            index.write(1, 1, ticket, live)
    # Twice is harmless.
    ticket.publish()
    pin.release()


def test_a_versioned_index_is_shared_between_handles(scratch):
    path, epochs_path = scratch("vindex"), scratch("vindex-epochs")
    first = subetha.VersionedIndex(path, epochs_path, chunk_bits=6)
    with first.pin() as pin, first.begin() as ticket:
        first.write(4, 40, ticket, pin)
    second = subetha.VersionedIndex.open(path, epochs_path)
    with second.pin() as pin:
        assert second.get_at(4, pin) == 40
    with pytest.raises(subetha.LayoutMismatch):
        subetha.VersionedIndex.open(path, epochs_path, max_pins=8)
    with pytest.raises(ValueError):
        subetha.VersionedIndex(scratch("bad"), scratch("bad-epochs"), chunk_bits=3)
    with first as same:
        assert same is first
    assert "VersionedIndex" in repr(first)


def test_unlinking_a_versioned_index_removes_its_files_and_leaves_the_epochs(scratch):
    path, epochs_path = scratch("vindex"), scratch("vindex-epochs")
    index = subetha.VersionedIndex(path, epochs_path, chunk_bits=6)
    with index.pin() as pin, index.begin() as ticket:
        for key in range(100):
            index.write(key, key * 10, ticket, pin)
    del index, pin, ticket
    gc.collect()
    stem = os.path.basename(path)
    named = [f for f in os.listdir(os.path.dirname(path)) if f == stem or f.startswith(stem + ".")]
    assert len(named) >= 2, "an index is its main file and the chunk files beside it"
    assert subetha.VersionedIndex.unlink(path) == {"removed": len(named), "missing": 0}
    left = [f for f in os.listdir(os.path.dirname(path)) if f == stem or f.startswith(stem + ".")]
    assert left == []
    assert os.path.exists(epochs_path), "the epochs file stays for whoever else stamps through it"
    assert subetha.VersionedIndex.unlink(path) == {"removed": 0, "missing": 1}


def test_an_index_pin_keeps_its_index_alive(scratch):
    pin = versioned_index(scratch).pin()
    ticket = versioned_index(scratch, "other").begin()
    gc.collect()
    assert isinstance(pin.epoch, int) and isinstance(ticket.epoch, int)
    assert "IndexPin" in repr(pin) and "IndexTicket" in repr(ticket)


def test_a_versioned_map_reads_back_what_was_put_in(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    assert index.insert(7, 100) is None, "the key held nothing before"
    assert index.get(7) == 100
    assert index.insert(7, 200) == 100, "an insert answers what the key held"
    assert index.get(7) == 200
    assert index.get(8) is None


def test_a_versioned_map_scan_sees_one_unchanging_view(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    index.insert_many([(1, 10), (2, 20), (3, 30)])

    with index.pin() as reader:
        # Written after the pin, so the scan must not pick it up.
        index.insert(4, 40)
        assert reader.scan() == [(1, 10), (2, 20), (3, 30)]
        assert reader.get(4) is None
        assert index.get(4) == 40, "a reader without a pin sees it"


def test_a_versioned_map_scan_takes_both_ends(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    index.insert_many([(n, n * 10) for n in range(10)])
    with index.pin() as reader:
        assert reader.scan(low=3, high=5) == [(3, 30), (4, 40), (5, 50)]
        assert reader.scan(low=8) == [(8, 80), (9, 90)]
        assert reader.scan(high=1) == [(0, 0), (1, 10)]


def test_a_versioned_map_scan_says_where_to_carry_on_from(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    index.insert_many([(n, n) for n in range(10)])
    with index.pin() as reader:
        first, cursor = reader.scan_from(limit=4)
        assert len(first) == 4
        assert cursor == 3
        rest, _ = reader.scan_from(low=cursor + 1)
        assert [key for key, _ in rest] == [4, 5, 6, 7, 8, 9]


def test_a_removed_key_stays_visible_to_a_reader_that_came_first(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    index.insert(1, 10)
    with index.pin() as reader:
        assert index.remove(1) == 10
        assert index.get(1) is None, "a later reader sees it gone"
        assert reader.get(1) == 10, "the pinned reader still sees it"


def test_removing_a_key_that_holds_nothing_answers_nothing(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    assert index.remove(99) is None


def test_sweeping_a_versioned_map_takes_the_removed_entries_away(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    index.insert_many([(1, 10), (2, 20)])
    index.remove(1)
    assert len(index) == 2, "a removed entry is still held until it is swept"

    swept = index.sweep()
    assert swept >= 1
    assert len(index) == 1
    assert index.get(2) == 20


def test_a_sweep_under_a_pin_leaves_what_the_pin_can_see(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    index.insert(1, 10)
    with index.pin() as reader:
        index.remove(1)
        # Nothing can be taken while the pin still reaches it, and that
        # is an ordinary answer of zero rather than a failure.
        assert index.sweep() == 0
        assert reader.get(1) == 10, "a sweep must not take what a live pin reaches"


def test_a_sweep_with_nothing_to_take_answers_zero(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    index.insert(1, 10)
    assert index.sweep() == 0, "nothing has been removed, so nothing goes"


def test_voiding_an_epoch_undoes_the_writes_to_a_map_stamped_at_it(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    index.insert_at(1, 10, 1)
    index.insert_at(2, 20, 2)
    assert index.get(2) == 20

    touched = index.void_epoch(2)
    assert touched >= 1
    assert index.get(2) is None, "the write stamped at that epoch is undone"
    assert index.get(1) == 10, "the one stamped elsewhere is untouched"


def test_a_map_pin_reports_its_epoch_and_can_be_given_back(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    index.insert(1, 10)
    reader = index.pin()
    assert isinstance(reader.epoch, int)
    assert reader.held is True
    reader.release()
    assert reader.held is False
    with pytest.raises(ValueError):
        reader.get(1)


def test_a_map_pin_keeps_its_map_alive(scratch):
    reader = subetha.VersionedMap(
        scratch("vmap"), 256, scratch("vmap-epochs")
    ).pin()
    gc.collect()
    assert reader.get(1) is None
    assert reader.scan() == []


def test_a_versioned_map_reads_several_keys_at_once(scratch):
    index = subetha.VersionedMap(scratch("vmap"), 256, scratch("vmap-epochs"))
    index.insert_many([(1, 10), (3, 30)])
    assert index.get_many([1, 2, 3]) == [10, None, 30]


def test_a_versioned_map_is_shared_between_handles(scratch):
    path, epochs = scratch("vmap"), scratch("vmap-epochs")
    first = subetha.VersionedMap(path, 256, epochs)
    first.insert(5, 50)
    second = subetha.VersionedMap.open(path, 256, epochs)
    assert second.get(5) == 50


def test_a_chain_gives_a_reader_the_value_as_it_stood(scratch):
    chain = subetha.VersionChain(scratch("chain"), capacity=8)
    chain.push(10, b"first")
    chain.push(20, b"second")

    assert chain.read_at(15) == b"first", "a reader at 15 must not see the later write"
    assert chain.read_at(25) == b"second"
    assert chain.current == (20, b"second")


def test_a_chain_has_nothing_before_its_first_write(scratch):
    chain = subetha.VersionChain(scratch("chain"), capacity=8)
    assert chain.current is None
    assert chain.read_at(100) is None
    chain.push(10, b"value")
    assert chain.read_at(5) is None, "a reader from before the first write sees nothing"


def test_a_chain_refuses_a_version_that_goes_backwards(scratch):
    chain = subetha.VersionChain(scratch("chain"), capacity=8)
    chain.push(20, b"later")
    with pytest.raises(ValueError):
        chain.push(10, b"earlier")


def test_a_chain_fills_up(scratch):
    chain = subetha.VersionChain(scratch("chain"), capacity=2)
    chain.push(1, b"a")
    chain.push(2, b"b")
    assert len(chain) == 2
    with pytest.raises(OSError):
        chain.push(3, b"c")


def test_clearing_a_chain_forgets_every_version(scratch):
    chain = subetha.VersionChain(scratch("chain"), capacity=8)
    chain.push(1, b"a")
    chain.push(2, b"b")
    chain.clear()
    assert len(chain) == 0
    assert chain.current is None
    # And the chain takes writes again from any version.
    chain.push(1, b"again")
    assert chain.current == (1, b"again")


def test_a_chain_refuses_a_value_that_does_not_fit(scratch):
    chain = subetha.VersionChain(scratch("chain"), capacity=8)
    with pytest.raises(ValueError):
        chain.push(1, b"x" * (subetha.VersionChain.max_value_bytes + 1))


def test_a_chain_is_shared_between_handles(scratch):
    path = scratch("chain")
    first = subetha.VersionChain(path, capacity=8)
    first.push(5, b"shared")
    second = subetha.VersionChain.open(path, capacity=8)
    assert second.current == (5, b"shared")


def test_a_versioned_slab_reads_back_what_was_written(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    slab.set(0, b"value")
    assert slab.get(0) == b"value"
    assert slab.get(1) is None, "a slot nothing wrote holds nothing"


def test_a_versioned_slab_keeps_what_a_pinned_reader_can_see(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    slab.set(0, b"before")

    with slab.pin() as reader:
        # Written after the pin was taken, so invisible to this reader.
        slab.set(0, b"after")
        assert reader.get(0) == b"before"
        assert slab.get(0) == b"after", "a reader without a pin sees the new value"


def test_a_pin_reads_several_slots_at_one_epoch(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    slab.set(0, b"a")
    slab.set(2, b"c")
    with slab.pin() as reader:
        assert reader.get_many([0, 1, 2]) == [b"a", None, b"c"]


def test_a_pin_reports_the_epoch_it_fixed(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    with slab.pin() as reader:
        assert isinstance(reader.epoch, int)
        assert reader.held is True


def test_a_pin_given_back_refuses_to_read(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    slab.set(0, b"value")
    reader = slab.pin()
    reader.release()
    assert reader.held is False
    with pytest.raises(ValueError):
        reader.get(0)
    # Twice is harmless.
    reader.release()


def test_a_pin_keeps_its_slab_alive(scratch):
    reader = subetha.VersionedSlab(
        scratch("vslab"), 8, scratch("vslab-epochs")
    ).pin()
    # The slab object is unreferenced now. The pin holds it, so
    # collecting must not take the memory it reads from out from under it.
    gc.collect()
    assert reader.get(0) is None


def test_a_retired_slot_reads_as_empty_but_keeps_its_history(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    slab.set(0, b"value")
    assert slab.retire(0) == b"value"
    assert slab.get(0) is None

    history = slab.history(0)
    assert len(history) == 1
    value, born, died = history[0]
    assert value == b"value"
    assert died is not None, "a retired version records when it stopped being current"
    assert born < died


def test_a_slot_history_is_newest_first(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    slab.set(0, b"old")
    slab.set(0, b"new")
    history = slab.history(0)
    assert [value for value, _, _ in history] == [b"new", b"old"]
    assert history[0][2] is None, "the current version has not stopped being current"


def test_a_slot_keeps_only_as_many_versions_as_its_depth(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    for n in range(subetha.VersionedSlab.depth + 3):
        slab.set(0, f"v{n}".encode())
    assert len(slab.history(0)) <= subetha.VersionedSlab.depth


def test_sweeping_a_slot_drops_what_nothing_can_see(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    slab.set(0, b"old")
    slab.set(0, b"new")
    assert len(slab.history(0)) == 2

    dropped = slab.sweep_slot(0)
    assert dropped >= 1, "the superseded version nothing holds must go"
    assert slab.get(0) == b"new", "and the current one must stay"


def test_voiding_an_epoch_undoes_the_writes_stamped_at_it(scratch):
    # Not reclamation: it rolls back a half-finished change, so the
    # value the undone write superseded becomes current again.
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    slab.set_at(0, b"old", 1)
    slab.set_at(1, b"old", 1)
    slab.set_at(0, b"new", 2)
    slab.set_at(1, b"new", 2)
    assert slab.get(0) == b"new"

    touched = slab.void_epoch(2)
    assert touched >= 4, "two versions taken away and two made current again"
    assert slab.get(0) == b"old", "what the undone write superseded is back"
    assert slab.get(1) == b"old"


def test_voiding_an_epoch_nothing_was_written_at_changes_nothing(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 8, scratch("vslab-epochs"))
    slab.set_at(0, b"value", 5)
    assert slab.void_epoch(9) == 0
    assert slab.get(0) == b"value"


def test_a_versioned_slab_refuses_a_slot_it_does_not_have(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 2, scratch("vslab-epochs"))
    with pytest.raises(IndexError):
        slab.set(5, b"value")
    with pytest.raises(IndexError):
        slab.get(5)


def test_a_versioned_slab_refuses_a_value_that_does_not_fit(scratch):
    slab = subetha.VersionedSlab(scratch("vslab"), 4, scratch("vslab-epochs"))
    with pytest.raises(ValueError):
        slab.set(0, b"x" * (subetha.VersionedSlab.max_value_bytes + 1))


def test_a_versioned_slab_is_shared_between_handles(scratch):
    path, epochs = scratch("vslab"), scratch("vslab-epochs")
    first = subetha.VersionedSlab(path, 8, epochs)
    first.set(3, b"shared")
    second = subetha.VersionedSlab.open(path, 8, epochs)
    assert second.get(3) == b"shared"


def test_a_reservoir_keeps_what_it_is_given_until_it_is_full(scratch):
    sample = subetha.Reservoir(scratch("res"), capacity=4)
    assert sample.capacity == 4
    for item in (b"a", b"b", b"c"):
        assert sample.record(item) is not None, "nothing is refused while there is room"
    assert sample.total_seen == 3
    assert sorted(sample.snapshot()) == [b"a", b"b", b"c"]


def test_a_reservoir_never_grows_past_its_capacity(scratch):
    sample = subetha.Reservoir(scratch("res"), capacity=8)
    sample.record_many([f"item-{n}".encode() for n in range(200)])
    assert sample.total_seen == 200
    assert len(sample.snapshot()) == 8, "the sample is bounded whatever the stream"
    assert len(sample) == 8


def test_a_reservoir_starts_refusing_once_it_is_full(scratch):
    sample = subetha.Reservoir(scratch("res"), capacity=4)
    kept = sample.record_many([f"item-{n}".encode() for n in range(400)])
    # Keeping everything would mean the sample was not sampling.
    assert kept < 400


def test_a_reservoir_value_keeps_its_trailing_zeros(scratch):
    sample = subetha.Reservoir(scratch("res"), capacity=4)
    sample.record(b"ab\x00\x00")
    assert sample.snapshot() == [b"ab\x00\x00"]


def test_a_reservoir_refuses_a_value_that_does_not_fit(scratch):
    sample = subetha.Reservoir(scratch("res"), capacity=4)
    with pytest.raises(ValueError):
        sample.record(b"x" * (subetha.Reservoir.max_value_bytes + 1))
    assert sample.record(b"x" * subetha.Reservoir.max_value_bytes) is not None


def test_resetting_a_reservoir_forgets_everything(scratch):
    sample = subetha.Reservoir(scratch("res"), capacity=4)
    sample.record_many([b"a", b"b"])
    sample.reset()
    assert sample.snapshot() == []
    assert sample.total_seen == 0


def test_a_reservoir_is_shared_between_handles(scratch):
    path = scratch("res")
    first = subetha.Reservoir(path, capacity=4)
    first.record(b"shared")
    second = subetha.Reservoir.open(path, capacity=4)
    assert second.snapshot() == [b"shared"]
    assert second.total_seen == 1


def test_a_blocked_filter_never_forgets_what_was_added(scratch):
    bits, hashes = subetha.BlockedBloomFilter.suggest(1000, 0.01)
    seen = subetha.BlockedBloomFilter(scratch("bbf"), bits, hashes)
    added = [f"item-{n}".encode() for n in range(1000)]
    seen.insert_many(added)
    assert all(item in seen for item in added), "a bloom filter never says no wrongly"


def test_a_blocked_filter_is_mostly_right_about_what_was_not_added(scratch):
    bits, hashes = subetha.BlockedBloomFilter.suggest(1000, 0.01)
    seen = subetha.BlockedBloomFilter(scratch("bbf"), bits, hashes)
    seen.insert_many([f"item-{n}".encode() for n in range(1000)])

    absent = [f"other-{n}".encode() for n in range(2000)]
    wrong = sum(seen.contains_many(absent))
    assert wrong / len(absent) < 0.05, "the rate of wrong yeses must stay near what was asked"


def test_a_blocked_filter_suggests_a_size_from_the_rate_asked_for(scratch):
    loose, _ = subetha.BlockedBloomFilter.suggest(1000, 0.1)
    tight, tight_hashes = subetha.BlockedBloomFilter.suggest(1000, 0.001)
    assert tight > loose, "fewer wrong yeses costs more room"
    assert tight_hashes >= 1


def test_a_blocked_filter_refuses_a_rate_that_is_not_a_rate(scratch):
    with pytest.raises(ValueError):
        subetha.BlockedBloomFilter.suggest(1000, 0.0)
    with pytest.raises(ValueError):
        subetha.BlockedBloomFilter.suggest(1000, 1.5)


def test_clearing_a_blocked_filter_empties_it(scratch):
    seen = subetha.BlockedBloomFilter(scratch("bbf"), 4096, 4)
    seen.insert(b"here")
    assert b"here" in seen
    seen.clear()
    assert b"here" not in seen


def test_a_blocked_filter_is_shared_between_handles(scratch):
    path = scratch("bbf")
    first = subetha.BlockedBloomFilter(path, 4096, 4)
    first.insert(b"shared")
    second = subetha.BlockedBloomFilter.open(path, 4096, 4)
    assert b"shared" in second
    assert second.blocks == first.blocks
    assert second.hashes == first.hashes


def test_a_handle_table_gives_back_what_was_put_in(scratch):
    table = subetha.HandleTable(scratch("handles"), capacity=8)
    handle = table.insert(b"value")
    assert table.get(handle) == b"value"
    assert handle in table
    assert len(table) == 1


def test_a_removed_handle_stops_naming_anything(scratch):
    table = subetha.HandleTable(scratch("handles"), capacity=8)
    handle = table.insert(b"value")
    assert table.remove(handle) == b"value"
    assert table.get(handle) is None
    assert handle not in table
    assert table.remove(handle) is None


def test_an_old_handle_does_not_reach_the_new_occupant(scratch):
    # This is the whole point of a handle over an index: the place gets
    # reused, and the old handle must not follow it there.
    table = subetha.HandleTable(scratch("handles"), capacity=2)
    stale = table.insert(b"first")
    table.remove(stale)
    fresh = table.insert(b"second")

    assert table.get(fresh) == b"second"
    assert table.get(stale) is None, "the reused place must not answer the old handle"


def test_a_handle_table_fills_up(scratch):
    table = subetha.HandleTable(scratch("handles"), capacity=2)
    handles = table.insert_many([b"a", b"b", b"c"])
    assert len(handles) == 2, "a short answer is how a full table says so"
    with pytest.raises(OSError):
        table.insert(b"d")


def test_a_handle_table_reads_several_handles_at_once(scratch):
    table = subetha.HandleTable(scratch("handles"), capacity=8)
    handles = table.insert_many([b"a", b"b", b"c"])
    table.remove(handles[1])
    assert table.get_many(handles) == [b"a", None, b"c"]


def test_a_handle_table_refuses_a_value_that_does_not_fit(scratch):
    table = subetha.HandleTable(scratch("handles"), capacity=4)
    with pytest.raises(ValueError):
        table.insert(b"x" * (subetha.HandleTable.max_value_bytes + 1))


def test_a_handle_table_is_shared_between_handles(scratch):
    path = scratch("handles")
    first = subetha.HandleTable(path, capacity=8)
    handle = first.insert(b"shared")
    second = subetha.HandleTable.open(path, capacity=8)
    assert second.get(handle) == b"shared"


def test_a_tile_hides_what_was_written_after_the_reader_arrived(scratch):
    tile = subetha.TimePointTile(scratch("tile"))
    tile.insert(10, b"early")
    tile.insert(30, b"late")

    # A reader at 20 sees only what was there by then.
    assert tile.visible_count(20) == 1
    assert [value for _, value in tile.visible(20)] == [b"early"]
    # A later reader sees both.
    assert tile.visible_count(30) == 2


def test_a_tile_says_which_places_a_reader_can_see(scratch):
    tile = subetha.TimePointTile(scratch("tile"))
    first = tile.insert(5, b"a")
    second = tile.insert(15, b"b")
    mask = tile.visible_mask(10)
    assert mask & (1 << first), "the earlier place is in the mask"
    assert not mask & (1 << second), "the later one is not"


def test_a_tile_reads_back_one_place(scratch):
    tile = subetha.TimePointTile(scratch("tile"))
    lane = tile.insert(7, b"value")
    assert tile.at(lane) == (7, b"value")
    tile.remove(lane)
    assert tile.at(lane) is None


def test_a_tile_holds_sixteen_and_then_refuses(scratch):
    tile = subetha.TimePointTile(scratch("tile"))
    for n in range(subetha.TimePointTile.lanes):
        tile.insert(n, bytes([n]))
    assert tile.full is True
    assert len(tile) == subetha.TimePointTile.lanes
    with pytest.raises(OSError):
        tile.insert(99, b"one too many")


def test_a_tile_refuses_a_place_it_does_not_have(scratch):
    tile = subetha.TimePointTile(scratch("tile"))
    with pytest.raises(ValueError):
        tile.at(subetha.TimePointTile.lanes)
    with pytest.raises(ValueError):
        tile.remove(subetha.TimePointTile.lanes)


def test_a_tile_refuses_a_value_that_does_not_fit(scratch):
    tile = subetha.TimePointTile(scratch("tile"))
    with pytest.raises(ValueError):
        tile.insert(1, b"x" * (subetha.TimePointTile.max_value_bytes + 1))


def test_a_tile_is_shared_between_handles(scratch):
    path = scratch("tile")
    first = subetha.TimePointTile(path)
    lane = first.insert(3, b"shared")
    second = subetha.TimePointTile.open(path)
    assert second.at(lane) == (3, b"shared")


def test_a_reorder_window_releases_the_smallest_stamp_first(scratch):
    window = subetha.ReorderWindow(floor=2, cap=8)
    # Out of order in, in order out.
    window.push_many([(30, b"third"), (10, b"first"), (20, b"second")])
    assert len(window) == 3
    item, stamp = window.take()
    assert stamp == 10
    assert item.startswith(b"first")


def test_a_reorder_window_holds_items_until_it_is_full(scratch):
    window = subetha.ReorderWindow(floor=4, cap=8)
    window.push(1, b"one")
    window.push(2, b"two")
    # Fewer items held than the window, so nothing is released yet.
    assert window.take() is None
    assert len(window) == 2


def test_a_reorder_window_gives_back_its_tail(scratch):
    window = subetha.ReorderWindow(floor=8, cap=8)
    window.push_many([(3, b"c"), (1, b"a"), (2, b"b")])
    assert window.take() is None, "the window is not full, so nothing streams"

    drained = window.flush_all()
    assert [stamp for _, stamp in drained] == [1, 2, 3]
    assert len(window) == 0


def test_a_reorder_window_flushes_one_at_a_time(scratch):
    window = subetha.ReorderWindow(floor=8, cap=8)
    window.push_many([(2, b"b"), (1, b"a")])
    first = window.flush()
    assert first[1] == 1
    assert window.flush()[1] == 2
    assert window.flush() is None


def test_a_reorder_window_widens_when_an_item_is_late(scratch):
    window = subetha.ReorderWindow(floor=1, cap=16)
    assert window.corrections == 0
    started_at = window.window

    # Release a high stamp, then offer a lower one: that is an item
    # further out of order than the window covered.
    window.push_many([(100, b"late"), (200, b"later")])
    window.take()
    window.push_many([(1, b"early"), (300, b"x")])
    window.take()

    assert window.corrections >= 1, "a late item must be counted"
    assert window.window > started_at, "and must widen the window"


def test_a_reorder_window_can_be_widened_by_hand(scratch):
    window = subetha.ReorderWindow(floor=2, cap=64)
    window.widen_to(16)
    assert window.window == 16
    # Widening to less than it already is leaves it alone.
    window.widen_to(4)
    assert window.window == 16


def test_a_ring_is_unstamped_unless_asked(scratch):
    ring = subetha.Ring(scratch("ord"), capacity=64)
    assert ring.stamped is False
    assert ring.stamps is None


def test_a_ring_carries_the_kind_of_stamp_it_was_asked_for(scratch):
    ring = subetha.Ring(scratch("ord"), capacity=64, stamps="counter")
    assert ring.stamped is True
    assert ring.stamps == "counter"


def test_a_ring_refuses_a_stamp_kind_it_does_not_have(scratch):
    with pytest.raises(ValueError):
        subetha.Ring(scratch("ord"), capacity=64, stamps="sundial")


def test_an_ordered_receiver_delivers_what_was_sent(scratch):
    ring = subetha.Ring(scratch("ord"), capacity=64, stamps="counter")
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    receiver = ring.ordered_receiver(consumer)

    ring.send_many(producer, [b"one", b"two", b"three"])

    taken = receiver.drain()

    assert len(taken) == 3, "everything sent must come back, tail included"
    assert [bytes(item).rstrip(b"\x00") for item, _ in taken] == [b"one", b"two", b"three"]


def test_an_ordered_receiver_delivers_by_stamp_not_by_arrival(scratch):
    ring = subetha.Ring(scratch("ord"), capacity=64, stamps="counter")
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    receiver = ring.ordered_receiver(consumer)
    ring.send_many(producer, [b"a", b"b", b"c", b"d"])

    stamps = [stamp for _, stamp in receiver.drain()]
    assert len(stamps) == 4
    assert stamps == sorted(stamps), "delivery must climb the stamps"


def test_a_receiver_streaming_one_at_a_time_leaves_a_tail(scratch):
    # recv answers None while the window is still filling, which is why
    # a stream that stops there must finish with the tail.
    ring = subetha.Ring(scratch("ord"), capacity=64, stamps="counter")
    producer = ring.register_producer()
    receiver = ring.ordered_receiver(ring.register_consumer())
    ring.send_many(producer, [b"one", b"two", b"three"])

    streamed = []
    for _ in range(16):
        got = receiver.recv()
        if got is not None:
            streamed.append(got)
    streamed.extend(receiver.flush_all())

    assert len(streamed) == 3


def test_a_drain_bounded_short_still_gives_back_what_it_read(scratch):
    ring = subetha.Ring(scratch("ord"), capacity=64, stamps="counter")
    producer = ring.register_producer()
    receiver = ring.ordered_receiver(ring.register_consumer())
    ring.send_many(producer, [b"one", b"two", b"three", b"four"])

    first = receiver.drain(max_items=2)
    assert len(first) == 2, "two reads of the ring, then the held tail"
    rest = receiver.drain()
    assert len(rest) == 2
    assert len(receiver.drain()) == 0


def test_a_drain_of_nothing_is_empty(scratch):
    ring = subetha.Ring(scratch("ord"), capacity=64, stamps="counter")
    receiver = ring.ordered_receiver(ring.register_consumer())
    assert receiver.drain() == []


def test_an_ordered_receiver_says_which_strategy_it_picked(scratch):
    ring = subetha.Ring(scratch("ord"), capacity=64, stamps="counter")
    consumer = ring.register_consumer()
    receiver = ring.ordered_receiver(consumer)
    assert receiver.strategy in ("reorder", "strict", "direct")
    assert receiver.corrections == 0


def test_a_counter_stamped_ring_gets_the_buffering_strategy(scratch):
    # A count shared between senders has no time in it, so the receiver
    # has to correct on its own side rather than trust arrival order.
    ring = subetha.Ring(scratch("ord"), capacity=64, stamps="counter")
    receiver = ring.ordered_receiver(ring.register_consumer())
    assert receiver.strategy == "reorder"


def test_a_clock_stamped_ring_needs_no_buffering(scratch):
    # A clock stamp is already in order at the ring, so the receiver
    # delivers straight through and holds no tail.
    ring = subetha.Ring(scratch("ord"), capacity=64, stamps="monotonic")
    receiver = ring.ordered_receiver(ring.register_consumer())
    assert receiver.strategy == "direct"
    assert receiver.flush() is None


def test_an_ordered_receiver_refuses_a_ring_with_no_stamps(scratch):
    ring = subetha.Ring(scratch("ord"), capacity=64)
    with pytest.raises(ValueError):
        ring.ordered_receiver(ring.register_consumer())


def test_an_ordered_receiver_keeps_its_ring_alive(scratch):
    receiver = subetha.Ring(
        scratch("ord"), capacity=64, stamps="counter"
    ).ordered_receiver(0)
    # The ring object is unreferenced now. The receiver holds it, so
    # collecting must not take the memory it reads from out from under it.
    gc.collect()
    assert receiver.recv() is None
    assert receiver.strategy in ("reorder", "strict", "direct")


def test_a_locale_ring_keeps_its_wakers_beside_its_file(scratch):
    # A waiter in any process, a C one included, parks on these files, and
    # a send, receive or migration from here wakes it through them.
    path = scratch("loc-wakers")
    ring = subetha.LocaleRing(path, capacity=8)
    assert os.path.exists(path + ".cwaker.bin")
    assert os.path.exists(path + ".pwaker.bin")
    assert ring.locale == "anon"


def test_a_locale_ring_starts_in_process(scratch):
    ring = subetha.LocaleRing(scratch("loc"), capacity=8)
    assert ring.locale == "anon"
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    assert ring.send(producer, b"here") is True
    assert ring.recv(consumer).startswith(b"here")


def test_a_locale_ring_moves_to_a_file_and_keeps_working(scratch):
    ring = subetha.LocaleRing(scratch("loc"), capacity=8)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    before = ring.locale_generation

    ring.migrate_to("file")
    assert ring.locale == "file"
    assert ring.locale_generation > before, "a migration must be visible to a holder"

    assert ring.send(producer, b"after") is True
    assert ring.recv(consumer).startswith(b"after")


def test_a_locale_ring_carries_what_it_holds_across_a_move(scratch):
    ring = subetha.LocaleRing(scratch("loc"), capacity=8)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    ring.send_many(producer, [b"one", b"two"])

    ring.migrate_to("file")

    taken = ring.recv_many(consumer, 10)
    assert len(taken) == 2, "a move must not drop what was already in the ring"
    assert {bytes(item).rstrip(b"\x00") for item in taken} == {b"one", b"two"}


def test_a_locale_ring_moves_to_named_memory(scratch):
    ring = subetha.LocaleRing(scratch("loc"), capacity=8)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    ring.migrate_to("shmfs")
    assert ring.locale == "shmfs"
    assert ring.send(producer, b"named") is True
    assert ring.recv(consumer).startswith(b"named")


def test_a_locale_ring_moving_where_it_already_is_changes_nothing(scratch):
    ring = subetha.LocaleRing(scratch("loc"), capacity=8)
    before = ring.locale_generation
    ring.migrate_to("anon")
    assert ring.locale == "anon"
    assert ring.locale_generation == before


def test_a_locale_ring_refuses_a_place_it_does_not_have(scratch):
    ring = subetha.LocaleRing(scratch("loc"), capacity=8)
    with pytest.raises(ValueError):
        ring.migrate_to("somewhere_else")


def test_a_locale_ring_hands_given_back_ids_out_again(scratch):
    ring = subetha.LocaleRing(scratch("loc-give-back"), capacity=8, max_producers=2, max_consumers=2)
    kept_producer = ring.register_producer()
    gone_producer = ring.register_producer()
    kept_consumer = ring.register_consumer()
    gone_consumer = ring.register_consumer()
    ring.unregister_consumer(gone_consumer)
    ring.unregister_producer(gone_producer)
    # A position past the consumer table names no consumer.
    ring.unregister_consumer(1 << 16)
    sent = [b"%d" % n for n in range(4)]
    assert ring.send_many(kept_producer, sent) == len(sent)
    taken = ring.recv_many(kept_consumer, 10)
    assert [bytes(item).rstrip(b"\x00") for item in taken] == sent, "the consumer that remains reads every item"
    assert ring.register_consumer() == gone_consumer, "the consumer's id is handed out again"
    assert ring.register_producer() == gone_producer, "the producer's id is handed out again"


def test_a_locale_ring_refuses_a_capacity_that_is_not_a_power_of_two(scratch):
    with pytest.raises(ValueError):
        subetha.LocaleRing(scratch("loc"), capacity=6)


def test_an_unstamped_locale_ring_has_no_ordering_to_report(scratch):
    ring = subetha.LocaleRing(scratch("loc"), capacity=8)
    assert ring.stamped is False
    assert ring.ordering_mode is None


def test_a_stamped_locale_ring_keeps_its_ordering_across_a_move(scratch):
    ring = subetha.LocaleRing(scratch("loc"), capacity=8, stamped=True)
    assert ring.stamped is True
    ring.set_ordering_mode("merge_by_stamp")
    assert ring.ordering_mode == "merge_by_stamp"

    producer = ring.register_producer()
    consumer = ring.register_consumer()
    ring.send_many(producer, [b"one", b"two"])
    ring.migrate_to("file")

    # The discipline is set on every backing, so the move does not
    # change it underneath the reader.
    assert ring.ordering_mode == "merge_by_stamp"
    taken = ring.recv_many(consumer, 10)
    assert len(taken) == 2
    assert ring.inversions == 0


def test_a_locale_ring_can_be_opened_by_another_holder(scratch):
    path = scratch("loc")
    ring = subetha.LocaleRing(path, capacity=8)
    ring.migrate_to("file")
    producer = ring.register_producer()
    ring.send(producer, b"across")

    second = subetha.LocaleRing.open(path, capacity=8)
    assert second.locale == "file", "the locale is shared, not per handle"
    consumer = second.register_consumer()
    assert second.recv(consumer).startswith(b"across")


def test_advancing_moves_the_published_epoch_when_nothing_is_open(scratch):
    epochs = subetha.Epochs(scratch("epochs"), capacity=8)
    before = epochs.now
    taken = epochs.advance()
    assert taken > before
    assert epochs.now == taken, "with no open ticket the published epoch follows"


def test_an_open_ticket_holds_the_published_epoch_below_itself(scratch):
    epochs = subetha.Epochs(scratch("epochs"), capacity=8)
    slot, epoch = epochs.claim_ticket()
    # The write this ticket stamps is not visible yet, so readers must
    # not see its epoch. That is the whole mechanism.
    assert epochs.now == epoch - 1
    assert epochs.open_tickets == 1
    epochs.publish_ticket(slot)
    assert epochs.open_tickets == 0
    assert epochs.now >= epoch, "publishing should let readers past it"


def test_an_open_ticket_holds_the_line_however_far_the_counter_moves(scratch):
    epochs = subetha.Epochs(scratch("epochs"), capacity=8)
    slot, epoch = epochs.claim_ticket()
    held_at = epochs.now
    for _ in range(5):
        epochs.advance()
    # The counter has run on, but the published epoch cannot pass the
    # open ticket: an in-flight compound write stays invisible.
    assert epochs.now == held_at
    epochs.publish_ticket(slot)
    assert epochs.now > held_at, "publishing should release the line"


def test_tickets_run_out_rather_than_overrunning(scratch):
    epochs = subetha.Epochs(scratch("epochs"), capacity=2)
    slots = []
    with pytest.raises(OSError):
        for _ in range(10):
            slot, _ = epochs.claim_ticket()
            slots.append(slot)
    for slot in slots:
        epochs.publish_ticket(slot)


def test_an_epochs_table_needs_a_capacity(scratch):
    with pytest.raises(ValueError):
        subetha.Epochs(scratch("epochs"), capacity=0)


def test_a_ticket_slot_past_the_capacity_raises_index_error(scratch):
    epochs = subetha.Epochs(scratch("epochs"), capacity=2)
    with pytest.raises(IndexError, match="slot 2 is past the 2 the table holds"):
        epochs.publish_ticket(2)


def test_an_lru_cache_holds_and_returns(scratch):
    cache = subetha.LruCache(scratch("lru"), capacity=4, key_size=2, value_size=4)
    cache.put(b"aa", b"1111")
    assert cache.get(b"aa") == b"1111"
    assert b"aa" in cache
    assert len(cache) == 1


def full_of_unused_entries(scratch):
    """A cache of three holding bb, cc and dd, none of them counted as used.

    Each entry is counted as used as it lands, so the first overflow clears
    every mark once around and evicts aa, the first entry it reaches again.
    """
    cache = subetha.LruCache(scratch("lru"), capacity=3, key_size=2, value_size=2)
    for key in (b"aa", b"bb", b"cc"):
        cache.put(key, b"11")
    cache.put(b"dd", b"44")
    assert b"aa" not in cache
    return cache


def test_an_lru_cache_evicts_an_unused_entry_and_passes_a_touched_one(scratch):
    cache = full_of_unused_entries(scratch)
    cache.touch(b"bb")
    cache.put(b"ee", b"55")
    assert b"ee" in cache and b"dd" in cache
    assert b"bb" in cache, "the touched key should have survived"
    assert b"cc" not in cache, "the untouched key should have been evicted"


def test_reading_without_touching_leaves_the_order_alone(scratch):
    cache = full_of_unused_entries(scratch)
    # A plain get must not count as use, so bb stays the next to go.
    assert cache.get(b"bb") == b"11"
    cache.put(b"ee", b"55")
    assert b"bb" not in cache, "a plain get should not have rescued it"
    assert b"cc" in cache


def test_get_and_touch_counts_as_use(scratch):
    cache = full_of_unused_entries(scratch)
    assert cache.get_and_touch(b"bb") == b"11"
    cache.put(b"ee", b"55")
    assert b"bb" in cache, "get_and_touch should have rescued it"
    assert b"cc" not in cache


def test_an_lru_cache_removes(scratch):
    cache = subetha.LruCache(scratch("lru"), capacity=4, key_size=2, value_size=2)
    cache.put(b"aa", b"11")
    assert cache.remove(b"aa") == b"11"
    assert cache.remove(b"aa") is None


def test_an_lru_cache_needs_a_capacity(scratch):
    with pytest.raises(ValueError):
        subetha.LruCache(scratch("lru"), capacity=0, key_size=2, value_size=2)


def test_a_hyperloglog_estimates_distinct_items(scratch):
    hll = subetha.HyperLogLog(scratch("hll"), precision=14)
    hll.insert_many([f"item-{i}".encode() for i in range(10000)])
    estimate = hll.estimate()
    # An estimate, not a count. Precision 14 is good to a couple of
    # percent, so the bound is loose enough not to be flaky and tight
    # enough to catch a binding that counts nothing.
    assert 9000 <= estimate <= 11000, f"estimated {estimate} distinct out of 10000"


def test_a_hyperloglog_ignores_repeats(scratch):
    hll = subetha.HyperLogLog(scratch("hll"), precision=14)
    hll.insert_many([b"the same item"] * 1000)
    assert hll.estimate() <= 5, "a thousand copies of one item is one distinct item"


def test_a_hyperloglog_refuses_a_precision_it_cannot_hold(scratch):
    with pytest.raises(ValueError):
        subetha.HyperLogLog(scratch("hll"), precision=3)
    with pytest.raises(ValueError):
        subetha.HyperLogLog(scratch("hll2"), precision=17)


def test_a_hyperloglog_resets(scratch):
    hll = subetha.HyperLogLog(scratch("hll"), precision=10)
    hll.insert_many([f"x{i}".encode() for i in range(500)])
    assert hll.estimate() > 100
    hll.reset()
    assert hll.estimate() == 0


def test_a_count_min_sketch_never_undercounts(scratch):
    depth, width = subetha.CountMinSketch.suggest_config(0.001, 0.01)
    sketch = subetha.CountMinSketch(scratch("cms"), depth=depth, width=width)
    sketch.insert_n(b"frequent", 500)
    sketch.insert_many([f"other-{i}".encode() for i in range(100)])
    # The one-sided guarantee: never less than the truth.
    assert sketch.estimate_count(b"frequent") >= 500


def test_a_count_min_sketch_reads_many_in_one_call(scratch):
    sketch = subetha.CountMinSketch(scratch("cms"), depth=4, width=256)
    sketch.insert_n(b"a", 3)
    sketch.insert_n(b"b", 7)
    counts = sketch.estimate_many([b"a", b"b"])
    assert counts[0] >= 3 and counts[1] >= 7


def test_a_count_min_sketch_refuses_a_zero_shape(scratch):
    with pytest.raises(ValueError):
        subetha.CountMinSketch(scratch("cms"), depth=0, width=16)
    with pytest.raises(ValueError):
        subetha.CountMinSketch.suggest_config(0.0, 0.5)


def test_a_bit_vec_sets_clears_and_toggles(scratch):
    bits = subetha.BitVec(scratch("bits"), capacity_bits=128)
    assert len(bits) == 128
    assert bits.set(5) is False, "set should report what the bit was"
    assert bits[5] is True
    assert bits.clear(5) is True
    assert bits[5] is False
    # toggle reports the value it landed on, where set and clear report
    # the value they replaced.
    assert bits.toggle(5) is True
    assert bits[5] is True
    assert bits.toggle(5) is False
    assert bits[5] is False


def test_a_bit_vec_sets_a_range_in_one_call(scratch):
    bits = subetha.BitVec(scratch("bits"), capacity_bits=64)
    bits.set_range(8, 16)
    assert all(bits[i] for i in range(8, 16))
    assert not bits[7] and not bits[16], "the range should not spill past its ends"


def test_a_bit_vec_index_past_the_end_raises(scratch):
    bits = subetha.BitVec(scratch("bits"), capacity_bits=8)
    with pytest.raises(IndexError):
        bits.get(99)


def test_a_bloom_filter_never_denies_what_it_holds(scratch):
    bits, hashes = subetha.BloomFilter.suggest_config(1000, 0.01)
    bloom = subetha.BloomFilter(scratch("bloom"), n_bits=bits, n_hashes=hashes)
    held = [f"item-{i}".encode() for i in range(100)]
    bloom.insert_many(held)
    # A Bloom filter may say yes wrongly; it must never say no wrongly.
    assert all(item in bloom for item in held), "a filter must never deny what it holds"


def test_a_bloom_filter_mostly_rejects_what_it_does_not_hold(scratch):
    bits, hashes = subetha.BloomFilter.suggest_config(1000, 0.01)
    bloom = subetha.BloomFilter(scratch("bloom"), n_bits=bits, n_hashes=hashes)
    bloom.insert_many([f"held-{i}".encode() for i in range(100)])
    absent = [f"absent-{i}".encode() for i in range(1000)]
    wrong = sum(bloom.contains_many(absent))
    assert wrong < 100, f"a filter sized for one percent said yes {wrong} times in 1000"


def test_a_bloom_filter_refuses_an_impossible_rate(scratch):
    with pytest.raises(ValueError):
        subetha.BloomFilter.suggest_config(1000, 0.0)
    with pytest.raises(ValueError):
        subetha.BloomFilter.suggest_config(1000, 1.0)


def test_a_bloom_filter_clears(scratch):
    bloom = subetha.BloomFilter(scratch("bloom"), n_bits=4096, n_hashes=3)
    bloom.insert(b"gone soon")
    assert b"gone soon" in bloom
    bloom.clear()
    assert b"gone soon" not in bloom


def test_a_histogram_counts_into_the_right_buckets(scratch):
    hist = subetha.Histogram(scratch("hist"), boundaries=[10, 100, 1000])
    hist.record_many([5, 50, 500, 5000])
    assert hist.total_count == 4
    assert sum(hist.counts) == 4, "every recorded value should land in some bucket"


def test_a_histogram_reads_every_bucket_in_one_call(scratch):
    hist = subetha.Histogram(scratch("hist"), boundaries=[10, 20])
    hist.record(5)
    hist.record(15)
    counts = hist.counts
    assert len(counts) == hist.n_buckets
    assert counts == [hist.count(i) for i in range(hist.n_buckets)]


def test_a_histogram_refuses_boundaries_that_do_not_rise(scratch):
    with pytest.raises(ValueError):
        subetha.Histogram(scratch("hist"), boundaries=[10, 5])
    with pytest.raises(ValueError):
        subetha.Histogram(scratch("hist2"), boundaries=[])


def test_a_histogram_percentile_is_bounded_by_its_buckets(scratch):
    hist = subetha.Histogram(scratch("hist"), boundaries=[10, 100, 1000])
    hist.record_many([5] * 90 + [500] * 10)
    assert hist.percentile(50) <= hist.percentile(99)
    with pytest.raises(ValueError):
        hist.percentile(101)


def test_a_rate_limiter_hands_out_its_capacity_then_refuses(scratch):
    limiter = subetha.RateLimiter(scratch("limiter"), capacity=3, refill_per_second=1)
    assert limiter.available == 3
    assert limiter.try_acquire(3) is True
    assert limiter.available == 0
    assert limiter.try_acquire(1) is False, "an empty bucket should answer False, not raise"


def test_a_rate_limiter_resets(scratch):
    limiter = subetha.RateLimiter(scratch("limiter"), capacity=2, refill_per_second=1)
    limiter.try_acquire(2)
    limiter.reset()
    assert limiter.available == 2


def test_a_rate_limiter_refuses_a_zero_configuration(scratch):
    with pytest.raises(ValueError):
        subetha.RateLimiter(scratch("limiter"), capacity=0, refill_per_second=1)
    with pytest.raises(ValueError):
        subetha.RateLimiter(scratch("limiter2"), capacity=1, refill_per_second=0)


# The adaptive ring, which the C ABI is organized around. Producers and
# consumers register for an id and every call names one.


def test_a_ring_carries_an_item_between_a_producer_and_a_consumer(scratch):
    ring = subetha.Ring(scratch("ring"), capacity=8)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    assert ring.send(producer, b"across") is True
    assert ring.recv(consumer).startswith(b"across")
    assert ring.recv(consumer) is None


def test_a_ring_reports_the_shape_it_is_in(scratch):
    ring = subetha.Ring(scratch("ring"), capacity=8)
    assert isinstance(ring.shape, str), "the shape should read as a name, not a number"
    assert ring.shape != ""


def test_a_ring_takes_a_batch_in_one_call(scratch):
    ring = subetha.Ring(scratch("ring"), capacity=64)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    assert ring.send_many(producer, [bytes([i]) for i in range(10)]) == 10
    assert len(ring.recv_many(consumer, 20)) == 10


def test_a_ring_takes_items_packed_in_one_buffer(scratch):
    ring = subetha.Ring(scratch("ring"), capacity=32)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    packed = b"".join(bytes([i]) * 4 for i in range(5))
    assert ring.send_buffer(producer, packed, 4) == 5
    assert len(ring.recv_many(consumer, 10)) == 5


def test_a_ring_hands_items_back_packed_in_one_buffer(scratch):
    ring = subetha.Ring(scratch("ring"), capacity=32)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    assert ring.send_many(producer, [bytes([i + 1]) * 4 for i in range(5)]) == 5
    packed, taken = ring.recv_buffer(consumer, 10)
    assert taken == 5
    assert len(packed) % taken == 0, "every item takes a whole slot"
    slot = len(packed) // taken
    assert [packed[i * slot : i * slot + 4] for i in range(taken)] == [bytes([i + 1]) * 4 for i in range(5)]
    assert ring.recv_buffer(consumer, 10) == (b"", 0)


def test_a_ring_wait_times_out_on_an_empty_ring_and_takes_an_item_once_one_is_there(scratch):
    ring = subetha.Ring(scratch("ring"), capacity=8)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    began = time.monotonic()
    assert ring.recv_for(consumer, timeout=0.05) is None, "a timeout answers None"
    assert time.monotonic() - began >= 0.04, "the wait lasted its timeout"
    assert ring.send(producer, b"one")
    assert ring.recv_for(consumer, timeout=5.0)[:3] == b"one"


def test_a_send_through_another_handle_wakes_a_waiting_receiver(scratch):
    path = scratch("ring")
    receiving = subetha.Ring(path, capacity=8)
    sending = subetha.Ring.open(path, capacity=8)
    consumer = receiving.register_consumer()
    producer = sending.register_producer()
    got = {}

    def wait():
        began = time.monotonic()
        got["item"] = receiving.recv_for(consumer, timeout=20.0)
        got["waited"] = time.monotonic() - began

    waiter = threading.Thread(target=wait)
    waiter.start()
    time.sleep(0.1)
    assert sending.send(producer, b"from the other handle")
    waiter.join()
    assert got["item"][:21] == b"from the other handle"
    assert got["waited"] < 10.0, "woken by the send, not by the timeout"


def test_a_receive_through_another_handle_wakes_a_sender_waiting_for_room(scratch):
    path = scratch("ring")
    sending = subetha.Ring(path, capacity=8)
    receiving = subetha.Ring.open(path, capacity=8)
    producer = sending.register_producer()
    consumer = receiving.register_consumer()
    while sending.send(producer, b"x"):
        pass
    assert sending.send_for(producer, b"late", timeout=0.05) is False, "a timeout answers False"
    result = {}

    def wait():
        result["sent"] = sending.send_for(producer, b"late", timeout=20.0)

    sender = threading.Thread(target=wait)
    sender.start()
    time.sleep(0.1)
    assert receiving.recv(consumer) is not None
    sender.join()
    assert result["sent"] is True


def test_an_ordered_receivers_pop_wakes_a_sender_waiting_for_room(scratch):
    ring = subetha.Ring(scratch("ord"), capacity=8, stamps="counter")
    producer = ring.register_producer()
    receiver = ring.ordered_receiver(ring.register_consumer())
    while ring.send(producer, b"x"):
        pass
    result = {}

    def wait():
        began = time.monotonic()
        result["sent"] = ring.send_for(producer, b"late", timeout=20.0)
        result["waited"] = time.monotonic() - began

    sender = threading.Thread(target=wait)
    sender.start()
    time.sleep(0.1)
    # The window may hold what the receiver took, so recv can answer None;
    # the item left the ring either way.
    receiver.recv()
    sender.join()
    assert result["sent"] is True
    assert result["waited"] < 10.0, "woken by the receiver's pop, not by the timeout"


def test_a_ring_carries_a_frame_larger_than_a_slot(scratch):
    ring = subetha.Ring(scratch("ring"), capacity=64)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    payload = bytes(range(256)) * 4
    assert ring.send_frame(producer, payload) is True
    assert ring.recv_frame(consumer) == payload, "a frame should survive the round trip whole"


def test_recv_any_names_each_kind_and_recv_frame_leaves_a_plain_item(scratch):
    ring = subetha.Ring(scratch("ring"), capacity=64)
    producer = ring.register_producer()
    consumer = ring.register_consumer()
    with pytest.raises(ValueError):
        ring.send(producer, b"\x07" * 64)  # the slot's last byte holds the kind
    assert ring.send(producer, b"\x07" * 63) is True
    assert ring.send_frame(producer, b"small") is True
    payload = bytes(range(256)) * 4
    assert ring.send_frame(producer, payload) is True

    with pytest.raises(subetha.NotAFrame):
        ring.recv_frame(consumer)
    assert ring.recv_any(consumer) == (b"\x07" * 63, "plain"), "the refused item stayed on the ring"
    assert ring.recv_any_for(consumer, timeout=5.0) == (b"small", "frame_inline")
    assert ring.recv_any(consumer) == (payload, "frame_offset")
    assert ring.recv_any(consumer) is None
    assert ring.recv_any_for(consumer, timeout=0.02) is None


def test_a_ring_needs_a_producer_and_a_consumer(scratch):
    with pytest.raises(ValueError):
        subetha.Ring(scratch("ring"), capacity=8, max_producers=0)
    with pytest.raises(ValueError):
        subetha.Ring(scratch("ring2"), capacity=8, max_consumers=0)


def test_a_ring_counts_its_registrations(scratch):
    ring = subetha.Ring(scratch("ring"), capacity=8, max_producers=2, max_consumers=2)
    assert ring.max_producers == 2
    assert ring.max_consumers == 2
    first = ring.register_producer()
    second = ring.register_producer()
    assert first != second, "each producer should get its own id"


def test_a_stack_and_a_deque_keep_their_wakers_beside_their_files(scratch):
    # A waiter in any process, a C one included, parks on these files, and
    # a push, pop or steal from here wakes it through them.
    stack_path = scratch("stack-wakers")
    stack = subetha.Stack(stack_path, capacity=8, element_size=2)
    deque_path = scratch("deque-wakers")
    deque = subetha.Deque(deque_path, capacity=8, element_size=2)
    for path in (stack_path, deque_path):
        assert os.path.exists(path + ".cwaker.bin")
        assert os.path.exists(path + ".pwaker.bin")
    assert stack.capacity == 8 and deque.capacity == 8


def test_a_stack_is_last_in_first_out(scratch):
    stack = subetha.Stack(scratch("stack"), capacity=8, element_size=2)
    stack.push(b"aa")
    stack.push(b"bb")
    assert stack.pop() == b"bb", "the last one in should come out first"
    assert stack.pop() == b"aa"
    assert stack.pop() is None


def test_a_stack_peeks_without_taking(scratch):
    stack = subetha.Stack(scratch("stack"), capacity=4, element_size=2)
    stack.push(b"aa")
    assert stack.peek() == b"aa"
    assert stack.peek() == b"aa", "peek should not consume"
    assert stack.pop() == b"aa"


def test_a_full_stack_refuses_rather_than_raising(scratch):
    stack = subetha.Stack(scratch("stack"), capacity=2, element_size=1)
    accepted = [stack.push(bytes([i])) for i in range(4)]
    assert False in accepted, "a stack of two should refuse the third"


def test_a_stack_refuses_an_alignment_that_is_not_a_power_of_two(scratch):
    with pytest.raises(ValueError):
        subetha.Stack(scratch("stack"), capacity=4, element_size=4, alignment=3)


def test_a_stack_refuses_a_zero_element_size(scratch):
    with pytest.raises(ValueError):
        subetha.Stack(scratch("stack"), capacity=4, element_size=0)


def test_a_deque_pops_its_own_end_and_steals_the_other(scratch):
    deque = subetha.Deque(scratch("deque"), capacity=8, element_size=2)
    deque.push_many([b"aa", b"bb", b"cc"])
    assert deque.pop() == b"cc", "the owner takes from its own end"
    assert deque.steal() == b"aa", "a thief takes from the far end"


def test_a_deque_refuses_a_capacity_that_is_not_a_power_of_two(scratch):
    with pytest.raises(ValueError):
        subetha.Deque(scratch("deque"), capacity=6, element_size=2)


def test_an_empty_deque_yields_nothing_to_either_end(scratch):
    deque = subetha.Deque(scratch("deque"), capacity=4, element_size=2)
    assert deque.pop() is None
    assert deque.steal() is None


def test_a_thief_attaches_to_a_deque_another_process_owns(scratch):
    path = scratch("deque")
    owner = subetha.Deque(path, capacity=8, element_size=2)
    owner.push_many([b"aa", b"bb"])
    thief = subetha.Deque.open_as_thief(path, element_size=2)
    assert thief.steal() == b"aa"


def test_every_subscriber_sees_every_item(scratch):
    ring = subetha.PubSub(scratch("pubsub"), capacity=8)
    first = ring.subscribe()
    second = ring.subscribe()
    ring.publish(b"to all")
    assert first.next().startswith(b"to all")
    assert second.next().startswith(b"to all"), "the second subscriber missed it"


def test_a_subscriber_starts_where_the_publisher_is(scratch):
    ring = subetha.PubSub(scratch("pubsub"), capacity=8)
    ring.publish(b"before anyone subscribed")
    late = ring.subscribe()
    assert late.next() is None, "a new subscriber should not see the past"
    ring.publish(b"after")
    assert late.next().startswith(b"after")


def test_a_subscriber_can_replay_from_a_recorded_position(scratch):
    ring = subetha.PubSub(scratch("pubsub"), capacity=8)
    ring.publish(b"one")
    ring.publish(b"two")
    replay = ring.subscribe_from(0)
    assert replay.next().startswith(b"one"), "replay should start where it was told"
    assert replay.next().startswith(b"two")


def test_a_subscriber_that_falls_behind_is_told_rather_than_lied_to(scratch):
    ring = subetha.PubSub(scratch("pubsub"), capacity=4)
    slow = ring.subscribe()
    # Publish well past the capacity, so what the subscriber wanted has
    # been overwritten rather than merely not arrived.
    for i in range(20):
        ring.publish(bytes([i]))

    with pytest.raises(subetha.Lagged) as caught:
        slow.next()
    assert "fell behind" in str(caught.value)


def test_a_lagged_subscriber_resumes_rather_than_wedging(scratch):
    ring = subetha.PubSub(scratch("pubsub"), capacity=4)
    slow = ring.subscribe()
    for i in range(20):
        ring.publish(bytes([i]))

    with pytest.raises(subetha.Lagged):
        slow.next()

    # It skipped to the publisher rather than staying stuck on the gap.
    assert slow.next() is None
    ring.publish(b"fresh")
    assert slow.next().startswith(b"fresh")


def test_a_subscriber_reports_its_lag(scratch):
    ring = subetha.PubSub(scratch("pubsub"), capacity=16)
    sub = ring.subscribe()
    assert sub.lag() == 0
    ring.publish(b"a")
    ring.publish(b"b")
    assert sub.lag() == 2
    sub.next_many(10)
    assert sub.lag() == 0


def test_a_publisher_refuses_an_oversized_item(scratch):
    ring = subetha.PubSub(scratch("pubsub"), capacity=4)
    with pytest.raises(ValueError):
        ring.publish(b"x" * (ring.payload_size + 1))


def test_a_lamport_pair_carries_items_in_order(scratch):
    producer, consumer = subetha.lamport_pair(scratch("lamport"), capacity=8)
    assert producer.push(b"first") is True
    assert producer.push(b"second") is True
    assert consumer.pop().startswith(b"first")
    assert consumer.pop().startswith(b"second")
    assert consumer.pop() is None


def test_a_lamport_pair_shares_one_capacity(scratch):
    producer, consumer = subetha.lamport_pair(scratch("lamport"), capacity=4)
    assert producer.capacity == consumer.capacity == 4


def test_a_lamport_producer_takes_a_batch(scratch):
    producer, consumer = subetha.lamport_pair(scratch("lamport"), capacity=16)
    assert producer.push_many([bytes([i]) for i in range(5)]) == 5
    assert len(consumer.pop_many(10)) == 5


def test_a_lamport_pair_reopens_onto_the_same_ring(scratch):
    path = scratch("lamport")
    producer, _ = subetha.lamport_pair(path, capacity=8)
    producer.push(b"left behind")
    _, consumer = subetha.lamport_pair_open(path, capacity=8)
    assert consumer.pop().startswith(b"left behind")


def test_a_frame_region_round_trips_a_payload(scratch):
    region = subetha.FrameRegion(scratch("frames"), block_size=256, block_count=8)
    assert region.block_size == 256
    index = region.write_new(b"a payload too big for a ring slot")
    assert index is not None
    assert region.read_block(index, 33) == b"a payload too big for a ring slot"
    region.free(index)


def test_a_frame_region_hands_out_every_block_then_refuses(scratch):
    region = subetha.FrameRegion(scratch("frames"), block_size=64, block_count=2)
    first = region.allocate()
    second = region.allocate()
    assert first is not None and second is not None
    assert region.allocate() is None, "an exhausted pool should answer None"
    region.free(first)
    assert region.allocate() is not None, "a freed block should come back"


def test_a_frame_region_refuses_a_payload_longer_than_a_block(scratch):
    region = subetha.FrameRegion(scratch("frames"), block_size=16, block_count=4)
    with pytest.raises(ValueError):
        region.write_new(b"x" * 17)


def test_take_block_reads_and_frees_in_one_call(scratch):
    region = subetha.FrameRegion(scratch("frames"), block_size=32, block_count=1)
    index = region.write_new(b"once")
    assert region.take_block(index, 4) == b"once"
    assert region.allocate() is not None, "take_block should have freed it"


def test_a_map_compare_exchange_swaps_only_on_a_match(scratch):
    m = subetha.HashMap(scratch("map"), capacity=8, key_size=2, value_size=2)
    m.insert(b"aa", b"11")
    swapped, current = m.compare_exchange(b"aa", b"99", b"22")
    assert swapped is False
    assert current == b"11", "it should report what was actually there"
    swapped, _ = m.compare_exchange(b"aa", b"11", b"22")
    assert swapped is True
    assert m.get(b"aa") == b"22"
