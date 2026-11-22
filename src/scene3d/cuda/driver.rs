//! A minimal CUDA driver API binding, loaded at run time.
//!
//! `libcuda.so.1` is opened with `dlopen` the first time the CUDA backend is used, so the binary
//! has no link-time dependency on CUDA and still starts on machines without an NVIDIA driver.
//! Only the dozen calls the rasterizer needs are resolved. Every call returns its `CUresult`
//! through [`Driver::check`], which names the failing call.

use anyhow::{anyhow, bail, Result};
use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};

pub type CUresult = c_int;
pub type CUdevice = c_int;
pub type CUdeviceptr = u64;
pub type Handle = *mut c_void;

macro_rules! driver {
    ($($name:ident: fn($($arg:ty),*);)*) => {
        /// Resolved driver entry points.
        #[allow(non_snake_case)]
        pub struct Driver {
            $(pub $name: unsafe extern "C" fn($($arg),*) -> CUresult,)*
        }

        impl Driver {
            /// Opens the driver library and resolves every entry point.
            pub fn load() -> Result<Self> {
                let lib = open("libcuda.so.1")?;
                Ok(Self {
                    $($name: unsafe {
                        std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) -> CUresult>(
                            symbol(lib, stringify!($name))?,
                        )
                    },)*
                })
            }
        }
    };
}

driver! {
    cuInit: fn(c_uint);
    cuDriverGetVersion: fn(*mut c_int);
    cuDeviceGetCount: fn(*mut c_int);
    cuDeviceGet: fn(*mut CUdevice, c_int);
    cuDeviceGetName: fn(*mut c_char, c_int, CUdevice);
    cuDeviceGetAttribute: fn(*mut c_int, c_int, CUdevice);
    cuDeviceGetUuid: fn(*mut u8, CUdevice);
    cuDevicePrimaryCtxRetain: fn(*mut Handle, CUdevice);
    cuCtxSetCurrent: fn(Handle);
    cuCtxSynchronize: fn();
    cuModuleLoadData: fn(*mut Handle, *const c_void);
    cuModuleGetFunction: fn(*mut Handle, Handle, *const c_char);
    cuMemAlloc_v2: fn(*mut CUdeviceptr, usize);
    cuMemFree_v2: fn(CUdeviceptr);
    cuMemcpyHtoD_v2: fn(CUdeviceptr, *const c_void, usize);
    cuMemcpyDtoH_v2: fn(*mut c_void, CUdeviceptr, usize);
    cuLaunchKernel: fn(Handle, c_uint, c_uint, c_uint, c_uint, c_uint, c_uint, c_uint, Handle, *mut *mut c_void, *mut *mut c_void);
    cuGetErrorName: fn(CUresult, *mut *const c_char);
}

/// `CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR` / `_MINOR`.
pub const CC_MAJOR: c_int = 75;
pub const CC_MINOR: c_int = 76;

fn open(name: &str) -> Result<Handle> {
    let c = CString::new(name)?;
    // SAFETY: a NUL-terminated name; the handle is kept for the life of the process.
    let handle = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW) };
    if handle.is_null() {
        // SAFETY: dlerror returns a static NUL-terminated string or null.
        let why = unsafe {
            let e = libc::dlerror();
            if e.is_null() {
                String::new()
            } else {
                CStr::from_ptr(e).to_string_lossy().into_owned()
            }
        };
        bail!("could not load {name}: {why}");
    }
    Ok(handle)
}

fn symbol(lib: Handle, name: &str) -> Result<*mut c_void> {
    let c = CString::new(name)?;
    // SAFETY: `lib` came from dlopen and the name is NUL-terminated.
    let ptr = unsafe { libc::dlsym(lib, c.as_ptr()) };
    if ptr.is_null() {
        bail!("libcuda.so.1 has no {name}");
    }
    Ok(ptr)
}

impl Driver {
    /// Turns a `CUresult` into an error naming the failing call.
    pub fn check(&self, call: &str, code: CUresult) -> Result<()> {
        if code == 0 {
            return Ok(());
        }
        let mut name: *const c_char = std::ptr::null();
        // SAFETY: the out pointer is valid; on success the driver returns a static string.
        let known = unsafe { (self.cuGetErrorName)(code, &mut name) } == 0 && !name.is_null();
        let label = if known {
            // SAFETY: checked non-null above; the driver's string is NUL-terminated.
            unsafe { CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned()
        } else {
            "unknown".into()
        };
        Err(anyhow!("CUDA {call} failed: {label} ({code})"))
    }
}
