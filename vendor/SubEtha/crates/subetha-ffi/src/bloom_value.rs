//! The two in-process Bloom filters through the C ABI, as values the caller
//! holds: a TinyBloom is one `uint64_t`, about eight keys, and a FineBloom
//! four words in a `subetha_fine_bloom`, about thirty-two keys. As with the
//! NaN-boxed values there is no handle and nothing shared: these entry
//! points set and test bits in the caller's own words, none takes a mode,
//! and `subetha_init` is not required. A key is hashed as its bytes, as
//! Python's `TinyBloom` and `FineBloom` hash it, so a word built on one
//! surface answers the same on another. False means the key was never
//! added; true means it probably was.

use subetha_pointers::bloom_pointer::{Bloom64, BloomFine};

use crate::batch::{batch_arguments, run_push_many_lens};
use crate::error::{fail, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_OK};
use crate::ring::bytes;
use crate::runtime::entry;

/// Keys a TinyBloom holds before its rate of wrong yeses climbs past a few
/// percent.
pub const SUBETHA_TINY_BLOOM_SUGGESTED_CAPACITY: usize = 8;
/// Keys a FineBloom holds before its rate of wrong yeses climbs past a few
/// percent.
pub const SUBETHA_FINE_BLOOM_SUGGESTED_CAPACITY: usize = 32;

const _: () = assert!(SUBETHA_TINY_BLOOM_SUGGESTED_CAPACITY == Bloom64::SUGGESTED_CAPACITY);
const _: () = assert!(SUBETHA_FINE_BLOOM_SUGGESTED_CAPACITY == BloomFine::SUGGESTED_CAPACITY);

/// A FineBloom: 256 bits in four words, bit `i` being bit `i % 64` of
/// `words[i / 64]`. All zero is the empty filter.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_fine_bloom {
    /// The filter's bits.
    pub words: [u64; 4],
}

/// Check that `count` keys can be read: a strided array and its lengths,
/// and somewhere to write a flag per key.
fn many_arguments(keys: *const u8, stride: usize, lens: *const usize, count: usize, out: *const bool) -> Result<(), i32> {
    if count == 0 {
        return Ok(());
    }
    if keys.is_null() || lens.is_null() || out.is_null() {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "keys, lens or out is null with a non-zero count"));
    }
    if stride.checked_mul(count).is_none() {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, format!("a stride of {stride} over {count} keys overruns the address space")));
    }
    Ok(())
}

/// Ask `contains` about each of `count` keys, writing one flag per key.
///
/// # Safety
/// `keys` addresses `count * stride` readable bytes, `lens` `count` readable
/// lengths and `out` `count` writable flags, as `many_arguments` checked
/// for nulls.
unsafe fn contains_each(
    keys: *const u8,
    stride: usize,
    lens: *const usize,
    count: usize,
    out: *mut bool,
    contains: impl Fn(&[u8]) -> bool,
) -> i32 {
    if let Err(code) = many_arguments(keys, stride, lens, count, out) {
        return code;
    }
    for i in 0..count {
        // SAFETY: `i` is below `count`, which the caller guarantees is
        // readable at `lens` and writable at `out`.
        let len = unsafe { *lens.add(i) };
        if len > stride {
            return fail(SUBETHA_E_INVALID_ARGUMENT, format!("key {i} is {len} bytes, past the stride of {stride}"));
        }
        // SAFETY: the caller guarantees the array spans this key.
        let key = unsafe { std::slice::from_raw_parts(keys.add(i * stride), len) };
        unsafe { *out.add(i) = contains(key) };
    }
    SUBETHA_OK
}

/// Add the `len` bytes at `key` to the TinyBloom `*bits`.
///
/// # Safety
/// `bits` is a valid pointer; `key` is null or points to `len` readable
/// bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_tiny_bloom_insert(bits: *mut u64, key: *const u8, len: usize) -> i32 {
    entry(|| {
        if bits.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "bits is null");
        }
        let key = match unsafe { bytes(key, len) } {
            Ok(k) => k,
            Err(code) => return code,
        };
        // SAFETY: checked non-null; the caller guarantees it is valid.
        let word = unsafe { &mut *bits };
        let mut filter = Bloom64(*word);
        filter.insert(key);
        *word = filter.0;
        SUBETHA_OK
    })
}

/// Whether the TinyBloom `bits` probably holds the `len` bytes at `key`,
/// into `out`: false means it was never added.
///
/// # Safety
/// `key` is null or points to `len` readable bytes; `out` is a valid
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_tiny_bloom_contains(bits: u64, key: *const u8, len: usize, out: *mut bool) -> i32 {
    entry(|| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        let key = match unsafe { bytes(key, len) } {
            Ok(k) => k,
            Err(code) => return code,
        };
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out = Bloom64(bits).might_contain(key) };
        SUBETHA_OK
    })
}

/// Add `count` keys to the TinyBloom `*bits`, the first at `keys` and each
/// next one `stride` bytes on, key `i` being `lens[i]` bytes. A length past
/// `stride` stops the batch; `out_done` counts the keys added.
///
/// # Safety
/// `bits` is a valid pointer; `keys` addresses `count * stride` readable
/// bytes, `lens` `count` readable lengths, and `out_done` is a valid
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_tiny_bloom_insert_many(
    bits: *mut u64,
    keys: *const u8,
    stride: usize,
    lens: *const usize,
    count: usize,
    out_done: *mut usize,
) -> i32 {
    entry(|| {
        if bits.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "bits is null");
        }
        // SAFETY: checked non-null; the caller guarantees it is valid.
        let word = unsafe { &mut *bits };
        let mut filter = Bloom64(*word);
        // SAFETY: the caller guarantees the array, the lengths and the count word.
        let code = unsafe {
            run_push_many_lens(keys, stride, lens, count, out_done, |key| {
                filter.insert(key);
                SUBETHA_OK
            })
        };
        *word = filter.0;
        code
    })
}

/// Ask the TinyBloom `bits` about `count` keys laid out as
/// `subetha_tiny_bloom_insert_many` takes them, writing one flag per key
/// into `out`.
///
/// # Safety
/// `keys` addresses `count * stride` readable bytes, `lens` `count` readable
/// lengths and `out` `count` writable flags.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_tiny_bloom_contains_many(
    bits: u64,
    keys: *const u8,
    stride: usize,
    lens: *const usize,
    count: usize,
    out: *mut bool,
) -> i32 {
    entry(|| unsafe { contains_each(keys, stride, lens, count, out, |key| Bloom64(bits).might_contain(key)) })
}

/// How many of the TinyBloom's 64 bits are set, which is how full it is.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_tiny_bloom_set_bits(bits: u64) -> u32 {
    Bloom64(bits).popcount()
}

/// The share of wrong yeses to expect once `keys` keys are in a TinyBloom,
/// between zero and one.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_tiny_bloom_false_positive_rate(keys: usize) -> f64 {
    Bloom64::estimated_fpr(keys)
}

/// Add the `len` bytes at `key` to the FineBloom `*filter`.
///
/// # Safety
/// `filter` is a valid pointer; `key` is null or points to `len` readable
/// bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_fine_bloom_insert(filter: *mut subetha_fine_bloom, key: *const u8, len: usize) -> i32 {
    entry(|| {
        if filter.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "filter is null");
        }
        let key = match unsafe { bytes(key, len) } {
            Ok(k) => k,
            Err(code) => return code,
        };
        // SAFETY: checked non-null; the caller guarantees it is valid.
        let held = unsafe { &mut *filter };
        let mut fine = BloomFine::from_words(held.words);
        fine.insert(key);
        held.words = fine.words();
        SUBETHA_OK
    })
}

/// Whether the FineBloom `*filter` probably holds the `len` bytes at `key`,
/// into `out`.
///
/// # Safety
/// `filter` is a valid pointer; `key` is null or points to `len` readable
/// bytes; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_fine_bloom_contains(
    filter: *const subetha_fine_bloom,
    key: *const u8,
    len: usize,
    out: *mut bool,
) -> i32 {
    entry(|| {
        if filter.is_null() || out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "filter or out is null");
        }
        let key = match unsafe { bytes(key, len) } {
            Ok(k) => k,
            Err(code) => return code,
        };
        // SAFETY: both checked non-null; the caller guarantees they are valid.
        unsafe { *out = BloomFine::from_words((*filter).words).might_contain(key) };
        SUBETHA_OK
    })
}

/// Add `count` keys to the FineBloom `*filter`, laid out as
/// `subetha_tiny_bloom_insert_many` takes them.
///
/// # Safety
/// `filter` is a valid pointer; `keys` addresses `count * stride` readable
/// bytes, `lens` `count` readable lengths, and `out_done` is a valid
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_fine_bloom_insert_many(
    filter: *mut subetha_fine_bloom,
    keys: *const u8,
    stride: usize,
    lens: *const usize,
    count: usize,
    out_done: *mut usize,
) -> i32 {
    entry(|| {
        if filter.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "filter is null");
        }
        if let Err(code) = batch_arguments(keys.is_null(), stride, 0, count, out_done.is_null()) {
            return code;
        }
        // SAFETY: checked non-null; the caller guarantees it is valid.
        let held = unsafe { &mut *filter };
        let mut fine = BloomFine::from_words(held.words);
        // SAFETY: the caller guarantees the array, the lengths and the count word.
        let code = unsafe {
            run_push_many_lens(keys, stride, lens, count, out_done, |key| {
                fine.insert(key);
                SUBETHA_OK
            })
        };
        held.words = fine.words();
        code
    })
}

/// Ask the FineBloom `*filter` about `count` keys laid out as
/// `subetha_tiny_bloom_insert_many` takes them, writing one flag per key
/// into `out`.
///
/// # Safety
/// `filter` is a valid pointer; `keys` addresses `count * stride` readable
/// bytes, `lens` `count` readable lengths and `out` `count` writable flags.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_fine_bloom_contains_many(
    filter: *const subetha_fine_bloom,
    keys: *const u8,
    stride: usize,
    lens: *const usize,
    count: usize,
    out: *mut bool,
) -> i32 {
    entry(|| {
        if filter.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "filter is null");
        }
        // SAFETY: checked non-null; the caller guarantees it is valid.
        let fine = BloomFine::from_words(unsafe { (*filter).words });
        unsafe { contains_each(keys, stride, lens, count, out, |key| fine.might_contain(key)) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key added to either filter is found, a key never added is
    /// usually not, the words match what Rust builds from the same bytes,
    /// and the batch forms agree with the single ones.
    #[test]
    fn a_key_added_is_found_and_the_words_match_rust() {
        let mut tiny = 0u64;
        let mut fine = subetha_fine_bloom::default();
        let mut found = false;
        // SAFETY: valid words, keys and out pointers throughout.
        unsafe {
            assert_eq!(subetha_tiny_bloom_insert(&mut tiny, b"alpha".as_ptr(), 5), SUBETHA_OK);
            assert_eq!(subetha_fine_bloom_insert(&mut fine, b"alpha".as_ptr(), 5), SUBETHA_OK);
            assert_eq!(subetha_tiny_bloom_contains(tiny, b"alpha".as_ptr(), 5, &mut found), SUBETHA_OK);
            assert!(found);
            assert_eq!(subetha_fine_bloom_contains(&fine, b"alpha".as_ptr(), 5, &mut found), SUBETHA_OK);
            assert!(found);
            assert_eq!(subetha_tiny_bloom_insert(std::ptr::null_mut(), b"x".as_ptr(), 1), SUBETHA_E_INVALID_ARGUMENT);
        }
        let mut rust_tiny = Bloom64::ZERO;
        rust_tiny.insert(&b"alpha"[..]);
        assert_eq!(tiny, rust_tiny.0, "a word built in C is the word Rust builds from the same bytes");
        let mut rust_fine = BloomFine::ZERO;
        rust_fine.insert(&b"alpha"[..]);
        assert_eq!(fine.words, rust_fine.words());
        assert_eq!(subetha_tiny_bloom_set_bits(tiny), rust_tiny.popcount());
        assert!(subetha_tiny_bloom_false_positive_rate(16) > subetha_tiny_bloom_false_positive_rate(4));

        const STRIDE: usize = 5;
        let keys: [u8; 3 * STRIDE] = *b"one\0\0two\0\0three";
        let lens = [3usize, 3, 5];
        let mut done = 0usize;
        let mut flags = [false; 3];
        let (mut tiny_many, mut fine_many) = (0u64, subetha_fine_bloom::default());
        // SAFETY: valid arrays and out pointers throughout.
        unsafe {
            assert_eq!(subetha_tiny_bloom_insert_many(&mut tiny_many, keys.as_ptr(), STRIDE, lens.as_ptr(), 3, &mut done), SUBETHA_OK);
            assert_eq!(done, 3);
            assert_eq!(subetha_fine_bloom_insert_many(&mut fine_many, keys.as_ptr(), STRIDE, lens.as_ptr(), 3, &mut done), SUBETHA_OK);
            assert_eq!(done, 3);
            assert_eq!(subetha_tiny_bloom_contains_many(tiny_many, keys.as_ptr(), STRIDE, lens.as_ptr(), 3, flags.as_mut_ptr()), SUBETHA_OK);
            assert_eq!(flags, [true; 3]);
            flags = [false; 3];
            assert_eq!(subetha_fine_bloom_contains_many(&fine_many, keys.as_ptr(), STRIDE, lens.as_ptr(), 3, flags.as_mut_ptr()), SUBETHA_OK);
            assert_eq!(flags, [true; 3]);
            let mut one = 0u64;
            assert_eq!(subetha_tiny_bloom_insert(&mut one, b"one".as_ptr(), 3), SUBETHA_OK);
            assert_eq!(one & !tiny_many, 0, "a batch sets every bit the single insert sets");
            let past = [STRIDE + 1];
            assert_eq!(
                subetha_tiny_bloom_contains_many(tiny_many, keys.as_ptr(), STRIDE, past.as_ptr(), 1, flags.as_mut_ptr()),
                SUBETHA_E_INVALID_ARGUMENT
            );
        }
    }
}
