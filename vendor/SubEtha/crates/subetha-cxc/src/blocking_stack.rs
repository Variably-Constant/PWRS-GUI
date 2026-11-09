//! A shared stack every process can wait on, whichever surface it reaches
//! SubEtha through.
//!
//! [`BlockingStack`] keeps a [`RawTreiberStack`] together with the two
//! wakers its waiters park on. A push through it wakes one waiter for an
//! item and a pop wakes one waiter for room, so a waiter in C, Python,
//! PowerShell or Rust wakes on a push or pop made through any of them. A
//! push or pop made through the [`RawTreiberStack`] itself wakes no one.
//!
//! The wakers sit in the files `<path>.cwaker.bin` and `<path>.pwaker.bin`
//! beside the stack at `<path>`, where the C ABI keeps them.

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub use crate::blocking_wait::{BlockingWaitError, MAX_WAITERS_DEFAULT};
use crate::blocking_wait::{deadline, file_wakers, WaitState, WAKE_ANY};
use crate::cross_process_waker::CrossProcessWaker;
use crate::raw_treiber_stack::RawTreiberStack;
use crate::shared_treiber_stack::StackError;

/// A [`RawTreiberStack`] with the wakers its waiters share across
/// processes.
pub struct BlockingStack {
    stack: RawTreiberStack,
    consumer_waker: Arc<CrossProcessWaker>,
    producer_waker: Arc<CrossProcessWaker>,
    state: WaitState,
}

impl BlockingStack {
    /// `stack` with its wakers in this process alone; each waker has
    /// `max_waiters` slots.
    pub fn anon(stack: RawTreiberStack, max_waiters: usize) -> Result<Self, BlockingWaitError<StackError>> {
        let consumer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        let producer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        Ok(Self::from_parts(stack, consumer_waker, producer_waker))
    }

    /// `stack` with its wakers in the files `<path>.cwaker.bin` and
    /// `<path>.pwaker.bin`, made when they do not exist and attached to
    /// when they do. `path` is the stack's own file.
    pub fn beside_file(stack: RawTreiberStack, path: impl AsRef<Path>, max_waiters: usize) -> Result<Self, BlockingWaitError<StackError>> {
        let (consumer_waker, producer_waker) = file_wakers(path.as_ref(), max_waiters)?;
        Ok(Self::from_parts(stack, consumer_waker, producer_waker))
    }

    /// `stack` with wakers its caller made. Waiters in other processes
    /// wake only when these are the ones beside the stack under the names
    /// above.
    pub fn from_parts(stack: RawTreiberStack, consumer_waker: CrossProcessWaker, producer_waker: CrossProcessWaker) -> Self {
        Self {
            stack,
            consumer_waker: Arc::new(consumer_waker),
            producer_waker: Arc::new(producer_waker),
            state: WaitState::default(),
        }
    }

    /// The stack itself, for everything that neither pushes nor pops.
    pub fn stack(&self) -> &RawTreiberStack {
        &self.stack
    }

    /// The waker poppers park on while the stack is empty.
    pub fn consumer_waker(&self) -> &Arc<CrossProcessWaker> {
        &self.consumer_waker
    }

    /// The waker pushers park on while every slot is in use.
    pub fn producer_waker(&self) -> &Arc<CrossProcessWaker> {
        &self.producer_waker
    }

    /// Waits through this handle that found every waiter slot in use and
    /// returned [`BlockingWaitError::WakerFull`].
    pub fn waiters_full(&self) -> u64 {
        self.state.waiters_full()
    }

    /// Whether [`close`](Self::close) has been called on this handle.
    pub fn is_closed(&self) -> bool {
        self.state.is_closed()
    }

    /// End every wait on this handle, now and from here on: each returns
    /// [`BlockingWaitError::Closed`]. Waits through other handles on the
    /// same stack wake, find the stack as it was and wait again.
    pub fn close(&self) {
        self.state.close();
        self.consumer_waker.wake_all();
        self.producer_waker.wake_all();
    }

    /// [`RawTreiberStack::push`], then wake one waiter for an item.
    pub fn try_push(&self, payload: &[u8]) -> Result<(), StackError> {
        self.stack.push(payload)?;
        self.consumer_waker.wake_one_up_to(WAKE_ANY);
        Ok(())
    }

    /// [`RawTreiberStack::pop`], then wake one waiter for room.
    pub fn try_pop(&self, out: &mut [u8]) -> Option<usize> {
        let n = self.stack.pop(out)?;
        self.producer_waker.wake_one_up_to(WAKE_ANY);
        Some(n)
    }

    /// Push, parking until a slot is free or `timeout` has passed; `None`
    /// waits as long as it takes.
    pub fn push_blocking(&self, payload: &[u8], timeout: Option<Duration>) -> Result<(), BlockingWaitError<StackError>> {
        self.state.wait(&self.producer_waker, deadline(timeout), |e| matches!(e, StackError::Full), || {
            self.try_push(payload)
        })
    }

    /// Pop, parking until there is an item or `timeout` has passed; `None`
    /// waits as long as it takes.
    pub fn pop_blocking(&self, out: &mut [u8], timeout: Option<Duration>) -> Result<usize, BlockingWaitError<Infallible>> {
        self.state.wait_for(&self.consumer_waker, deadline(timeout), || self.try_pop(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw_treiber_stack::ElementLayout;
    use std::path::PathBuf;
    use std::time::Instant;

    /// A folder of its own under the temporary directory, removed when the
    /// test ends; declared before the stacks so it drops after them.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subetha-blocking-stack-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("the wall clock is after the epoch")
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).expect("make the test's folder");
            Self(dir)
        }

        fn path(&self) -> PathBuf {
            self.0.join("stack")
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_dir_all(&self.0) {
                eprintln!("could not remove {}: {e}", self.0.display());
            }
        }
    }

    const LAYOUT: ElementLayout = ElementLayout { slot_size: 8, alignment: 1, tag: 0 };

    /// A handle on the stack of four slots at `path`, made or attached to
    /// as another process would.
    fn handle(path: &Path) -> BlockingStack {
        let stack = RawTreiberStack::create(path, 4, LAYOUT).expect("make or attach to the stack");
        BlockingStack::beside_file(stack, path, MAX_WAITERS_DEFAULT).expect("its wakers")
    }

    /// Push until the stack refuses, so the next push must wait for room.
    fn fill(stack: &BlockingStack) {
        loop {
            match stack.try_push(b"x") {
                Ok(()) => {}
                Err(StackError::Full) => break,
                Err(e) => panic!("filling the stack: {e}"),
            }
        }
    }

    /// A thread pushing onto a full stack through `stack`, which reports
    /// what its push came to and how long it took, returned once its wait
    /// has begun.
    #[allow(clippy::type_complexity)]
    fn waiting_pusher(
        stack: &Arc<BlockingStack>,
        timeout: Duration,
    ) -> std::thread::JoinHandle<(Result<(), BlockingWaitError<StackError>>, Duration)> {
        let waiting = Arc::clone(stack);
        let returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pusher_returned = Arc::clone(&returned);
        let pusher = std::thread::spawn(move || {
            let began = Instant::now();
            let pushed = waiting.push_blocking(b"late", Some(timeout));
            pusher_returned.store(true, std::sync::atomic::Ordering::Release);
            (pushed, began.elapsed())
        });
        stack.producer_waker().until_a_wait_begins(&returned);
        pusher
    }

    #[test]
    fn a_wait_for_an_item_times_out_and_takes_one_once_one_is_there() {
        let folder = Folder::new("timeout");
        let stack = RawTreiberStack::create(folder.path(), 4, LAYOUT).expect("make the stack");
        let blocking = BlockingStack::anon(stack, MAX_WAITERS_DEFAULT).expect("its wakers");
        let mut out = [0u8; 8];
        let began = Instant::now();
        assert!(matches!(
            blocking.pop_blocking(&mut out, Some(Duration::from_millis(50))),
            Err(BlockingWaitError::Timeout)
        ));
        assert!(began.elapsed() >= Duration::from_millis(40), "the wait lasted its timeout");
        blocking.try_push(b"one").expect("push");
        blocking.pop_blocking(&mut out, Some(Duration::from_secs(5))).expect("pop");
        assert_eq!(&out[..3], b"one");
    }

    #[test]
    fn a_push_through_another_handle_on_the_file_wakes_a_waiting_popper() {
        let folder = Folder::new("wake");
        let path = folder.path();
        let popping = Arc::new(handle(&path));
        let pushing = handle(&path);
        let waiting = Arc::clone(&popping);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; 8];
            let began = Instant::now();
            let taken = waiting.pop_blocking(&mut out, Some(Duration::from_secs(20)));
            (taken, out, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        pushing.try_push(b"pushed").expect("push");
        let (taken, item, waited) = waiter.join().expect("the waiter thread");
        taken.expect("the woken pop takes the item");
        assert_eq!(&item[..6], b"pushed");
        assert!(waited < Duration::from_secs(10), "woken by the push, not by the timeout: {waited:?}");
    }

    #[test]
    fn a_pop_through_another_handle_wakes_a_pusher_waiting_for_room() {
        let folder = Folder::new("room");
        let path = folder.path();
        let pushing = Arc::new(handle(&path));
        let popping = handle(&path);
        fill(&pushing);
        let waiter = waiting_pusher(&pushing, Duration::from_secs(20));
        let mut out = [0u8; 8];
        popping.try_pop(&mut out).expect("make room");
        let (pushed, waited) = waiter.join().expect("the pusher thread");
        pushed.expect("the woken push lands in the freed slot");
        assert!(waited < Duration::from_secs(10), "woken by the pop, not by the timeout: {waited:?}");
    }

    #[test]
    fn a_pop_through_the_stack_alone_leaves_that_pusher_asleep() {
        let folder = Folder::new("alone");
        let path = folder.path();
        let pushing = Arc::new(handle(&path));
        let popping = handle(&path);
        fill(&pushing);
        let timeout = Duration::from_millis(600);
        let waiter = waiting_pusher(&pushing, timeout);
        let mut out = [0u8; 8];
        popping.stack().pop(&mut out).expect("make room without waking anyone");
        let (pushed, waited) = waiter.join().expect("the pusher thread");
        // Its wait ends at the timeout, and the try that follows finds the
        // room the silent pop made.
        pushed.expect("the push lands once its wait ends");
        assert!(waited >= timeout - Duration::from_millis(50), "slept until its timeout: {waited:?}");
    }

    #[test]
    fn closing_ends_a_wait_on_that_handle() {
        let folder = Folder::new("close");
        let stack = RawTreiberStack::create(folder.path(), 4, LAYOUT).expect("make the stack");
        let blocking = Arc::new(BlockingStack::anon(stack, MAX_WAITERS_DEFAULT).expect("its wakers"));
        let waiting = Arc::clone(&blocking);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; 8];
            waiting.pop_blocking(&mut out, None)
        });
        std::thread::sleep(Duration::from_millis(50));
        blocking.close();
        assert!(matches!(waiter.join().expect("the waiter thread"), Err(BlockingWaitError::Closed)));
    }
}
