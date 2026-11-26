//! Host system services: CPU identity and topology, container (cgroup) limits, peak memory, and
//! memory hints, independent of composition, rendering, and encoding.
//!
//! With the opt-in `native-system` feature these come from `native/system.cpp` (CPUID and XGETBV,
//! sysfs, cgroup files, `getrusage`, `malloc_trim`, `madvise`, `sched_setaffinity`) through
//! `native.rs`. Without it, `fallback.rs` answers from the standard library and the hints do
//! nothing. Nothing here can change a pixel or an encoded byte: callers use it for thread counts,
//! reports, and how memory is backed.

#[cfg(not(feature = "native-system"))]
mod fallback;
#[cfg(feature = "native-system")]
mod native;

#[cfg(not(feature = "native-system"))]
use fallback as backend;
#[cfg(feature = "native-system")]
use native as backend;

use std::sync::OnceLock;

/// Instruction sets the CPU has and the OS enables.
pub const FEATURES: [(u64, &str); 10] = [
    (1 << 0, "sse4.2"),
    (1 << 1, "avx"),
    (1 << 2, "avx2"),
    (1 << 3, "fma"),
    (1 << 4, "bmi2"),
    (1 << 5, "avx512f"),
    (1 << 6, "avx512bw"),
    (1 << 7, "avx512vl"),
    (1 << 8, "avx512vnni"),
    (1 << 9, "avxvnni"),
];

/// What the process knows about the CPU it runs on.
#[derive(Clone, Debug, Default)]
pub struct CpuInfo {
    pub vendor: String,
    pub brand: String,
    /// CPUs this process may run on (its affinity mask).
    pub logical: Option<u32>,
    /// Distinct physical cores among them.
    pub physical: Option<u32>,
    /// Data cache sizes in bytes.
    pub l1d: Option<u64>,
    pub l2: Option<u64>,
    pub l3: Option<u64>,
    /// [`FEATURES`] bits.
    pub features: u64,
}

impl CpuInfo {
    /// Names of the instruction sets present, in [`FEATURES`] order.
    pub fn feature_names(&self) -> Vec<&'static str> {
        FEATURES
            .iter()
            .filter(|(bit, _)| self.features & bit != 0)
            .map(|&(_, name)| name)
            .collect()
    }
}

/// Smallest buffer worth backing with huge pages (two 2 MiB pages).
const HUGE_PAGE_MIN: usize = 4 << 20;

/// Whether the native layer is compiled in.
pub const NATIVE: bool = cfg!(feature = "native-system");

/// The CPU, read once per process.
pub fn cpu() -> &'static CpuInfo {
    static CPU: OnceLock<CpuInfo> = OnceLock::new();
    CPU.get_or_init(backend::cpu)
}

/// The container's memory limit in bytes, if any.
pub fn memory_limit() -> Option<u64> {
    backend::memory_limit()
}

/// The container's CPU quota in CPUs (for example 2.5), if any.
pub fn cpu_limit() -> Option<f64> {
    backend::cpu_limit()
}

/// Peak resident memory of this process in bytes.
pub fn peak_rss() -> Option<u64> {
    backend::peak_rss()
}

/// Returns freed heap pages to the OS. Long-lived processes (the MCP server) call it after a
/// render so the memory of finished frames, caches, and encoders does not stay resident.
pub fn trim_heap() -> bool {
    backend::trim_heap()
}

/// A zeroed byte buffer of `len` bytes. With `AETHER_HUGE_PAGES=on`, large ones ask for
/// transparent huge pages where the OS supports them; the contents are the same. Off by default:
/// it made no measurable difference on 1080p and 4K frames (BENCHMARK.md, Native host layer).
pub fn zeroed_buffer(len: usize) -> Vec<u8> {
    let mut buffer = vec![0u8; len];
    if len >= HUGE_PAGE_MIN && huge_pages_enabled() {
        // `vec![0; n]` of this size is a fresh zero mapping whose pages are not touched yet.
        backend::advise_huge(&mut buffer);
    }
    buffer
}

/// `AETHER_HUGE_PAGES=on` enables the huge-page hint, to compare speed and memory.
fn huge_pages_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("AETHER_HUGE_PAGES").is_ok_and(|v| v == "on"))
}

/// With `AETHER_PIN_THREADS=1`, pins the calling worker thread to the `slot`-th CPU of the
/// affinity mask, physical cores first. Off by default: on a shared machine the scheduler places
/// threads better than a fixed mapping. Returns whether the thread was pinned.
pub fn pin_worker(slot: usize) -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    let on = *ON.get_or_init(|| std::env::var("AETHER_PIN_THREADS").is_ok_and(|v| v == "1"));
    on && backend::pin_thread(slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_report_is_consistent() {
        let info = cpu();
        if let (Some(logical), Some(physical)) = (info.logical, info.physical) {
            assert!(physical >= 1 && physical <= logical);
        }
        #[cfg(target_arch = "x86_64")]
        {
            let names = info.feature_names();
            assert_eq!(
                names.contains(&"avx2"),
                std::arch::is_x86_feature_detected!("avx2")
            );
            assert_eq!(
                names.contains(&"avx512f"),
                std::arch::is_x86_feature_detected!("avx512f")
            );
        }
    }

    #[test]
    fn zeroed_buffers_are_zero_at_every_size() {
        for len in [
            0,
            1,
            HUGE_PAGE_MIN - 1,
            HUGE_PAGE_MIN,
            HUGE_PAGE_MIN * 3 + 12_345,
        ] {
            let buffer = zeroed_buffer(len);
            assert_eq!(buffer.len(), len);
            assert!(buffer.iter().all(|&b| b == 0));
        }
        trim_heap();
        assert!(peak_rss().is_none_or(|bytes| bytes > 0));
    }
}
