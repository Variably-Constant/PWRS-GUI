//! Copies into and out of shared memory that another participant may read
//! or write while the copy runs, a whole element at a time, so a copy that
//! races a write is no data race: each element comes out wholly old or
//! wholly new, and the bytes copied out hold still once copied. A single
//! 8-byte word moves as a relaxed `AtomicU64` access, which the compiler
//! keeps whole and otherwise orders freely; a copy of several elements moves
//! each by one single-copy-atomic load or store inside `asm!`, which the
//! compiler treats as an opaque access. Whether the elements make one value
//! is the caller's to confirm (a generation, a head, a sequence) before it
//! reads them as a `T`.
//!
//! A load reads 16-byte elements where one plain 16-byte load is atomic
//! ([`plain_sixteen`](crate::atomic_pair::plain_sixteen)) and the source is
//! 16-byte aligned, and 8-byte elements otherwise. A store writes 8-byte
//! elements and a tail of 4, 2 and 1 bytes, so it writes exactly the bytes
//! asked for. A source or destination in shared memory is 8-byte aligned.

use std::mem::{size_of, MaybeUninit};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::atomic_pair::plain_sixteen;

/// One aligned 8-byte word, read whole as a relaxed atomic load, which the
/// compiler orders like any other rather than as a barrier.
///
/// # Safety
/// `src` is 8-byte aligned and readable for 8 bytes.
#[inline(always)]
pub(crate) unsafe fn load_word(src: *const u64) -> u64 {
    // SAFETY: the caller's contract; the word is aligned and live.
    unsafe { (*(src as *const AtomicU64)).load(Ordering::Relaxed) }
}

/// Write `value` as one aligned 8-byte word, whole, as a relaxed atomic
/// store.
///
/// # Safety
/// `dst` is 8-byte aligned and writable for 8 bytes.
#[inline(always)]
pub(crate) unsafe fn store_word(dst: *mut u64, value: u64) {
    // SAFETY: the caller's contract; the word is aligned and live.
    unsafe { (*(dst as *const AtomicU64)).store(value, Ordering::Relaxed) }
}

/// `len` bytes from `src` into `dst`, a whole element at a time.
///
/// # Safety
/// `src` is 8-byte aligned and readable for `len` bytes, `len` is a multiple
/// of 8, and `dst` is writable for `len` bytes and touched by nothing else
/// during the copy.
#[inline]
pub(crate) unsafe fn load_bytes(dst: *mut u8, src: *const u8, len: usize) {
    debug_assert!(src.addr().is_multiple_of(8) && len.is_multiple_of(8));
    let mut done = 0;
    if len >= 16 && src.addr().is_multiple_of(16) && plain_sixteen() {
        done = len & !15;
        // SAFETY: the caller's contract, `src` 16-byte aligned, `done` a
        // positive multiple of 16, and one plain 16-byte load atomic here.
        unsafe { imp::load16(dst, src, done) };
    }
    if done < len {
        // SAFETY: the caller's contract; the rest is a positive multiple of 8
        // from an 8-byte aligned address.
        unsafe { imp::load8(dst.add(done), src.add(done), len - done) };
    }
}

/// The `T` at `src` as bytes copied a whole element at a time: a value that
/// is a `T` only once the caller's check confirms it.
///
/// # Safety
/// `src` is 8-byte aligned, and the `size_of::<T>()` bytes at it, rounded up
/// to a multiple of 8, are readable.
#[inline(always)]
pub(crate) unsafe fn load<T>(src: *const T) -> MaybeUninit<T> {
    let size = size_of::<T>();
    let mut out = MaybeUninit::<T>::uninit();
    if size == 0 {
        return out;
    }
    if size <= 8 {
        // SAFETY: the caller's contract covers the rounded-up word.
        let word = unsafe { load_word(src as *const u64) };
        // SAFETY: the word's first `size` bytes are the value's, in memory
        // order, and `out` holds `size` bytes.
        unsafe { ptr::copy_nonoverlapping(&word as *const u64 as *const u8, out.as_mut_ptr() as *mut u8, size) };
    } else if size <= 64 {
        let dst = out.as_mut_ptr() as *mut u8;
        // SAFETY: the caller's contract covers `size` rounded up to whole
        // words, which is what each path reads, and `out` holds `size` bytes.
        unsafe {
            if size >= 16 && src.addr().is_multiple_of(16) && plain_sixteen() {
                imp::load_lanes(dst, src as *const u8, size);
            } else {
                imp::load_words(dst, src as *const u8, size);
            }
        }
    } else if size.is_multiple_of(8) {
        // SAFETY: the caller's contract; `out` is private and `size` long.
        unsafe { load_bytes(out.as_mut_ptr() as *mut u8, src as *const u8, size) };
    } else {
        let mut words = MaybeUninit::<Rounded<T>>::uninit();
        // SAFETY: the caller's contract; `Rounded<T>` holds `size` rounded up
        // to 8 and is private.
        unsafe { load_bytes(words.as_mut_ptr() as *mut u8, src as *const u8, size.next_multiple_of(8)) };
        // SAFETY: both hold at least `size` bytes.
        unsafe { ptr::copy_nonoverlapping(words.as_ptr() as *const u8, out.as_mut_ptr() as *mut u8, size) };
    }
    out
}

/// [`load`] of a `T` at a 16-byte aligned `src`, taking a value of 16 to 64
/// bytes in 16-byte lanes when `lanes` is true and in 8-byte words when it
/// is false, without asking the processor which.
///
/// # Safety
/// As for [`load`]; `src` is 16-byte aligned, and `lanes` is true only where
/// one plain 16-byte load is atomic, as [`crate::atomic_pair::plain_sixteen`]
/// returns.
#[inline(always)]
pub(crate) unsafe fn load_aligned<T>(src: *const T, lanes: bool) -> MaybeUninit<T> {
    let size = size_of::<T>();
    if !(16..=64).contains(&size) {
        // SAFETY: the caller's contract.
        return unsafe { load(src) };
    }
    debug_assert!(src.addr().is_multiple_of(16));
    let mut out = MaybeUninit::<T>::uninit();
    let dst = out.as_mut_ptr() as *mut u8;
    // SAFETY: the caller's contract covers `size` rounded up to whole words,
    // which each path reads, and `out` holds `size` bytes.
    unsafe {
        if lanes {
            imp::load_lanes(dst, src as *const u8, size);
        } else {
            imp::load_words(dst, src as *const u8, size);
        }
    }
    out
}

/// [`load`] with whether one plain 16-byte load is atomic here given as
/// `lanes`: 16-byte lanes where `src` is 16-byte aligned and `lanes` is true,
/// 8-byte words elsewhere.
///
/// # Safety
/// As for [`load`]; `lanes` is true only where one plain 16-byte load is
/// atomic, as [`crate::atomic_pair::plain_sixteen`] returns.
#[inline(always)]
pub(crate) unsafe fn load_with_lanes<T>(src: *const T, lanes: bool) -> MaybeUninit<T> {
    let size = size_of::<T>();
    if !(16..=64).contains(&size) {
        // SAFETY: the caller's contract.
        return unsafe { load(src) };
    }
    let mut out = MaybeUninit::<T>::uninit();
    let dst = out.as_mut_ptr() as *mut u8;
    // SAFETY: the caller's contract covers `size` rounded up to whole words,
    // which each path reads, the lanes read only an aligned `src`, and `out`
    // holds `size` bytes.
    unsafe {
        if lanes && src.addr().is_multiple_of(16) {
            imp::load_lanes(dst, src as *const u8, size);
        } else {
            imp::load_words(dst, src as *const u8, size);
        }
    }
    out
}

/// A `T` and room past it for the copy's rounding up to whole words.
#[repr(C)]
struct Rounded<T> {
    _value: MaybeUninit<T>,
    _rest: MaybeUninit<u64>,
}

/// `len` bytes from `src` into `dst`, exactly, a whole element at a time.
///
/// # Safety
/// `dst` is 8-byte aligned and writable for `len` bytes, and `src` is
/// readable for `len` bytes and touched by nothing else during the copy.
#[inline]
pub(crate) unsafe fn store_bytes(dst: *mut u8, src: *const u8, len: usize) {
    debug_assert!(dst.addr().is_multiple_of(8));
    let words = len & !7;
    if words > 0 {
        // SAFETY: the caller's contract, `words` a positive multiple of 8.
        unsafe { imp::store8(dst, src, words) };
    }
    // SAFETY: the caller's contract; the tail starts 8-byte aligned.
    unsafe { store_tail(dst.add(words), src.add(words), len - words) };
}

/// The last `rest` bytes of a store, fewer than 8, as 4-, 2- and 1-byte
/// elements.
///
/// # Safety
/// `dst` is 8-byte aligned and writable, and `src` readable, for `rest`
/// bytes, and `rest` is below 8.
#[inline(always)]
unsafe fn store_tail(dst: *mut u8, src: *const u8, mut rest: usize) {
    let mut at = 0;
    // SAFETY: each store lies inside `rest` bytes at an address aligned to
    // its own size, since `dst` is 8-byte aligned and the tail runs 4, 2,
    // then 1.
    unsafe {
        if rest >= 4 {
            imp::store4(dst, ptr::read_unaligned(src as *const u32));
            at = 4;
            rest -= 4;
        }
        if rest >= 2 {
            imp::store2(dst.add(at), ptr::read_unaligned(src.add(at) as *const u16));
            at += 2;
            rest -= 2;
        }
        if rest == 1 {
            imp::store1(dst.add(at), *src.add(at));
        }
    }
}

/// Write `value` at `dst`, exactly its bytes, a whole element at a time: a
/// value of 8 to 64 bytes as one block of 8-byte stores from registers, then
/// its tail.
///
/// # Safety
/// `dst` is 8-byte aligned and writable for `size_of::<T>()` bytes.
#[inline(always)]
pub(crate) unsafe fn store<T>(dst: *mut T, value: &T) {
    let size = size_of::<T>();
    let src = value as *const T as *const u8;
    let dst = dst as *mut u8;
    // SAFETY: the caller's contract; `value` is a live `T` of the same size,
    // which nothing else touches, so plain loads read it.
    unsafe {
        if (8..=64).contains(&size) {
            let words = size / 8;
            imp::store_words(dst, src, words);
            store_tail(dst.add(words * 8), src.add(words * 8), size - words * 8);
        } else {
            store_bytes(dst, src, size);
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use core::arch::asm;
    use core::arch::x86_64::{__m128i, _mm_storeu_si128};
    use std::ptr;

    /// One asm block of 16-byte `movdqa` loads then 8-byte `mov` loads, each
    /// at its displacement from `$src`, into registers; then the lanes, and
    /// `$rest` bytes of the words after them, into `$dst` by plain stores the
    /// compiler sees. The block reads memory and writes none, so values the
    /// compiler holds around it stay in registers.
    macro_rules! lanes {
        ($dst:ident, $src:ident, $rest:expr; [$($x:ident @ $xo:literal),+]; [$($w:ident @ $wo:literal),*]) => {{
            $(let $x: __m128i;)+
            $(let $w: u64;)*
            asm!(
                $(concat!("movdqa {", stringify!($x), "}, xmmword ptr [{s} + ", stringify!($xo), "]"),)+
                $(concat!("mov {", stringify!($w), "}, qword ptr [{s} + ", stringify!($wo), "]"),)*
                s = in(reg) $src,
                $($x = out(xmm_reg) $x,)+
                $($w = out(reg) $w,)*
                options(nostack, readonly, preserves_flags),
            );
            let mut at = 0usize;
            $(
                _mm_storeu_si128($dst.add(at) as *mut __m128i, $x);
                at += 16;
            )+
            let words: &[u64] = &[$($w),*];
            ptr::copy_nonoverlapping(words.as_ptr() as *const u8, $dst.add(at), $rest);
        }};
    }

    /// The `size` bytes at `src`, 16 to 64 of them, into `dst`: whole 16-byte
    /// lanes, then the rest by whole 8-byte words, in one asm block.
    ///
    /// # Safety
    /// `src` is 16-byte aligned and readable for `size` rounded up to a
    /// multiple of 8, one plain 16-byte load is atomic on this processor,
    /// `dst` is writable for `size` bytes, and `size` is 16 to 64.
    #[inline(always)]
    pub(super) unsafe fn load_lanes(dst: *mut u8, src: *const u8, size: usize) {
        let full = size / 16;
        let rest = size - full * 16;
        // SAFETY: the caller's contract; each arm reads `full` lanes and
        // `rest` rounded up to whole words, and writes `size` bytes.
        unsafe {
            match (full, rest.div_ceil(8)) {
                (1, 0) => lanes!(dst, src, rest; [a @ 0]; []),
                (1, 1) => lanes!(dst, src, rest; [a @ 0]; [p @ 16]),
                (1, _) => lanes!(dst, src, rest; [a @ 0]; [p @ 16, q @ 24]),
                (2, 0) => lanes!(dst, src, rest; [a @ 0, b @ 16]; []),
                (2, 1) => lanes!(dst, src, rest; [a @ 0, b @ 16]; [p @ 32]),
                (2, _) => lanes!(dst, src, rest; [a @ 0, b @ 16]; [p @ 32, q @ 40]),
                (3, 0) => lanes!(dst, src, rest; [a @ 0, b @ 16, c @ 32]; []),
                (3, 1) => lanes!(dst, src, rest; [a @ 0, b @ 16, c @ 32]; [p @ 48]),
                (3, _) => lanes!(dst, src, rest; [a @ 0, b @ 16, c @ 32]; [p @ 48, q @ 56]),
                _ => lanes!(dst, src, rest; [a @ 0, b @ 16, c @ 32, d @ 48]; []),
            }
        }
    }

    /// One asm block of 8-byte `mov` loads, each at its displacement from
    /// `$src`, then `$size` bytes of them into `$dst` by plain stores.
    macro_rules! words {
        ($dst:ident, $src:ident, $size:expr; $($w:ident @ $wo:literal),+) => {{
            $(let $w: u64;)+
            asm!(
                $(concat!("mov {", stringify!($w), "}, qword ptr [{s} + ", stringify!($wo), "]"),)+
                s = in(reg) $src,
                $($w = out(reg) $w,)+
                options(nostack, readonly, preserves_flags),
            );
            let words = [$($w),+];
            ptr::copy_nonoverlapping(words.as_ptr() as *const u8, $dst, $size);
        }};
    }

    /// The `size` bytes at `src`, 9 to 64 of them, into `dst`, by whole 8-byte
    /// words in one asm block.
    ///
    /// # Safety
    /// `src` is 8-byte aligned and readable for `size` rounded up to a
    /// multiple of 8, `dst` is writable for `size` bytes, and `size` is 9 to
    /// 64.
    #[inline(always)]
    pub(super) unsafe fn load_words(dst: *mut u8, src: *const u8, size: usize) {
        // SAFETY: the caller's contract; each arm reads `size` rounded up to
        // whole words and writes `size` bytes.
        unsafe {
            match size.div_ceil(8) {
                2 => words!(dst, src, size; a @ 0, b @ 8),
                3 => words!(dst, src, size; a @ 0, b @ 8, c @ 16),
                4 => words!(dst, src, size; a @ 0, b @ 8, c @ 16, d @ 24),
                5 => words!(dst, src, size; a @ 0, b @ 8, c @ 16, d @ 24, e @ 32),
                6 => words!(dst, src, size; a @ 0, b @ 8, c @ 16, d @ 24, e @ 32, f @ 40),
                7 => words!(dst, src, size; a @ 0, b @ 8, c @ 16, d @ 24, e @ 32, f @ 40, g @ 48),
                _ => words!(dst, src, size; a @ 0, b @ 8, c @ 16, d @ 24, e @ 32, f @ 40, g @ 48, h @ 56),
            }
        }
    }

    /// One asm block of 8-byte `mov` stores of `$w`, words already in
    /// registers, each at its displacement from `$dst`.
    macro_rules! store_block {
        ($dst:ident; $($w:ident @ $wo:literal),+) => {
            asm!(
                $(concat!("mov qword ptr [{base} + ", stringify!($wo), "], {", stringify!($w), "}"),)+
                base = in(reg) $dst,
                $($w = in(reg) $w,)+
                options(nostack, preserves_flags),
            )
        };
    }

    /// The first `words` 8-byte words at `src` into aligned 8-byte elements
    /// at `dst`, read by plain loads and stored in one asm block.
    ///
    /// # Safety
    /// `src` is readable, and touched by nothing else, and `dst` 8-byte
    /// aligned and writable, for `words * 8` bytes, and `words` is 1 to 8.
    #[inline(always)]
    pub(super) unsafe fn store_words(dst: *mut u8, src: *const u8, words: usize) {
        // SAFETY: the caller's contract, `i` below `words`.
        let w = |i: usize| unsafe { ptr::read_unaligned(src.add(i * 8) as *const u64) };
        // SAFETY: the caller's contract; each arm stores `words` words.
        unsafe {
            match words {
                1 => {
                    let a = w(0);
                    store_block!(dst; a @ 0)
                }
                2 => {
                    let (a, b) = (w(0), w(1));
                    store_block!(dst; a @ 0, b @ 8)
                }
                3 => {
                    let (a, b, c) = (w(0), w(1), w(2));
                    store_block!(dst; a @ 0, b @ 8, c @ 16)
                }
                4 => {
                    let (a, b, c, d) = (w(0), w(1), w(2), w(3));
                    store_block!(dst; a @ 0, b @ 8, c @ 16, d @ 24)
                }
                5 => {
                    let (a, b, c, d, e) = (w(0), w(1), w(2), w(3), w(4));
                    store_block!(dst; a @ 0, b @ 8, c @ 16, d @ 24, e @ 32)
                }
                6 => {
                    let (a, b, c, d, e, f) = (w(0), w(1), w(2), w(3), w(4), w(5));
                    store_block!(dst; a @ 0, b @ 8, c @ 16, d @ 24, e @ 32, f @ 40)
                }
                7 => {
                    let (a, b, c, d, e, f, g) = (w(0), w(1), w(2), w(3), w(4), w(5), w(6));
                    store_block!(dst; a @ 0, b @ 8, c @ 16, d @ 24, e @ 32, f @ 40, g @ 48)
                }
                _ => {
                    let (a, b, c, d, e, f, g, h) = (w(0), w(1), w(2), w(3), w(4), w(5), w(6), w(7));
                    store_block!(dst; a @ 0, b @ 8, c @ 16, d @ 24, e @ 32, f @ 40, g @ 48, h @ 56)
                }
            }
        }
    }

    /// # Safety
    /// `len` is a positive multiple of 8, `src` 8-byte aligned and readable
    /// and `dst` writable for `len` bytes.
    #[inline]
    pub(super) unsafe fn load8(dst: *mut u8, src: *const u8, len: usize) {
        // SAFETY: the caller's contract; each element is one aligned 8-byte
        // `mov`.
        unsafe {
            asm!(
                "2:",
                "mov {t}, qword ptr [{src}]",
                "mov qword ptr [{dst}], {t}",
                "add {src}, 8",
                "add {dst}, 8",
                "sub {n}, 8",
                "jnz 2b",
                src = inout(reg) src => _,
                dst = inout(reg) dst => _,
                n = inout(reg) len => _,
                t = out(reg) _,
                options(nostack),
            );
        }
    }

    /// # Safety
    /// `len` is a positive multiple of 16, `src` 16-byte aligned and
    /// readable, `dst` writable for `len` bytes, and the processor has AVX.
    #[inline]
    pub(super) unsafe fn load16(dst: *mut u8, src: *const u8, len: usize) {
        // SAFETY: the caller's contract; Intel and AMD guarantee an aligned
        // 16-byte `vmovdqa` load atomic on every processor with AVX, and the
        // private destination takes an unaligned `vmovdqu`.
        unsafe {
            asm!(
                "2:",
                "vmovdqa {x}, xmmword ptr [{src}]",
                "vmovdqu xmmword ptr [{dst}], {x}",
                "add {src}, 16",
                "add {dst}, 16",
                "sub {n}, 16",
                "jnz 2b",
                src = inout(reg) src => _,
                dst = inout(reg) dst => _,
                n = inout(reg) len => _,
                x = out(xmm_reg) _,
                options(nostack),
            );
        }
    }

    /// # Safety
    /// `len` is a positive multiple of 8, `dst` 8-byte aligned and writable
    /// and `src` readable for `len` bytes.
    #[inline]
    pub(super) unsafe fn store8(dst: *mut u8, src: *const u8, len: usize) {
        // SAFETY: the caller's contract; each element is one aligned 8-byte
        // `mov` into `dst`.
        unsafe {
            asm!(
                "2:",
                "mov {t}, qword ptr [{src}]",
                "mov qword ptr [{dst}], {t}",
                "add {src}, 8",
                "add {dst}, 8",
                "sub {n}, 8",
                "jnz 2b",
                src = inout(reg) src => _,
                dst = inout(reg) dst => _,
                n = inout(reg) len => _,
                t = out(reg) _,
                options(nostack),
            );
        }
    }

    /// # Safety
    /// `dst` is 4-byte aligned and writable for 4 bytes.
    #[inline(always)]
    pub(super) unsafe fn store4(dst: *mut u8, v: u32) {
        // SAFETY: the caller's contract.
        unsafe { asm!("mov dword ptr [{dst}], {v:e}", dst = in(reg) dst, v = in(reg) v, options(nostack, preserves_flags)) };
    }

    /// # Safety
    /// `dst` is 2-byte aligned and writable for 2 bytes.
    #[inline(always)]
    pub(super) unsafe fn store2(dst: *mut u8, v: u16) {
        // SAFETY: the caller's contract.
        unsafe { asm!("mov word ptr [{dst}], {v:x}", dst = in(reg) dst, v = in(reg) v, options(nostack, preserves_flags)) };
    }

    /// # Safety
    /// `dst` is writable for 1 byte.
    #[inline(always)]
    pub(super) unsafe fn store1(dst: *mut u8, v: u8) {
        // SAFETY: the caller's contract.
        unsafe { asm!("mov byte ptr [{dst}], {v}", dst = in(reg) dst, v = in(reg_byte) v, options(nostack, preserves_flags)) };
    }
}

#[cfg(target_arch = "aarch64")]
mod imp {
    use core::arch::asm;

    /// As for the x86-64 `load_reg`, by one `ldr`.
    ///
    /// # Safety
    /// As for the x86-64 `load_reg`.
    #[inline(always)]
    pub(super) unsafe fn load_reg(src: *const u64) -> u64 {
        let v;
        // SAFETY: the caller's contract; one aligned 8-byte `ldr`.
        unsafe { asm!("ldr {v}, [{s}]", s = in(reg) src, v = out(reg) v, options(nostack, readonly, preserves_flags)) };
        v
    }

    /// The 8 bytes at `src` into the aligned 8-byte element at `dst`, by one
    /// `ldr` and one `str`.
    ///
    /// # Safety
    /// `src` is readable and `dst` 8-byte aligned and writable for 8 bytes.
    #[inline(always)]
    pub(super) unsafe fn copy_word(dst: *mut u64, src: *const u64) {
        // SAFETY: the caller's contract; the store is one aligned 8-byte `str`.
        unsafe {
            asm!(
                "ldr {t}, [{s}]",
                "str {t}, [{d}]",
                s = in(reg) src,
                d = in(reg) dst,
                t = out(reg) _,
                options(nostack, preserves_flags),
            )
        };
    }

    /// As for the x86-64 `load_lanes`: whole 16-byte pairs by `ldp`, then the
    /// rest by whole 8-byte words.
    ///
    /// # Safety
    /// As for the x86-64 `load_lanes`, on a processor with LSE2.
    #[inline(always)]
    pub(super) unsafe fn load_lanes(dst: *mut u8, src: *const u8, size: usize) {
        let full = size & !15;
        // SAFETY: the caller's contract; `full` is a positive multiple of 16.
        unsafe { load16(dst, src, full) };
        let rest = size - full;
        if rest > 0 {
            let mut words = [0u64; 2];
            for (i, word) in words.iter_mut().enumerate().take(rest.div_ceil(8)) {
                // SAFETY: the caller's contract covers `rest` rounded up to
                // whole words past the lanes.
                *word = unsafe { load_reg(src.add(full + i * 8) as *const u64) };
            }
            // SAFETY: `dst` holds `size` bytes, `rest` of them past the lanes.
            unsafe { std::ptr::copy_nonoverlapping(words.as_ptr() as *const u8, dst.add(full), rest) };
        }
    }

    /// As for the x86-64 `load_words`.
    ///
    /// # Safety
    /// As for the x86-64 `load_words`.
    #[inline(always)]
    pub(super) unsafe fn load_words(dst: *mut u8, src: *const u8, size: usize) {
        let mut words = [0u64; 8];
        for (i, word) in words.iter_mut().enumerate().take(size.div_ceil(8)) {
            // SAFETY: the caller's contract covers `size` rounded up to words.
            *word = unsafe { load_reg(src.add(i * 8) as *const u64) };
        }
        // SAFETY: `dst` holds `size` bytes, at most the 64 `words` holds.
        unsafe { std::ptr::copy_nonoverlapping(words.as_ptr() as *const u8, dst, size) };
    }

    /// As for the x86-64 `store_words`, a word at a time.
    ///
    /// # Safety
    /// As for the x86-64 `store_words`.
    #[inline(always)]
    pub(super) unsafe fn store_words(dst: *mut u8, src: *const u8, words: usize) {
        for i in 0..words {
            // SAFETY: the caller's contract, one element per word.
            unsafe { copy_word(dst.add(i * 8) as *mut u64, src.add(i * 8) as *const u64) };
        }
    }

    /// # Safety
    /// As for the x86-64 `load8`.
    #[inline]
    pub(super) unsafe fn load8(dst: *mut u8, src: *const u8, len: usize) {
        // SAFETY: the caller's contract; each element is one aligned 8-byte
        // `ldr`.
        unsafe {
            asm!(
                "2:",
                "ldr {t}, [{src}], #8",
                "str {t}, [{dst}], #8",
                "subs {n}, {n}, #8",
                "b.ne 2b",
                src = inout(reg) src => _,
                dst = inout(reg) dst => _,
                n = inout(reg) len => _,
                t = out(reg) _,
                options(nostack),
            );
        }
    }

    /// # Safety
    /// As for the x86-64 `load16`, on a processor with LSE2.
    #[inline]
    pub(super) unsafe fn load16(dst: *mut u8, src: *const u8, len: usize) {
        // SAFETY: the caller's contract; with LSE2 an aligned `ldp` of two
        // registers is single-copy atomic.
        unsafe {
            asm!(
                "2:",
                "ldp {a}, {b}, [{src}], #16",
                "stp {a}, {b}, [{dst}], #16",
                "subs {n}, {n}, #16",
                "b.ne 2b",
                src = inout(reg) src => _,
                dst = inout(reg) dst => _,
                n = inout(reg) len => _,
                a = out(reg) _,
                b = out(reg) _,
                options(nostack),
            );
        }
    }

    /// # Safety
    /// As for the x86-64 `store8`.
    #[inline]
    pub(super) unsafe fn store8(dst: *mut u8, src: *const u8, len: usize) {
        // SAFETY: the caller's contract; each element is one aligned 8-byte
        // `str` into `dst`.
        unsafe {
            asm!(
                "2:",
                "ldr {t}, [{src}], #8",
                "str {t}, [{dst}], #8",
                "subs {n}, {n}, #8",
                "b.ne 2b",
                src = inout(reg) src => _,
                dst = inout(reg) dst => _,
                n = inout(reg) len => _,
                t = out(reg) _,
                options(nostack),
            );
        }
    }

    /// # Safety
    /// As for the x86-64 `store4`.
    #[inline(always)]
    pub(super) unsafe fn store4(dst: *mut u8, v: u32) {
        // SAFETY: the caller's contract.
        unsafe { asm!("str {v:w}, [{dst}]", dst = in(reg) dst, v = in(reg) v, options(nostack, preserves_flags)) };
    }

    /// # Safety
    /// As for the x86-64 `store2`.
    #[inline(always)]
    pub(super) unsafe fn store2(dst: *mut u8, v: u16) {
        // SAFETY: the caller's contract.
        unsafe { asm!("strh {v:w}, [{dst}]", dst = in(reg) dst, v = in(reg) v, options(nostack, preserves_flags)) };
    }

    /// # Safety
    /// As for the x86-64 `store1`.
    #[inline(always)]
    pub(super) unsafe fn store1(dst: *mut u8, v: u8) {
        // SAFETY: the caller's contract.
        unsafe { asm!("strb {v:w}, [{dst}]", dst = in(reg) dst, v = in(reg) v, options(nostack, preserves_flags)) };
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
mod imp {
    use std::sync::atomic::{AtomicU8, AtomicU16, AtomicU32, AtomicU64, Ordering};

    /// # Safety
    /// As for [`super::load_word`].
    #[inline(always)]
    pub(super) unsafe fn load_word(src: *const u64) -> u64 {
        // SAFETY: the caller's contract; the word is aligned and live.
        unsafe { (*(src as *const AtomicU64)).load(Ordering::Relaxed) }
    }

    /// The 8 bytes at `src` into the aligned 8-byte element at `dst`, as one
    /// relaxed atomic store.
    ///
    /// # Safety
    /// `src` is readable and `dst` 8-byte aligned and writable for 8 bytes.
    #[inline(always)]
    pub(super) unsafe fn copy_word(dst: *mut u64, src: *const u64) {
        // SAFETY: the caller's contract.
        unsafe { (*(dst as *const AtomicU64)).store(src.read_unaligned(), Ordering::Relaxed) }
    }

    /// # Safety
    /// Never called: no plain 16-byte load is atomic on these targets.
    pub(super) unsafe fn load_lanes(_dst: *mut u8, _src: *const u8, _size: usize) {
        unreachable!("no single-copy-atomic 16-byte load on this architecture")
    }

    /// As for the x86-64 `load_words`, a relaxed atomic load per word.
    ///
    /// # Safety
    /// As for the x86-64 `load_words`.
    #[inline(always)]
    pub(super) unsafe fn load_words(dst: *mut u8, src: *const u8, size: usize) {
        let mut words = [0u64; 8];
        for (i, word) in words.iter_mut().enumerate().take(size.div_ceil(8)) {
            // SAFETY: the caller's contract covers `size` rounded up to words.
            *word = unsafe { load_word(src.add(i * 8) as *const u64) };
        }
        // SAFETY: `dst` holds `size` bytes, at most the 64 `words` holds.
        unsafe { std::ptr::copy_nonoverlapping(words.as_ptr() as *const u8, dst, size) };
    }

    /// As for the x86-64 `store_words`, a relaxed atomic store per word.
    ///
    /// # Safety
    /// As for the x86-64 `store_words`.
    #[inline(always)]
    pub(super) unsafe fn store_words(dst: *mut u8, src: *const u8, words: usize) {
        for i in 0..words {
            // SAFETY: the caller's contract, one element per word.
            unsafe { copy_word(dst.add(i * 8) as *mut u64, src.add(i * 8) as *const u64) };
        }
    }

    /// # Safety
    /// As for the x86-64 `load8`.
    #[inline]
    pub(super) unsafe fn load8(dst: *mut u8, src: *const u8, len: usize) {
        let mut at = 0;
        while at < len {
            // SAFETY: the caller's contract; `dst` is private.
            unsafe { (dst.add(at) as *mut u64).write_unaligned(load_word(src.add(at) as *const u64)) };
            at += 8;
        }
    }

    /// # Safety
    /// Never called: no plain 16-byte load is atomic on these targets.
    pub(super) unsafe fn load16(_dst: *mut u8, _src: *const u8, _len: usize) {
        unreachable!("no single-copy-atomic 16-byte load on this architecture")
    }

    /// # Safety
    /// As for the x86-64 `store8`.
    #[inline]
    pub(super) unsafe fn store8(dst: *mut u8, src: *const u8, len: usize) {
        let mut at = 0;
        while at < len {
            // SAFETY: the caller's contract.
            unsafe {
                (*(dst.add(at) as *const AtomicU64)).store((src.add(at) as *const u64).read_unaligned(), Ordering::Relaxed);
            }
            at += 8;
        }
    }

    /// # Safety
    /// As for the x86-64 `store4`.
    #[inline(always)]
    pub(super) unsafe fn store4(dst: *mut u8, v: u32) {
        // SAFETY: the caller's contract.
        unsafe { (*(dst as *const AtomicU32)).store(v, Ordering::Relaxed) };
    }

    /// # Safety
    /// As for the x86-64 `store2`.
    #[inline(always)]
    pub(super) unsafe fn store2(dst: *mut u8, v: u16) {
        // SAFETY: the caller's contract.
        unsafe { (*(dst as *const AtomicU16)).store(v, Ordering::Relaxed) };
    }

    /// # Safety
    /// As for the x86-64 `store1`.
    #[inline(always)]
    pub(super) unsafe fn store1(dst: *mut u8, v: u8) {
        // SAFETY: the caller's contract.
        unsafe { (*(dst as *const AtomicU8)).store(v, Ordering::Relaxed) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A 16-byte aligned scratch region.
    #[repr(C, align(16))]
    struct Region([u8; 256]);

    #[test]
    fn copies_of_every_length_and_offset_carry_the_bytes_exactly() {
        let mut src = Region([0; 256]);
        for (i, b) in src.0.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(37).wrapping_add(11);
        }
        for offset in [0usize, 8, 16, 24] {
            for len in (0..=200).step_by(8) {
                let mut out = [0u8; 256];
                // SAFETY: an 8-byte aligned source of `len` bytes inside the
                // region, and a private destination.
                unsafe { load_bytes(out.as_mut_ptr(), src.0.as_ptr().add(offset), len) };
                assert_eq!(&out[..len], &src.0[offset..offset + len], "load of {len} at {offset}");
            }
            for len in 0..=200 {
                let mut dst = Region([0xEE; 256]);
                // SAFETY: an 8-byte aligned destination of `len` bytes inside
                // the region, and a private source.
                unsafe { store_bytes(dst.0.as_mut_ptr().add(offset), src.0.as_ptr(), len) };
                assert_eq!(&dst.0[offset..offset + len], &src.0[..len], "store of {len} at {offset}");
                assert!(dst.0[offset + len..].iter().all(|&b| b == 0xEE), "store of {len} at {offset} wrote past its end");
                assert!(dst.0[..offset].iter().all(|&b| b == 0xEE), "store of {len} at {offset} wrote before its start");
            }
        }
    }

    #[test]
    fn typed_copies_round_trip_values_of_awkward_sizes() {
        #[derive(Clone, Copy, PartialEq, Debug)]
        #[repr(C)]
        struct Odd {
            a: u64,
            b: u32,
            c: u8,
        }
        #[repr(C, align(8))]
        struct Slot([u8; 32]);
        let mut slot = Slot([0; 32]);
        let v = Odd { a: 0x0102_0304_0506_0708, b: 0x0A0B_0C0D, c: 0x7F };
        // SAFETY: the slot is 8-byte aligned and holds an `Odd` rounded up to 8.
        unsafe { store(slot.0.as_mut_ptr() as *mut Odd, &v) };
        // SAFETY: as above, and the copy is of a value just stored whole.
        let back = unsafe { load(slot.0.as_ptr() as *const Odd).assume_init() };
        assert_eq!(back, v);
        let small: u32 = 0xDEAD_BEEF;
        // SAFETY: as above.
        unsafe { store(slot.0.as_mut_ptr() as *mut u32, &small) };
        // SAFETY: as above.
        assert_eq!(unsafe { load(slot.0.as_ptr() as *const u32).assume_init() }, small);
        let three = [0x1111_1111_1111_1111u64, 0x2222_2222_2222_2222, 0x3333_3333_3333_3333];
        // SAFETY: as above; the slot holds 32 bytes.
        unsafe { store(slot.0.as_mut_ptr() as *mut [u64; 3], &three) };
        // SAFETY: as above.
        assert_eq!(unsafe { load(slot.0.as_ptr() as *const [u64; 3]).assume_init() }, three);
        let four = [0x4444_4444_4444_4444u64, 0x5555_5555_5555_5555, 0x6666_6666_6666_6666, 0x7777_7777_7777_7777];
        // SAFETY: as above.
        unsafe { store(slot.0.as_mut_ptr() as *mut [u64; 4], &four) };
        // SAFETY: as above.
        assert_eq!(unsafe { load(slot.0.as_ptr() as *const [u64; 4]).assume_init() }, four);
        // A source only 4-byte aligned, still copied a word at a time.
        let six: [u32; 6] = [1, 2, 3, 4, 5, 6];
        // SAFETY: as above.
        unsafe { store(slot.0.as_mut_ptr() as *mut [u32; 6], &six) };
        // SAFETY: as above.
        assert_eq!(unsafe { load(slot.0.as_ptr() as *const [u32; 6]).assume_init() }, six);
    }

    #[test]
    fn typed_copies_of_five_to_eight_words_round_trip() {
        #[repr(C, align(8))]
        struct Slot([u8; 64]);
        fn round_trip<T: Copy + PartialEq + std::fmt::Debug>(slot: &mut Slot, value: T) {
            // SAFETY: the slot is 8-byte aligned and holds 64 bytes, at least
            // a `T` rounded up to 8.
            unsafe { store(slot.0.as_mut_ptr() as *mut T, &value) };
            // SAFETY: as above, and the copy is of a value just stored whole.
            assert_eq!(unsafe { load(slot.0.as_ptr() as *const T).assume_init() }, value);
        }
        let mut slot = Slot([0; 64]);
        round_trip(&mut slot, [5u64; 5]);
        round_trip(&mut slot, core::array::from_fn::<u64, 6, _>(|i| i as u64 * 0x0101_0101));
        round_trip(&mut slot, core::array::from_fn::<u64, 7, _>(|i| !(i as u64)));
        round_trip(&mut slot, core::array::from_fn::<u64, 8, _>(|i| i as u64 + 1));
        round_trip(&mut slot, core::array::from_fn::<u8, 52, _>(|i| i as u8));
        round_trip(&mut slot, core::array::from_fn::<u32, 13, _>(|i| i as u32 * 7));
    }

    /// Every size from 1 to 64 bytes, loaded from a 16-byte aligned address,
    /// which takes 16-byte lanes where the processor loads 16 bytes whole, and
    /// from one 8 bytes past it, which takes words; then stored back.
    #[test]
    fn typed_copies_of_every_size_carry_the_bytes_at_either_alignment() {
        fn check<const N: usize>() {
            let mut src = Region([0; 256]);
            for (i, b) in src.0.iter_mut().enumerate() {
                *b = (i as u8).wrapping_mul(29).wrapping_add(N as u8);
            }
            for offset in [16usize, 24] {
                // SAFETY: an 8-byte aligned source inside the region with N
                // rounded up to 8 bytes after it.
                let back = unsafe { load(src.0.as_ptr().add(offset) as *const [u8; N]).assume_init() };
                assert_eq!(&back[..], &src.0[offset..offset + N], "load of {N} at {offset}");
                let mut dst = Region([0xEE; 256]);
                // SAFETY: an 8-byte aligned destination of N bytes inside the
                // region.
                unsafe { store(dst.0.as_mut_ptr().add(offset) as *mut [u8; N], &back) };
                assert_eq!(&dst.0[offset..offset + N], &src.0[offset..offset + N], "store of {N} at {offset}");
                assert!(dst.0[offset + N..].iter().all(|&b| b == 0xEE), "store of {N} at {offset} wrote past its end");
                assert!(dst.0[..offset].iter().all(|&b| b == 0xEE), "store of {N} at {offset} wrote before its start");
            }
        }
        macro_rules! sizes {
            ($($n:literal)*) => { $(check::<$n>();)* };
        }
        sizes!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32
               33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63 64);
    }

    /// A 16-byte aligned region other threads write.
    #[repr(C, align(16))]
    struct Shared(std::cell::UnsafeCell<[u8; 256]>);

    // SAFETY: the test touches the region only through whole-pair atomic stores
    // and the copies under test.
    unsafe impl Sync for Shared {}

    /// The 8-byte word the race test's writers store for number `k`: the same
    /// 32 bits in both halves, so a word read in two pieces shows unequal
    /// halves.
    fn word_for(k: u32) -> u64 {
        u64::from(k) << 32 | u64::from(k)
    }

    /// Whether this processor's plain 16-byte loads are single-copy atomic,
    /// asked of the processor itself: `plain_sixteen` follows the forms
    /// `atomic_pair`'s tests force while they run beside these.
    fn sixteen_on_this_processor() -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            std::arch::is_x86_feature_detected!("avx")
        }
        #[cfg(target_arch = "aarch64")]
        {
            std::arch::is_aarch64_feature_detected!("lse2")
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            false
        }
    }

    /// Writers rewrite a region so every 16-byte element holds one word twice,
    /// with whole 16-byte stores; readers copy it through `load_bytes` and by
    /// each element size directly, and find every 8-byte word whole, and every
    /// 16-byte element whole where the processor loads 16 bytes whole.
    #[test]
    fn copies_racing_writers_find_every_element_whole() {
        const ELEMENTS: usize = 16;
        let region = Arc::new(Shared(std::cell::UnsafeCell::new([0; 256])));
        let base = region.0.get() as usize;
        let stop = Arc::new(AtomicBool::new(false));
        let whole_word = |w: u64| w as u32 == (w >> 32) as u32;
        let sixteen = sixteen_on_this_processor();
        std::thread::scope(|s| {
            for writer in 0..2u32 {
                let stop = Arc::clone(&stop);
                s.spawn(move || {
                    let mut k = writer;
                    while !stop.load(Ordering::Relaxed) {
                        k = k.wrapping_add(2);
                        let word = u128::from(word_for(k));
                        for e in 0..ELEMENTS {
                            // SAFETY: each element is a 16-byte aligned pair inside
                            // the region, written only as a whole pair.
                            let pair = unsafe { crate::atomic_pair::AtomicPair::at((base + e * 16) as *mut u8) };
                            pair.store(word << 64 | word);
                        }
                    }
                });
            }
            for _ in 0..2 {
                let stop = Arc::clone(&stop);
                s.spawn(move || {
                    for _ in 0..100_000 {
                        let mut out = [0u64; ELEMENTS * 2];
                        // SAFETY: the region is 16-byte aligned and 256 bytes; the
                        // destination is private.
                        unsafe { load_bytes(out.as_mut_ptr() as *mut u8, base as *const u8, ELEMENTS * 16) };
                        assert!(out.iter().all(|&w| whole_word(w)), "load_bytes read a torn word: {out:x?}");
                        if sixteen {
                            // SAFETY: as above, on a processor that loads 16 aligned
                            // bytes whole.
                            unsafe { imp::load16(out.as_mut_ptr() as *mut u8, base as *const u8, ELEMENTS * 16) };
                            for e in 0..ELEMENTS {
                                let (lo, hi) = (out[2 * e], out[2 * e + 1]);
                                assert_eq!(lo, hi, "element {e} joined two writes: {lo:#x} {hi:#x}");
                            }
                        }
                        let mut words = [0u64; ELEMENTS * 2];
                        // SAFETY: as above; the 8-byte path, which every host has.
                        unsafe { imp::load8(words.as_mut_ptr() as *mut u8, base as *const u8, ELEMENTS * 16) };
                        assert!(words.iter().all(|&w| whole_word(w)), "the 8-byte path read a torn word: {words:x?}");
                    }
                    stop.store(true, Ordering::Relaxed);
                });
            }
        });
    }
}
