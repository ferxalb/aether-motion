//! Build script. With the opt-in `cpp-kernel` feature it compiles `native/texture_kernel.cpp`
//! (batched trilinear texture sampling, see `src/scene3d/look/kernel.rs`), and with
//! `native-system` it compiles `native/system.cpp` (the host layer of `src/system/`), each into a
//! static library linked into the executable; without the features it does nothing.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(feature = "cpp-kernel")]
    cpp_kernel();
    #[cfg(feature = "native-system")]
    native_system();
}

#[cfg(feature = "native-system")]
fn native_system() {
    println!("cargo:rerun-if-changed=native/system.cpp");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("native/system.cpp")
        // Plain C ABI on top of libc: no C++ runtime, exceptions, or RTTI.
        .cpp_link_stdlib(None)
        .opt_level(2);
    if build.get_compiler().is_like_msvc() {
        build.flag("/std:c++17").flag("/EHs-c-").flag("/GR-");
    } else {
        build
            .std("c++17")
            .flag("-fno-exceptions")
            .flag("-fno-rtti")
            .flag("-fno-asynchronous-unwind-tables");
    }
    build.compile("aether_native_system");
}

#[cfg(feature = "cpp-kernel")]
fn cpp_kernel() {
    println!("cargo:rerun-if-changed=native/texture_kernel.cpp");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("native/texture_kernel.cpp")
        // The kernel uses no C++ runtime: no allocation, exceptions, RTTI, or static guards.
        .cpp_link_stdlib(None);
    if build.get_compiler().is_like_msvc() {
        // Precise IEEE semantics without contraction (the default of /fp:precise since VS 2022).
        build
            .flag("/std:c++17")
            .flag("/fp:precise")
            .flag("/EHs-c-")
            .flag("/GR-");
    } else {
        // Pixel parity with the Rust path needs IEEE arithmetic without FMA contraction (GCC
        // contracts by default in C++). The ISA baseline is the target's; AVX2 and AVX-512
        // code is enabled per function and chosen at run time.
        build
            .std("c++17")
            .flag("-fno-exceptions")
            .flag("-fno-rtti")
            .flag("-ffp-contract=off")
            .flag("-fno-fast-math");
    }
    build.compile("aether_texture_kernel");
}
