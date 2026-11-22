// CUDA triangle rasterizer for 3D scenes: coverage, depth, and resolve.
//
// It reproduces the CPU rasterizer (`src/scene3d/raster.rs`) sample for sample. The host clips and
// snaps every triangle exactly as the CPU does (`raster::setups`) and uploads the `Setup` records
// below; each thread owns one output pixel, walks the setups in draw order, and for each of the
// pixel's `ss * ss` samples runs the same integer edge test with the top-left rule and the same
// reversed depth test (`z > depth`, so a tie keeps the earlier triangle). The resolve is the CPU's
// integer mean. Build with `--fmad=false` so the depth plane is evaluated without fused
// multiply-adds, as Rust does; see `tools/build_cuda_kernel.py`.

typedef long long i64;

// Must match `raster::Setup` (repr(C), 112 bytes).
struct Setup {
    i64 v[3][2];     // snapped corners, 1/256 sample, relative to the region corner
    i64 bounds[4];   // x0, y0, x1, y1 in samples (end exclusive)
    double depth[3]; // z = z0 + zx * x + zy * y at sample centers
    unsigned color;  // r | g << 8 | b << 16 | 0xff << 24
    unsigned pad;
};

#define ONE 256LL
#define MAX_SAMPLES 16

__device__ __forceinline__ i64 edge_fn(const i64 *a, const i64 *b, i64 px, i64 py) {
    return (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0]);
}

__device__ __forceinline__ int top_left(const i64 *a, const i64 *b) {
    i64 dx = b[0] - a[0], dy = b[1] - a[1];
    return (dy == 0 && dx > 0) || dy < 0;
}

extern "C" __global__ void rasterize(const Setup *setups, int count, unsigned *out, int width,
                                     int height, int ss) {
    int px = blockIdx.x * blockDim.x + threadIdx.x;
    int py = blockIdx.y * blockDim.y + threadIdx.y;
    if (px >= width || py >= height) return;
    int n = ss * ss;
    float depth[MAX_SAMPLES];
    unsigned color[MAX_SAMPLES];
    for (int i = 0; i < n; i++) { depth[i] = 0.0f; color[i] = 0; }
    i64 x0 = (i64)px * ss, y0 = (i64)py * ss;
    for (int t = 0; t < count; t++) {
        const Setup &s = setups[t];
        if (s.bounds[0] >= x0 + ss || s.bounds[2] <= x0 || s.bounds[1] >= y0 + ss ||
            s.bounds[3] <= y0)
            continue;
        const i64 *a = s.v[0], *b = s.v[1], *c = s.v[2];
        const i64 *ep[3] = {a, b, c};
        const i64 *eq[3] = {b, c, a};
        i64 bias[3];
        for (int k = 0; k < 3; k++) bias[k] = top_left(ep[k], eq[k]) ? 0 : 1;
        for (int sy = 0; sy < ss; sy++) {
            i64 y = y0 + sy;
            if (y < s.bounds[1] || y >= s.bounds[3]) continue;
            double zrow = s.depth[0] + s.depth[2] * ((double)y + 0.5);
            for (int sx = 0; sx < ss; sx++) {
                i64 x = x0 + sx;
                if (x < s.bounds[0] || x >= s.bounds[2]) continue;
                i64 cx = x * ONE + ONE / 2, cy = y * ONE + ONE / 2;
                if (edge_fn(a, b, cx, cy) - bias[0] < 0) continue;
                if (edge_fn(b, c, cx, cy) - bias[1] < 0) continue;
                if (edge_fn(c, a, cx, cy) - bias[2] < 0) continue;
                float z = __double2float_rn(zrow + s.depth[1] * ((double)x + 0.5));
                int i = sy * ss + sx;
                if (z > depth[i]) { depth[i] = z; color[i] = s.color; }
            }
        }
    }
    unsigned covered = 0, r = 0, g = 0, b = 0;
    for (int i = 0; i < n; i++) {
        if (depth[i] > 0.0f) {
            covered++;
            r += color[i] & 0xff; g += (color[i] >> 8) & 0xff; b += (color[i] >> 16) & 0xff;
        }
    }
    unsigned pixel = 0;
    if (covered) {
        unsigned h = covered / 2;
        pixel = ((r + h) / covered) | (((g + h) / covered) << 8) | (((b + h) / covered) << 16) |
                (((covered * 255u + (unsigned)n / 2) / (unsigned)n) << 24);
    }
    out[(size_t)py * width + px] = pixel;
}
