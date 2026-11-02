//! A value taken at most once, by whichever thread swaps it out first.

use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};

/// A value stored until one caller takes it; every later take returns
/// `None`. One atomic swap, so no thread ever waits on another.
pub(crate) struct TakeOnce<T>(AtomicPtr<T>);

impl<T> TakeOnce<T> {
    pub(crate) fn new(value: T) -> TakeOnce<T> {
        TakeOnce(AtomicPtr::new(Box::into_raw(Box::new(value))))
    }

    pub(crate) fn take(&self) -> Option<T> {
        let taken = self.0.swap(null_mut(), Ordering::AcqRel);
        if taken.is_null() {
            None
        } else {
            // SAFETY: a non-null pointer came from Box::into_raw in `new`,
            // and the swap gave it to this caller alone.
            Some(*unsafe { Box::from_raw(taken) })
        }
    }
}

impl<T> Drop for TakeOnce<T> {
    fn drop(&mut self) {
        drop(self.take());
    }
}

// SAFETY: the value moves to exactly one taker, so sharing the cell
// shares nothing but the right to take it.
unsafe impl<T: Send> Send for TakeOnce<T> {}
unsafe impl<T: Send> Sync for TakeOnce<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn the_value_goes_to_one_taker() {
        for _ in 0..100 {
            let cell = Arc::new(TakeOnce::new(String::from("once")));
            let takers: Vec<_> = (0..4)
                .map(|_| {
                    let cell = Arc::clone(&cell);
                    std::thread::spawn(move || cell.take())
                })
                .collect();
            let got: Vec<String> = takers.into_iter().filter_map(|t| t.join().expect("a taking thread")).collect();
            assert_eq!(got, vec![String::from("once")]);
        }
    }
}
