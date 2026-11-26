// Opt-in native host layer (Cargo feature `native-system`): what the process can learn from and
// ask of the operating system and the CPU, below what the Rust standard library exposes.
//
// - CPU identity and instruction sets from CPUID and XGETBV (AVX and AVX-512 count only when
//   the OS saves their registers), logical CPUs in this process's affinity mask, physical cores,
//   and data cache sizes from sysfs.
// - The cgroup (v2, then v1) CPU and memory limits of the container the process runs in.
// - Peak resident memory (getrusage), returning freed heap pages to the OS (glibc malloc_trim),
//   transparent huge pages for large pixel buffers (madvise), and pinning a worker thread to one
//   CPU of the affinity mask, physical cores first (sched_setaffinity).
//
// ABI (mirrored by src/system/native.rs): plain C functions and one POD struct; nothing is
// retained, nothing throws, nothing calls back into Rust, and every function is safe to call from
// any thread. Unsupported queries return -1 (or 0 for "not done"); they never fail the caller.
// None of this changes what is rendered: it only informs thread counts, reports, and how memory
// is backed.

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(__x86_64__) || defined(_M_X64) || defined(__i386__) || defined(_M_IX86)
#define AM_X86 1
#if defined(_MSC_VER)
#include <intrin.h>
#else
#include <cpuid.h>
#endif
#endif

#if defined(__linux__)
#include <malloc.h>
#include <sched.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <unistd.h>
#elif defined(__APPLE__)
#include <sys/resource.h>
#endif

namespace {

// Feature bits of `AmCpu::features`; src/system/native.rs names the same bits.
enum : uint64_t {
    kSse42 = 1u << 0,
    kAvx = 1u << 1,
    kAvx2 = 1u << 2,
    kFma = 1u << 3,
    kBmi2 = 1u << 4,
    kAvx512F = 1u << 5,
    kAvx512Bw = 1u << 6,
    kAvx512Vl = 1u << 7,
    kAvx512Vnni = 1u << 8,
    kAvxVnni = 1u << 9,
};

#if AM_X86
void cpuid(uint32_t leaf, uint32_t sub, uint32_t out[4]) {
#if defined(_MSC_VER)
    int regs[4];
    __cpuidex(regs, (int)leaf, (int)sub);
    for (int i = 0; i < 4; ++i) out[i] = (uint32_t)regs[i];
#else
    __cpuid_count(leaf, sub, out[0], out[1], out[2], out[3]);
#endif
}

uint64_t xgetbv0() {
#if defined(_MSC_VER)
    return _xgetbv(0);
#else
    uint32_t lo, hi;
    __asm__ volatile("xgetbv" : "=a"(lo), "=d"(hi) : "c"(0));
    return ((uint64_t)hi << 32) | lo;
#endif
}
#endif

#if defined(__linux__)
// Reads a small text file into `buf`; returns false when it cannot be read.
bool read_text(const char* path, char* buf, size_t size) {
    FILE* file = fopen(path, "re");
    if (!file) return false;
    size_t n = fread(buf, 1, size - 1, file);
    fclose(file);
    buf[n] = '\0';
    return n > 0;
}

// "48K", "2048K", "32M", or plain bytes.
int64_t parse_size(const char* text) {
    char* end = nullptr;
    long long value = strtoll(text, &end, 10);
    if (end == text || value < 0) return -1;
    if (*end == 'K') return value * 1024;
    if (*end == 'M') return value * 1024 * 1024;
    if (*end == 'G') return value * 1024 * 1024 * 1024;
    return value;
}

// The cgroup v2 directory of this process ("0::/path" in /proc/self/cgroup), or empty.
bool cgroup2_dir(char* out, size_t size) {
    char buf[4096];
    if (!read_text("/proc/self/cgroup", buf, sizeof buf)) return false;
    char* save = nullptr;
    for (char* line = strtok_r(buf, "\n", &save); line; line = strtok_r(nullptr, "\n", &save)) {
        if (strncmp(line, "0::", 3) == 0) {
            snprintf(out, size, "/sys/fs/cgroup%s", line + 3);
            return true;
        }
    }
    return false;
}

// Reads `name` from this process's cgroup v2 directory, then from the namespace root (a
// container's own cgroup is usually mounted as the root).
bool cgroup2_file(const char* name, char* buf, size_t size) {
    char dir[2048], path[2304];
    if (cgroup2_dir(dir, sizeof dir)) {
        snprintf(path, sizeof path, "%s/%s", dir, name);
        if (read_text(path, buf, size)) return true;
    }
    snprintf(path, sizeof path, "/sys/fs/cgroup/%s", name);
    return read_text(path, buf, size);
}

// CPUs in this process's affinity mask, physical cores first: the first sibling of every core,
// then the remaining siblings. Returns the count written to `order`.
int ordered_cpus(int* order, int capacity, int* physical) {
    cpu_set_t set;
    CPU_ZERO(&set);
    if (sched_getaffinity(0, sizeof set, &set) != 0) return 0;
    int cpus[CPU_SETSIZE];
    long keys[CPU_SETSIZE];
    int count = 0;
    for (int cpu = 0; cpu < CPU_SETSIZE && count < capacity; ++cpu) {
        if (!CPU_ISSET(cpu, &set)) continue;
        char path[160], buf[32];
        long package = 0, core = cpu;
        snprintf(path, sizeof path, "/sys/devices/system/cpu/cpu%d/topology/physical_package_id", cpu);
        if (read_text(path, buf, sizeof buf)) package = strtol(buf, nullptr, 10);
        snprintf(path, sizeof path, "/sys/devices/system/cpu/cpu%d/topology/core_id", cpu);
        if (read_text(path, buf, sizeof buf)) core = strtol(buf, nullptr, 10);
        cpus[count] = cpu;
        keys[count] = package * 1000003L + core;
        ++count;
    }
    int written = 0, cores = 0;
    bool taken[CPU_SETSIZE] = {};
    for (int i = 0; i < count; ++i) {  // first sibling of each core
        bool seen = false;
        for (int j = 0; j < i; ++j) seen = seen || keys[j] == keys[i];
        if (!seen) {
            order[written++] = cpus[i];
            taken[i] = true;
            ++cores;
        }
    }
    for (int i = 0; i < count; ++i) {
        if (!taken[i]) order[written++] = cpus[i];
    }
    if (physical) *physical = cores;
    return written;
}
#endif

}  // namespace

extern "C" {

struct AmCpu {
    uint32_t logical;   // CPUs this process may run on (affinity mask), or 0
    uint32_t physical;  // distinct cores among them, or 0
    int64_t l1d;        // data cache sizes in bytes, or -1
    int64_t l2;
    int64_t l3;
    uint64_t features;  // the k* bits above
    char vendor[16];
    char brand[64];
};

void am_sys_cpu(AmCpu* info) {
    memset(info, 0, sizeof *info);
    info->l1d = info->l2 = info->l3 = -1;
#if AM_X86
    uint32_t r[4];
    cpuid(0, 0, r);
    uint32_t max_leaf = r[0];
    memcpy(info->vendor + 0, &r[1], 4);
    memcpy(info->vendor + 4, &r[3], 4);
    memcpy(info->vendor + 8, &r[2], 4);
    if (max_leaf >= 1) {
        cpuid(1, 0, r);
        bool osxsave = (r[2] >> 27) & 1;
        uint64_t xcr0 = osxsave ? xgetbv0() : 0;
        bool ymm = (xcr0 & 0x6) == 0x6;     // SSE and AVX state
        bool zmm = (xcr0 & 0xe6) == 0xe6;   // plus opmask and upper ZMM state
        if ((r[2] >> 20) & 1) info->features |= kSse42;
        if (ymm && ((r[2] >> 28) & 1)) info->features |= kAvx;
        if (ymm && ((r[2] >> 12) & 1)) info->features |= kFma;
        if (max_leaf >= 7) {
            cpuid(7, 0, r);
            if (ymm && ((r[1] >> 5) & 1)) info->features |= kAvx2;
            if ((r[1] >> 8) & 1) info->features |= kBmi2;
            if (zmm && ((r[1] >> 16) & 1)) info->features |= kAvx512F;
            if (zmm && ((r[1] >> 30) & 1)) info->features |= kAvx512Bw;
            if (zmm && ((r[1] >> 31) & 1)) info->features |= kAvx512Vl;
            if (zmm && ((r[2] >> 11) & 1)) info->features |= kAvx512Vnni;
            cpuid(7, 1, r);
            if (ymm && ((r[0] >> 4) & 1)) info->features |= kAvxVnni;
        }
    }
    cpuid(0x80000000u, 0, r);
    if (r[0] >= 0x80000004u) {
        for (uint32_t leaf = 0; leaf < 3; ++leaf) {
            cpuid(0x80000002u + leaf, 0, r);
            memcpy(info->brand + leaf * 16, r, 16);
        }
        info->brand[63] = '\0';
    }
#endif
#if defined(__linux__)
    int order[CPU_SETSIZE];
    int physical = 0;
    info->logical = (uint32_t)ordered_cpus(order, CPU_SETSIZE, &physical);
    info->physical = (uint32_t)physical;
    for (int index = 0; index < 8; ++index) {
        char path[128], level[16], type[32], size[32];
        snprintf(path, sizeof path, "/sys/devices/system/cpu/cpu0/cache/index%d/level", index);
        if (!read_text(path, level, sizeof level)) break;
        snprintf(path, sizeof path, "/sys/devices/system/cpu/cpu0/cache/index%d/type", index);
        if (!read_text(path, type, sizeof type)) continue;
        snprintf(path, sizeof path, "/sys/devices/system/cpu/cpu0/cache/index%d/size", index);
        if (!read_text(path, size, sizeof size)) continue;
        int64_t bytes = parse_size(size);
        int lv = atoi(level);
        bool data = strncmp(type, "Data", 4) == 0 || strncmp(type, "Unified", 7) == 0;
        if (lv == 1 && data) info->l1d = bytes;
        if (lv == 2 && data) info->l2 = bytes;
        if (lv == 3 && data) info->l3 = bytes;
    }
#endif
}

// The container's memory limit in bytes, or -1 when there is none or it cannot be read.
int64_t am_sys_memory_limit() {
#if defined(__linux__)
    char buf[64];
    if (cgroup2_file("memory.max", buf, sizeof buf)) {
        if (strncmp(buf, "max", 3) == 0) return -1;
        return parse_size(buf);
    }
    if (read_text("/sys/fs/cgroup/memory/memory.limit_in_bytes", buf, sizeof buf)) {
        int64_t value = parse_size(buf);
        // cgroup v1 reports "no limit" as a page-rounded INT64_MAX.
        return value <= 0 || value >= (int64_t)1 << 60 ? -1 : value;
    }
#endif
    return -1;
}

// The container's CPU quota in CPUs (for example 2.5), or -1 when there is none.
double am_sys_cpu_limit() {
#if defined(__linux__)
    char buf[64];
    if (cgroup2_file("cpu.max", buf, sizeof buf)) {
        if (strncmp(buf, "max", 3) == 0) return -1;
        char* end = nullptr;
        double quota = strtod(buf, &end);
        double period = strtod(end, nullptr);
        return quota > 0 && period > 0 ? quota / period : -1;
    }
    char q[32], p[32];
    if (read_text("/sys/fs/cgroup/cpu/cpu.cfs_quota_us", q, sizeof q) &&
        read_text("/sys/fs/cgroup/cpu/cpu.cfs_period_us", p, sizeof p)) {
        double quota = strtod(q, nullptr), period = strtod(p, nullptr);
        return quota > 0 && period > 0 ? quota / period : -1;
    }
#endif
    return -1;
}

// Peak resident set size of this process in bytes, or -1.
int64_t am_sys_peak_rss() {
#if defined(__linux__) || defined(__APPLE__)
    struct rusage usage;
    if (getrusage(RUSAGE_SELF, &usage) != 0) return -1;
#if defined(__APPLE__)
    return (int64_t)usage.ru_maxrss;  // bytes on macOS
#else
    return (int64_t)usage.ru_maxrss * 1024;  // KiB on Linux
#endif
#else
    return -1;
#endif
}

// Returns freed heap memory to the OS. 1 when memory was released, else 0.
int am_sys_trim_heap() {
#if defined(__linux__) && defined(__GLIBC__)
    return malloc_trim(0);
#else
    return 0;
#endif
}

// Asks for transparent huge pages on the whole 2 MiB pages inside [ptr, ptr + len). Call it
// before the memory is first touched. 1 when advised, else 0. Contents are never changed.
int am_sys_advise_huge(void* ptr, size_t len) {
#if defined(__linux__) && defined(MADV_HUGEPAGE)
    const uintptr_t huge = (uintptr_t)2 << 20;
    uintptr_t start = ((uintptr_t)ptr + huge - 1) & ~(huge - 1);
    uintptr_t end = ((uintptr_t)ptr + len) & ~(huge - 1);
    if (end <= start) return 0;
    return madvise((void*)start, end - start, MADV_HUGEPAGE) == 0;
#else
    (void)ptr;
    (void)len;
    return 0;
#endif
}

// Pins the calling thread to the `slot`-th CPU of the affinity mask, physical cores first
// (wrapping around). 1 when pinned, else 0.
int am_sys_pin_thread(uint32_t slot) {
#if defined(__linux__)
    int order[CPU_SETSIZE];
    int count = ordered_cpus(order, CPU_SETSIZE, nullptr);
    if (count == 0) return 0;
    cpu_set_t set;
    CPU_ZERO(&set);
    CPU_SET(order[slot % (uint32_t)count], &set);
    return sched_setaffinity(0, sizeof set, &set) == 0;
#else
    (void)slot;
    return 0;
#endif
}

}  // extern "C"
