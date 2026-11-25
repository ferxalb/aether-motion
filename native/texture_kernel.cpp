// Opt-in native trilinear texture sampling for textured 3D meshes (Cargo feature `cpp-kernel`).
//
// Each entry point samples one texture at `n` points and is bit-identical to Rust's
// `Texture::sample` (src/scene3d/look/texture.rs): the same `f64` operations in the same order,
// no FMA contraction (built with -ffp-contract=off; the AVX2 target does not enable FMA), exact
// `floor`, saturating float-to-int casts, and wrapping `+ 1` on texel coordinates.
//
// ABI (see the `extern "C"` block and src/scene3d/look/kernel.rs, which mirrors it):
// - `AmTexture` and its `AmLevel` array are borrowed for the call; nothing is retained.
// - `u`, `v`, `lod` each hold `n` readable doubles; `out` holds `4 * n` writable doubles,
//   channel-major: R at `out[i]`, G at `out[n + i]`, B at `out[2n + i]`, alpha at `out[3n + i]`.
// - `alpha == 0` lets the SIMD paths skip alpha: its plane is then unspecified.
// - `n` must be a multiple of 8 (callers pad with copies of a real sample). No alignment is
//   required. The kernels never allocate, throw, or call back into Rust.
// - `levels[k].rgba` points to `width * height` RGBA texels, row-major, top row first.
//
// The SIMD paths take groups of 4 (AVX2) or 8 (AVX-512F) samples that read the same mip levels
// with power-of-two repeat (or clamp) wrapping and texel coordinates within +-2^30; any other
// group falls back to the scalar code for those samples, so every input gives the scalar result.

#include <math.h>
#include <stddef.h>
#include <stdint.h>

#if defined(__x86_64__) || defined(_M_X64)
#define AM_X86 1
#include <immintrin.h>
#endif

#if defined(__GNUC__) || defined(__clang__)
#define AM_TARGET(features) __attribute__((target(features)))
#else
#define AM_TARGET(features)
#endif

extern "C" {

struct AmLevel {
    const uint8_t* rgba;
    int32_t width;
    int32_t height;
};

struct AmTexture {
    const AmLevel* levels;
    uint32_t level_count;
    // 0: sRGB color (decoded through `srgb`), 1: linear data (`byte / 255`).
    uint32_t encoding;
    // 0: repeat, 1: clamp.
    uint32_t wrap;
    // 0 when the caller ignores alpha: the SIMD paths then leave the alpha plane unwritten.
    uint32_t alpha;
    // 256 linear values of the sRGB bytes.
    const double* srgb;
};

}  // extern "C"

static_assert(sizeof(AmLevel) == 16, "AmLevel layout");
static_assert(sizeof(AmTexture) == 32, "AmTexture layout");

namespace {

// Rust's `f64 as i64`: NaN is 0 and out-of-range values saturate.
inline int64_t saturate_i64(double d) {
    if (d != d) return 0;
    if (d >= 9223372036854775808.0) return INT64_MAX;
    if (d <= -9223372036854775808.0) return INT64_MIN;
    return static_cast<int64_t>(d);
}

inline int64_t wrap_coord(int64_t c, int64_t size, uint32_t wrap) {
    if (wrap == 0) {
        int64_t r = c % size;
        return r < 0 ? r + size : r;
    }
    return c < 0 ? 0 : (c > size - 1 ? size - 1 : c);
}

inline void texel(const AmTexture& t, const AmLevel& l, int64_t x, int64_t y, double out[4]) {
    const int64_t w = l.width, h = l.height;
    x = wrap_coord(x, w, t.wrap);
    y = wrap_coord(y, h, t.wrap);
    const uint8_t* p = l.rgba + 4 * static_cast<size_t>(y * w + x);
    out[3] = static_cast<double>(p[3]) / 255.0;
    if (t.encoding == 0) {
        out[0] = t.srgb[p[0]];
        out[1] = t.srgb[p[1]];
        out[2] = t.srgb[p[2]];
    } else {
        out[0] = static_cast<double>(p[0]) / 255.0;
        out[1] = static_cast<double>(p[1]) / 255.0;
        out[2] = static_cast<double>(p[2]) / 255.0;
    }
}

inline void bilinear(const AmTexture& t, size_t level, double u, double v, double out[4]) {
    const AmLevel& l = t.levels[level];
    const double x = u * static_cast<double>(l.width) - 0.5;
    const double y = v * static_cast<double>(l.height) - 0.5;
    const double x0 = floor(x), y0 = floor(y);
    const double fx = x - x0, fy = y - y0;
    const int64_t ix = saturate_i64(x0), iy = saturate_i64(y0);
    const int64_t ix1 = static_cast<int64_t>(static_cast<uint64_t>(ix) + 1u);
    const int64_t iy1 = static_cast<int64_t>(static_cast<uint64_t>(iy) + 1u);
    double p00[4], p10[4], p01[4], p11[4];
    texel(t, l, ix, iy, p00);
    texel(t, l, ix1, iy, p10);
    texel(t, l, ix, iy1, p01);
    texel(t, l, ix1, iy1, p11);
    for (int c = 0; c < 4; ++c) {
        const double top = p00[c] + (p10[c] - p00[c]) * fx;
        const double bottom = p01[c] + (p11[c] - p01[c]) * fx;
        out[c] = top + (bottom - top) * fy;
    }
}

inline void sample_one(const AmTexture& t, double u, double v, double lod, double out[4]) {
    const double top = static_cast<double>(t.level_count - 1);
    if (isfinite(lod)) {
        if (lod < 0.0) lod = 0.0;
        if (lod > top) lod = top;
    } else {
        lod = 0.0;
    }
    const double low = floor(lod);
    const double fraction = lod - low;
    double a[4];
    bilinear(t, static_cast<size_t>(low), u, v, a);
    if (fraction <= 0.0 || low >= top) {
        for (int c = 0; c < 4; ++c) out[c] = a[c];
        return;
    }
    double b[4];
    bilinear(t, static_cast<size_t>(low) + 1, u, v, b);
    for (int c = 0; c < 4; ++c) out[c] = a[c] + (b[c] - a[c]) * fraction;
}

inline void scalar_range(const AmTexture& t, const double* u, const double* v, const double* lod,
                         size_t begin, size_t end, size_t n, double* out) {
    for (size_t i = begin; i < end; ++i) {
        double o[4];
        sample_one(t, u[i], v[i], lod[i], o);
        out[i] = o[0];
        out[n + i] = o[1];
        out[2 * n + i] = o[2];
        out[3 * n + i] = o[3];
    }
}

inline bool power_of_two(int32_t v) { return v > 0 && (v & (v - 1)) == 0; }

// A level the SIMD paths can read: clamp wrapping, or repeat on power-of-two sizes (where
// `rem_euclid` is a mask).
inline bool simd_level(const AmTexture& t, const AmLevel& l) {
    return t.wrap != 0 || (power_of_two(l.width) && power_of_two(l.height));
}

// `byte / 255.0` for every byte, computed at compile time with the same IEEE division, so a load
// from it equals the division the scalar code performs.
struct UnormTable {
    double v[256];
    constexpr UnormTable() : v() {
        for (int i = 0; i < 256; ++i) v[i] = static_cast<double>(i) / 255.0;
    }
};
constexpr UnormTable kUnorm{};

// Texel coordinates the SIMD paths convert to `i32` exactly, with room for `+ 1`.
constexpr double kCoordLimit = 1073741824.0;  // 2^30

#if AM_X86

// ---------------------------------------------------------------------------------------------
// AVX2: 4 samples per group. Texels are fetched with 32-bit gathers, sRGB bytes are decoded
// (and alpha, and linear bytes) with 64-bit gathers from 2 KiB tables instead of divisions, and floors are `vroundpd`.

AM_TARGET("avx2") inline __m256d floor4(__m256d x) {
    return _mm256_round_pd(x, _MM_FROUND_TO_NEG_INF | _MM_FROUND_NO_EXC);
}

AM_TARGET("avx2") inline void decode4(const AmTexture& t, __m128i p, __m256d c[4]) {
    const __m128i ff = _mm_set1_epi32(0xff);
    const __m128i r = _mm_and_si128(p, ff);
    const __m128i g = _mm_and_si128(_mm_srli_epi32(p, 8), ff);
    const __m128i b = _mm_and_si128(_mm_srli_epi32(p, 16), ff);
    const double* table = t.encoding == 0 ? t.srgb : kUnorm.v;
    c[0] = _mm256_i32gather_pd(table, r, 8);
    c[1] = _mm256_i32gather_pd(table, g, 8);
    c[2] = _mm256_i32gather_pd(table, b, 8);
    c[3] = t.alpha ? _mm256_i32gather_pd(kUnorm.v, _mm_srli_epi32(p, 24), 8) : _mm256_setzero_pd();
}

AM_TARGET("avx2") inline bool bilinear4(const AmTexture& t, const AmLevel& l, __m256d u, __m256d v,
                                        __m256d out[4]) {
    const __m256d half = _mm256_set1_pd(0.5);
    const __m256d x = _mm256_sub_pd(_mm256_mul_pd(u, _mm256_set1_pd(static_cast<double>(l.width))), half);
    const __m256d y = _mm256_sub_pd(_mm256_mul_pd(v, _mm256_set1_pd(static_cast<double>(l.height))), half);
    const __m256d x0 = floor4(x), y0 = floor4(y);
    const __m256d hi = _mm256_set1_pd(kCoordLimit), lo = _mm256_set1_pd(-kCoordLimit);
    const __m256d in_range = _mm256_and_pd(
        _mm256_and_pd(_mm256_cmp_pd(x0, lo, _CMP_GE_OQ), _mm256_cmp_pd(x0, hi, _CMP_LE_OQ)),
        _mm256_and_pd(_mm256_cmp_pd(y0, lo, _CMP_GE_OQ), _mm256_cmp_pd(y0, hi, _CMP_LE_OQ)));
    if (_mm256_movemask_pd(in_range) != 0xf) return false;
    const __m256d fx = _mm256_sub_pd(x, x0), fy = _mm256_sub_pd(y, y0);
    __m128i ix = _mm256_cvttpd_epi32(x0), iy = _mm256_cvttpd_epi32(y0);
    const __m128i one = _mm_set1_epi32(1);
    __m128i ix1 = _mm_add_epi32(ix, one), iy1 = _mm_add_epi32(iy, one);
    const __m128i wm = _mm_set1_epi32(l.width - 1), hm = _mm_set1_epi32(l.height - 1);
    if (t.wrap == 0) {
        ix = _mm_and_si128(ix, wm);
        ix1 = _mm_and_si128(ix1, wm);
        iy = _mm_and_si128(iy, hm);
        iy1 = _mm_and_si128(iy1, hm);
    } else {
        const __m128i zero = _mm_setzero_si128();
        ix = _mm_min_epi32(_mm_max_epi32(ix, zero), wm);
        ix1 = _mm_min_epi32(_mm_max_epi32(ix1, zero), wm);
        iy = _mm_min_epi32(_mm_max_epi32(iy, zero), hm);
        iy1 = _mm_min_epi32(_mm_max_epi32(iy1, zero), hm);
    }
    const __m128i w = _mm_set1_epi32(l.width);
    const __m128i row0 = _mm_mullo_epi32(iy, w), row1 = _mm_mullo_epi32(iy1, w);
    const int* base = reinterpret_cast<const int*>(l.rgba);
    __m256d c00[4], c10[4], c01[4], c11[4];
    decode4(t, _mm_i32gather_epi32(base, _mm_add_epi32(row0, ix), 4), c00);
    decode4(t, _mm_i32gather_epi32(base, _mm_add_epi32(row0, ix1), 4), c10);
    decode4(t, _mm_i32gather_epi32(base, _mm_add_epi32(row1, ix), 4), c01);
    decode4(t, _mm_i32gather_epi32(base, _mm_add_epi32(row1, ix1), 4), c11);
    const int channels = t.alpha ? 4 : 3;
    for (int c = 0; c < channels; ++c) {
        const __m256d top = _mm256_add_pd(c00[c], _mm256_mul_pd(_mm256_sub_pd(c10[c], c00[c]), fx));
        const __m256d bottom = _mm256_add_pd(c01[c], _mm256_mul_pd(_mm256_sub_pd(c11[c], c01[c]), fx));
        out[c] = _mm256_add_pd(top, _mm256_mul_pd(_mm256_sub_pd(bottom, top), fy));
    }
    return true;
}

AM_TARGET("avx2") inline bool group4(const AmTexture& t, const double* u, const double* v,
                                     const double* lod, size_t n, double* out) {
    const __m256d zero = _mm256_setzero_pd();
    const __m256d top = _mm256_set1_pd(static_cast<double>(t.level_count - 1));
    __m256d l = _mm256_loadu_pd(lod);
    const __m256d finite = _mm256_cmp_pd(_mm256_sub_pd(l, l), zero, _CMP_EQ_OQ);
    l = _mm256_blendv_pd(l, zero, _mm256_cmp_pd(l, zero, _CMP_LT_OQ));
    l = _mm256_blendv_pd(l, top, _mm256_cmp_pd(l, top, _CMP_GT_OQ));
    l = _mm256_and_pd(l, finite);
    const __m256d low = floor4(l);
    const __m256d fraction = _mm256_sub_pd(l, low);
    double lows[4];
    _mm256_storeu_pd(lows, low);
    if (_mm256_movemask_pd(_mm256_cmp_pd(low, _mm256_set1_pd(lows[0]), _CMP_EQ_OQ)) != 0xf) return false;
    const int blend = _mm256_movemask_pd(_mm256_and_pd(_mm256_cmp_pd(fraction, zero, _CMP_GT_OQ),
                                                       _mm256_cmp_pd(low, top, _CMP_LT_OQ)));
    if (blend != 0 && blend != 0xf) return false;
    const size_t level = static_cast<size_t>(lows[0]);
    if (!simd_level(t, t.levels[level]) || (blend && !simd_level(t, t.levels[level + 1]))) return false;
    const int channels = t.alpha ? 4 : 3;
    const __m256d uu = _mm256_loadu_pd(u), vv = _mm256_loadu_pd(v);
    __m256d a[4];
    if (!bilinear4(t, t.levels[level], uu, vv, a)) return false;
    if (blend) {
        __m256d b[4];
        if (!bilinear4(t, t.levels[level + 1], uu, vv, b)) return false;
        for (int c = 0; c < channels; ++c) a[c] = _mm256_add_pd(a[c], _mm256_mul_pd(_mm256_sub_pd(b[c], a[c]), fraction));
    }
    for (int c = 0; c < channels; ++c) _mm256_storeu_pd(out + c * n, a[c]);
    return true;
}

// ---------------------------------------------------------------------------------------------
// AVX-512F: 8 samples per group. Same algorithm; doubles in zmm, `i32` coordinates in ymm.

AM_TARGET("avx512f,avx2") inline __m512d floor8(__m512d x) {
    return _mm512_roundscale_pd(x, _MM_FROUND_TO_NEG_INF | _MM_FROUND_NO_EXC);
}

AM_TARGET("avx512f,avx2") inline void decode8(const AmTexture& t, __m256i p, __m512d c[4]) {
    const __m256i ff = _mm256_set1_epi32(0xff);
    const __m256i r = _mm256_and_si256(p, ff);
    const __m256i g = _mm256_and_si256(_mm256_srli_epi32(p, 8), ff);
    const __m256i b = _mm256_and_si256(_mm256_srli_epi32(p, 16), ff);
    const double* table = t.encoding == 0 ? t.srgb : kUnorm.v;
    c[0] = _mm512_i32gather_pd(r, table, 8);
    c[1] = _mm512_i32gather_pd(g, table, 8);
    c[2] = _mm512_i32gather_pd(b, table, 8);
    c[3] = t.alpha ? _mm512_i32gather_pd(_mm256_srli_epi32(p, 24), kUnorm.v, 8) : _mm512_setzero_pd();
}

AM_TARGET("avx512f,avx2") inline bool bilinear8(const AmTexture& t, const AmLevel& l, __m512d u,
                                                __m512d v, __m512d out[4]) {
    const __m512d half = _mm512_set1_pd(0.5);
    const __m512d x = _mm512_sub_pd(_mm512_mul_pd(u, _mm512_set1_pd(static_cast<double>(l.width))), half);
    const __m512d y = _mm512_sub_pd(_mm512_mul_pd(v, _mm512_set1_pd(static_cast<double>(l.height))), half);
    const __m512d x0 = floor8(x), y0 = floor8(y);
    const __m512d hi = _mm512_set1_pd(kCoordLimit), lo = _mm512_set1_pd(-kCoordLimit);
    const __mmask8 in_range = _mm512_cmp_pd_mask(x0, lo, _CMP_GE_OQ) & _mm512_cmp_pd_mask(x0, hi, _CMP_LE_OQ) &
                              _mm512_cmp_pd_mask(y0, lo, _CMP_GE_OQ) & _mm512_cmp_pd_mask(y0, hi, _CMP_LE_OQ);
    if (in_range != 0xff) return false;
    const __m512d fx = _mm512_sub_pd(x, x0), fy = _mm512_sub_pd(y, y0);
    __m256i ix = _mm512_cvttpd_epi32(x0), iy = _mm512_cvttpd_epi32(y0);
    const __m256i one = _mm256_set1_epi32(1);
    __m256i ix1 = _mm256_add_epi32(ix, one), iy1 = _mm256_add_epi32(iy, one);
    const __m256i wm = _mm256_set1_epi32(l.width - 1), hm = _mm256_set1_epi32(l.height - 1);
    if (t.wrap == 0) {
        ix = _mm256_and_si256(ix, wm);
        ix1 = _mm256_and_si256(ix1, wm);
        iy = _mm256_and_si256(iy, hm);
        iy1 = _mm256_and_si256(iy1, hm);
    } else {
        const __m256i zero = _mm256_setzero_si256();
        ix = _mm256_min_epi32(_mm256_max_epi32(ix, zero), wm);
        ix1 = _mm256_min_epi32(_mm256_max_epi32(ix1, zero), wm);
        iy = _mm256_min_epi32(_mm256_max_epi32(iy, zero), hm);
        iy1 = _mm256_min_epi32(_mm256_max_epi32(iy1, zero), hm);
    }
    const __m256i w = _mm256_set1_epi32(l.width);
    const __m256i row0 = _mm256_mullo_epi32(iy, w), row1 = _mm256_mullo_epi32(iy1, w);
    const int* base = reinterpret_cast<const int*>(l.rgba);
    __m512d c00[4], c10[4], c01[4], c11[4];
    decode8(t, _mm256_i32gather_epi32(base, _mm256_add_epi32(row0, ix), 4), c00);
    decode8(t, _mm256_i32gather_epi32(base, _mm256_add_epi32(row0, ix1), 4), c10);
    decode8(t, _mm256_i32gather_epi32(base, _mm256_add_epi32(row1, ix), 4), c01);
    decode8(t, _mm256_i32gather_epi32(base, _mm256_add_epi32(row1, ix1), 4), c11);
    const int channels = t.alpha ? 4 : 3;
    for (int c = 0; c < channels; ++c) {
        const __m512d top = _mm512_add_pd(c00[c], _mm512_mul_pd(_mm512_sub_pd(c10[c], c00[c]), fx));
        const __m512d bottom = _mm512_add_pd(c01[c], _mm512_mul_pd(_mm512_sub_pd(c11[c], c01[c]), fx));
        out[c] = _mm512_add_pd(top, _mm512_mul_pd(_mm512_sub_pd(bottom, top), fy));
    }
    return true;
}

AM_TARGET("avx512f,avx2") inline bool group8(const AmTexture& t, const double* u, const double* v,
                                             const double* lod, size_t n, double* out) {
    const __m512d zero = _mm512_setzero_pd();
    const __m512d top = _mm512_set1_pd(static_cast<double>(t.level_count - 1));
    __m512d l = _mm512_loadu_pd(lod);
    const __mmask8 finite = _mm512_cmp_pd_mask(_mm512_sub_pd(l, l), zero, _CMP_EQ_OQ);
    l = _mm512_mask_blend_pd(_mm512_cmp_pd_mask(l, zero, _CMP_LT_OQ), l, zero);
    l = _mm512_mask_blend_pd(_mm512_cmp_pd_mask(l, top, _CMP_GT_OQ), l, top);
    l = _mm512_maskz_mov_pd(finite, l);
    const __m512d low = floor8(l);
    const __m512d fraction = _mm512_sub_pd(l, low);
    double lows[8];
    _mm512_storeu_pd(lows, low);
    if (_mm512_cmp_pd_mask(low, _mm512_set1_pd(lows[0]), _CMP_EQ_OQ) != 0xff) return false;
    const __mmask8 blend = _mm512_cmp_pd_mask(fraction, zero, _CMP_GT_OQ) & _mm512_cmp_pd_mask(low, top, _CMP_LT_OQ);
    if (blend != 0 && blend != 0xff) return false;
    const size_t level = static_cast<size_t>(lows[0]);
    if (!simd_level(t, t.levels[level]) || (blend && !simd_level(t, t.levels[level + 1]))) return false;
    const int channels = t.alpha ? 4 : 3;
    const __m512d uu = _mm512_loadu_pd(u), vv = _mm512_loadu_pd(v);
    __m512d a[4];
    if (!bilinear8(t, t.levels[level], uu, vv, a)) return false;
    if (blend) {
        __m512d b[4];
        if (!bilinear8(t, t.levels[level + 1], uu, vv, b)) return false;
        for (int c = 0; c < channels; ++c) a[c] = _mm512_add_pd(a[c], _mm512_mul_pd(_mm512_sub_pd(b[c], a[c]), fraction));
    }
    for (int c = 0; c < channels; ++c) _mm512_storeu_pd(out + c * n, a[c]);
    return true;
}

#endif  // AM_X86

}  // namespace

extern "C" {

void aether_tex_sample_scalar(const AmTexture* t, const double* u, const double* v, const double* lod,
                              size_t n, double* out) {
    scalar_range(*t, u, v, lod, 0, n, n, out);
}

#if AM_X86

AM_TARGET("avx2")
void aether_tex_sample_avx2(const AmTexture* t, const double* u, const double* v, const double* lod,
                            size_t n, double* out) {
    for (size_t i = 0; i < n; i += 4) {
        if (!group4(*t, u + i, v + i, lod + i, n, out + i)) scalar_range(*t, u, v, lod, i, i + 4, n, out);
    }
}

AM_TARGET("avx512f,avx2")
void aether_tex_sample_avx512(const AmTexture* t, const double* u, const double* v, const double* lod,
                              size_t n, double* out) {
    for (size_t i = 0; i < n; i += 8) {
        if (!group8(*t, u + i, v + i, lod + i, n, out + i)) scalar_range(*t, u, v, lod, i, i + 8, n, out);
    }
}

#endif  // AM_X86

}  // extern "C"
