//! FFI to `native/system.cpp` (feature `native-system`). The C++ side mirrors `AmCpu` exactly,
//! retains nothing, never throws, and returns -1 or 0 for what the platform cannot answer.

use super::CpuInfo;
use std::ffi::{c_int, c_void, CStr};

#[repr(C)]
struct AmCpu {
    logical: u32,
    physical: u32,
    l1d: i64,
    l2: i64,
    l3: i64,
    features: u64,
    vendor: [u8; 16],
    brand: [u8; 64],
}

extern "C" {
    fn am_sys_cpu(info: *mut AmCpu);
    fn am_sys_memory_limit() -> i64;
    fn am_sys_cpu_limit() -> f64;
    fn am_sys_peak_rss() -> i64;
    fn am_sys_trim_heap() -> c_int;
    fn am_sys_advise_huge(ptr: *mut c_void, len: usize) -> c_int;
    fn am_sys_pin_thread(slot: u32) -> c_int;
}

fn text(bytes: &[u8]) -> String {
    CStr::from_bytes_until_nul(bytes)
        .map(|s| s.to_string_lossy().trim().to_string())
        .unwrap_or_default()
}

fn positive(value: i64) -> Option<u64> {
    u64::try_from(value).ok().filter(|&v| v > 0)
}

pub(super) fn cpu() -> CpuInfo {
    let mut raw = AmCpu {
        logical: 0,
        physical: 0,
        l1d: -1,
        l2: -1,
        l3: -1,
        features: 0,
        vendor: [0; 16],
        brand: [0; 64],
    };
    // SAFETY: `raw` is a valid, writable `AmCpu`; the call only fills it.
    unsafe { am_sys_cpu(&mut raw) };
    CpuInfo {
        vendor: text(&raw.vendor),
        brand: text(&raw.brand),
        logical: (raw.logical > 0).then_some(raw.logical),
        physical: (raw.physical > 0).then_some(raw.physical),
        l1d: positive(raw.l1d),
        l2: positive(raw.l2),
        l3: positive(raw.l3),
        features: raw.features,
    }
}

pub(super) fn memory_limit() -> Option<u64> {
    // SAFETY: reads cgroup files; no arguments.
    positive(unsafe { am_sys_memory_limit() })
}

pub(super) fn cpu_limit() -> Option<f64> {
    // SAFETY: reads cgroup files; no arguments.
    let cpus = unsafe { am_sys_cpu_limit() };
    (cpus > 0.0).then_some(cpus)
}

pub(super) fn peak_rss() -> Option<u64> {
    // SAFETY: `getrusage` on this process; no arguments.
    positive(unsafe { am_sys_peak_rss() })
}

pub(super) fn trim_heap() -> bool {
    // SAFETY: `malloc_trim` only releases free heap pages.
    unsafe { am_sys_trim_heap() != 0 }
}

pub(super) fn advise_huge(buffer: &mut [u8]) {
    // SAFETY: the range is the live, exclusively borrowed buffer; `madvise(MADV_HUGEPAGE)` only
    // changes how its pages are backed, never their contents.
    unsafe { am_sys_advise_huge(buffer.as_mut_ptr().cast(), buffer.len()) };
}

pub(super) fn pin_thread(slot: usize) -> bool {
    // SAFETY: changes only the calling thread's affinity.
    unsafe { am_sys_pin_thread(u32::try_from(slot).unwrap_or(u32::MAX)) != 0 }
}
