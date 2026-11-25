//! Build script. With the opt-in `cpp-kernel` feature it compiles `native/texture_kernel.cpp`
//! (batched trilinear texture sampling, see `src/scene3d/look/kernel.rs`) into a static library
//! linked into the executable; without the feature it does nothing.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(feature = "cpp-kernel")]
    cpp_kernel();
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
