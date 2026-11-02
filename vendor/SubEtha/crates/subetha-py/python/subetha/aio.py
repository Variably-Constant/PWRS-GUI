"""Awaiting SubEtha from asyncio.

The Rust side has an async engine of its own: a reactor, an executor, a
task pool. None of it is bound and none of it could be, because every
entry point takes or returns a Rust future and Python has no way to make
one. Python has asyncio instead, and what asyncio needs is not Rust's
executor but a way to wait without blocking its loop.

That is what this module is, and it is deliberately small, because only
some of the surface can be awaited soundly.

**A queue can.** `recv` and `send` take the binding's own bounded wait,
which sleeps rather than spins and releases the interpreter while it
sleeps, and run it on a worker thread. What comes back is bytes, which
belong to nobody.

**A link has no wait to borrow.** `SensReceiver` cannot leave the thread
that made it, and it answers with whatever it could rebuild each time it
is asked rather than waiting. `poll` therefore asks on this thread and
yields to the loop between tries, which is cheap because each ask is
non-blocking.

**A notifier can, and it costs different things on each platform.**
`wait` hands the notifier's descriptor to the event loop on Unix, where
waiting then costs nothing at all. Windows has no public way to watch
the handle it gives out: the proactor loop, which is the default there,
does not implement `add_reader`, and the Windows selector loop selects
only on sockets. So on Windows the wait runs on a worker thread. What
you await is the same either way; only what it costs differs.

    import asyncio
    import subetha
    from subetha import aio

    async def main():
        chan = subetha.Channel("/tmp/work", capacity=1024)
        item = await aio.recv(chan, timeout=5)
        if item is not None:
            handle(item)

Nothing here is faster than calling the binding directly. Reach for it
when a coroutine must not block its loop, and call the binding directly
everywhere else.
"""

import asyncio
import sys

__all__ = [
    "recv",
    "send",
    "wait",
    "poll",
]

# Whether this interpreter's event loop can be handed a notifier's
# descriptor. Everywhere but Windows it can. On Windows the default loop
# since 3.8 is the proactor, which does not implement `add_reader` at
# all, and switching to the Windows selector loop does not help because
# its select accepts sockets and nothing else.
_LOOP_TAKES_THE_DESCRIPTOR = sys.platform != "win32"

# How long each underlying wait runs before it comes back to be retried.
# A canceled await cannot interrupt a wait already in progress, so this
# also bounds how long cancellation takes to take effect.
_SLICE_SECONDS = 0.25


async def _until(attempt, timeout):
    """Run `attempt(slice)` on a thread until it answers, or time runs out.

    `attempt` returns None to mean "nothing yet, ask again". Splitting
    the wait into slices is what lets a canceled await stop within a
    slice rather than at the far end of the caller's whole timeout.
    """
    loop = asyncio.get_running_loop()
    deadline = None if timeout is None else loop.time() + timeout
    while True:
        if deadline is None:
            slice_seconds = _SLICE_SECONDS
        else:
            remaining = deadline - loop.time()
            if remaining <= 0:
                return None
            slice_seconds = min(_SLICE_SECONDS, remaining)
        got = await asyncio.to_thread(attempt, slice_seconds)
        if got is not None:
            return got


async def recv(channel, timeout=None):
    """The next item from a `Channel` or `AdaptiveQueue`.

    Waits without blocking the event loop. Answers None when `timeout`
    seconds pass with nothing arriving; None with no timeout waits as
    long as it takes.
    """
    return await _until(lambda seconds: channel.recv_for(timeout=seconds), timeout)


async def send(channel, item, timeout=None):
    """Send one item, waiting for room without blocking the event loop.

    Answers True once it has gone, False when `timeout` seconds pass
    with no room.
    """

    def attempt(seconds):
        # False means no room this slice, which is "ask again" rather
        # than an answer, so it maps to None for the loop above.
        return True if channel.send_for(item, timeout=seconds) else None

    return bool(await _until(attempt, timeout))


async def wait(notifier, timeout=None):
    """Wait for a signal on a `Notifier` without blocking the loop.

    `True` when a signal arrived, `False` when `timeout` seconds passed
    with none. `None` waits for as long as it takes.

    Like the blocking `Notifier.wait`, this does not consume the signal.
    A notifier stays readable until somebody drains it, so a loop that
    waits again without calling `drain` returns at once on the signal it
    already saw:

        while await aio.wait(n, timeout=5):
            n.drain()
            handle_whatever_arrived()

    On Unix the descriptor goes to the event loop, so waiting occupies
    nothing. On Windows there is no public way to watch the handle a
    notifier gives out, so the wait runs on a worker thread from the
    loop's executor, and a wait with no timeout holds that thread until
    a signal arrives. Give Windows waits a timeout if the pool is
    small.

    One waiter per notifier. A loop keeps one reader per descriptor, so
    a second coroutine awaiting the same notifier replaces the first
    one's callback and the first never wakes. Give each waiter a
    notifier of its own, which is what `NotifierSet.attach` is for: a
    signal wakes every notifier in the set, so several waiters is what
    the set is shaped for and sharing one is not.
    """
    if timeout is not None and timeout < 0:
        raise ValueError("the timeout must not be negative")

    loop = asyncio.get_running_loop()
    if not _LOOP_TAKES_THE_DESCRIPTOR:
        return await loop.run_in_executor(None, notifier.wait, timeout)

    fd = notifier.native
    signaled = loop.create_future()

    def readable():
        if not signaled.done():
            signaled.set_result(True)

    loop.add_reader(fd, readable)
    try:
        if timeout is None:
            return await signaled
        return await asyncio.wait_for(signaled, timeout)
    except (asyncio.TimeoutError, TimeoutError):
        return False
    finally:
        # Before the future is discarded, so an already-readable
        # descriptor cannot go on calling back into a dead waiter.
        loop.remove_reader(fd)


async def poll(receiver, timeout=None, every=0.005):
    """Items from a `SensReceiver`, waiting for the first to arrive.

    The receiver cannot leave the thread that made it, so this asks on
    this thread rather than a worker one. That is cheap: each ask is
    non-blocking and answers with whatever the link could rebuild, and
    the sleep between tries is what yields to the loop.

    An empty list means the timeout passed with nothing arriving.
    """
    loop = asyncio.get_running_loop()
    deadline = None if timeout is None else loop.time() + timeout
    while True:
        taken = receiver.poll()
        if taken:
            return taken
        if deadline is not None and loop.time() >= deadline:
            return []
        await asyncio.sleep(every)
