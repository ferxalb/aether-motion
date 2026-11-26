//! Standard-library answers for builds without the `native-system` feature: the logical CPU
//! count and the instruction sets `std` detects; limits, peak memory, and the hints are absent.

use super::CpuInfo;

pub(super) fn cpu() -> CpuInfo {
    let mut info = CpuInfo {
        logical: std::thread::available_parallelism()
            .ok()
            .and_then(|n| u32::try_from(n.get()).ok()),
        ..CpuInfo::default()
    };
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::is_x86_feature_detected as has;
        let detected = [
            has!("sse4.2"),
            has!("avx"),
            has!("avx2"),
            has!("fma"),
            has!("bmi2"),
            has!("avx512f"),
            has!("avx512bw"),
            has!("avx512vl"),
            has!("avx512vnni"),
            false,
        ];
        for ((bit, _), on) in super::FEATURES.iter().zip(detected) {
            if on {
                info.features |= bit;
            }
        }
    }
    info
}

pub(super) fn memory_limit() -> Option<u64> {
    None
}

pub(super) fn cpu_limit() -> Option<f64> {
    None
}

pub(super) fn peak_rss() -> Option<u64> {
    None
}

pub(super) fn trim_heap() -> bool {
    false
}

pub(super) fn advise_huge(_buffer: &mut [u8]) {}

pub(super) fn pin_thread(_slot: usize) -> bool {
    false
}
