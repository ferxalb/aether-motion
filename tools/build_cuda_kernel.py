#!/usr/bin/env python3
"""Compile `src/scene3d/cuda/raster.cu` with NVRTC into the artifacts the CUDA backend loads.

Writes, into the output directory: `raster_sm86.cubin` (native code for compute capability 8.6,
the A10) and `raster_compute75.ptx` (portable PTX the driver JIT-compiles for newer GPUs, such as
Ada, Hopper, and Blackwell), plus `MANIFEST` with the NVRTC version and sha256 sums. Needs only
the `nvidia-cuda-nvrtc-cu12` wheel, not a CUDA toolkit. `--fmad=false` keeps the depth plane free of
fused multiply-adds, as on the CPU.

Usage: build_cuda_kernel.py OUTPUT_DIR
"""

import ctypes
import glob
import hashlib
import os
import sys
from pathlib import Path

import nvidia.cuda_nvrtc as pkg

SOURCE = Path(__file__).resolve().parent.parent / "src/scene3d/cuda/raster.cu"
lib = ctypes.CDLL(glob.glob(os.path.join(list(pkg.__path__)[0], "lib", "libnvrtc.so.*"))[0])


def build(arch: str, cubin: bool) -> bytes:
    prog = ctypes.c_void_p()
    assert lib.nvrtcCreateProgram(ctypes.byref(prog), SOURCE.read_bytes(), b"raster.cu", 0, None, None) == 0
    opts = (ctypes.c_char_p * 3)(f"--gpu-architecture={arch}".encode(), b"--fmad=false", b"--std=c++14")
    if lib.nvrtcCompileProgram(prog, 3, opts):
        n = ctypes.c_size_t()
        lib.nvrtcGetProgramLogSize(prog, ctypes.byref(n))
        log = ctypes.create_string_buffer(n.value)
        lib.nvrtcGetProgramLog(prog, log)
        sys.exit(log.value.decode())
    n = ctypes.c_size_t()
    (lib.nvrtcGetCUBINSize if cubin else lib.nvrtcGetPTXSize)(prog, ctypes.byref(n))
    buf = ctypes.create_string_buffer(n.value)
    (lib.nvrtcGetCUBIN if cubin else lib.nvrtcGetPTX)(prog, buf)
    return buf.raw


def main() -> None:
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    major, minor = ctypes.c_int(), ctypes.c_int()
    lib.nvrtcVersion(ctypes.byref(major), ctypes.byref(minor))
    lines = [f"nvrtc {major.value}.{minor.value}", f"source sha256 {hashlib.sha256(SOURCE.read_bytes()).hexdigest()}"]
    for name, arch, cubin in (("raster_sm86.cubin", "sm_86", True), ("raster_compute75.ptx", "compute_75", False)):
        data = build(arch, cubin)
        (out / name).write_bytes(data)
        lines.append(f"{hashlib.sha256(data).hexdigest()}  {name}")
    (out / "MANIFEST").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
