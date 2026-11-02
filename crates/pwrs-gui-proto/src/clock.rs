//! A clock every process on the machine reads alike, so an input taken in
//! a renderer can be timed against its effect in the session.

/// Nanoseconds on a monotonic clock shared by every process on the
/// machine: QueryPerformanceCounter on Windows, `CLOCK_MONOTONIC`
/// elsewhere. Its zero is arbitrary; only differences mean anything.
pub fn now_ns() -> u64 {
    imp::now_ns()
}

#[cfg(windows)]
mod imp {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    /// Counter ticks per second, fixed at boot.
    fn frequency() -> u128 {
        static FREQUENCY: OnceLock<u128> = OnceLock::new();
        *FREQUENCY.get_or_init(|| {
            let mut f = 0i64;
            // Succeeds on every Windows since XP.
            unsafe { QueryPerformanceFrequency(&mut f) };
            u128::try_from(f).expect("the performance counter's frequency is positive")
        })
    }

    pub fn now_ns() -> u64 {
        let mut ticks = 0i64;
        // Succeeds on every Windows since XP.
        unsafe { QueryPerformanceCounter(&mut ticks) };
        let ticks = u128::try_from(ticks).expect("the performance counter reads positive");
        u64::try_from(ticks * 1_000_000_000 / frequency()).expect("nanoseconds since boot fit 64 bits")
    }
}

#[cfg(unix)]
mod imp {
    pub fn now_ns() -> u64 {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        assert_eq!(rc, 0, "CLOCK_MONOTONIC cannot be read: {}", std::io::Error::last_os_error());
        let secs = u64::try_from(ts.tv_sec).expect("CLOCK_MONOTONIC reads positive");
        let nanos = u64::try_from(ts.tv_nsec).expect("CLOCK_MONOTONIC's nanoseconds read positive");
        secs * 1_000_000_000 + nanos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_moves_forward_with_a_sleep() {
        let before = now_ns();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let after = now_ns();
        assert!(after > before, "{after} is not after {before}");
        assert!(after - before >= 15_000_000, "a 20 ms sleep read as {} ns", after - before);
    }
}
