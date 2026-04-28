//! Portable prefetch helper.
//!
//! Issues a hardware prefetch hint for the cache line containing `ptr`,
//! so the line starts fetching from DRAM in parallel with whatever work
//! the caller is about to do before actually loading from that address.
//! The big use case is the transposition table: TT clusters are far
//! larger than L3 in any realistic setting, so probes are near-guaranteed
//! cache misses; issuing a prefetch when the key is computed (and
//! before the eval/movegen work that sits between key-compute and
//! probe) hides the ~100 ns DRAM round-trip behind useful work.
//!
//! The helper is a no-op on architectures where we don't have a stable
//! prefetch path; the caller's code remains correct, just unaccelerated.

/// Prefetch the cache line containing `ptr` for read access into L1,
/// retain hint. Safe to call with any pointer value — prefetch hints
/// never fault on a bad address.
#[inline(always)]
pub fn prefetch_read<T>(ptr: *const T) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
        _mm_prefetch(ptr as *const i8, _MM_HINT_T0);
    }
    #[cfg(target_arch = "x86")]
    unsafe {
        use core::arch::x86::{_mm_prefetch, _MM_HINT_T0};
        _mm_prefetch(ptr as *const i8, _MM_HINT_T0);
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        // PRFM PLDL1KEEP — prefetch for load into L1, keep on retain.
        core::arch::asm!(
            "prfm pldl1keep, [{0}]",
            in(reg) ptr,
            options(nostack, preserves_flags, readonly),
        );
    }
    // Other architectures: no-op. Suppress unused-variable warning.
    #[cfg(not(any(target_arch = "x86_64", target_arch = "x86", target_arch = "aarch64")))]
    {
        let _ = ptr;
    }
}
