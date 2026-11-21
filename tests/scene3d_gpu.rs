//! The wgpu backend of `data-hf-3d` scenes (`--features gpu`) against the CPU reference, within the
//! documented tolerance (see `docs/3d.md`), and the fallback when no adapter exists. Without an
//! adapter (for example on a machine with no Vulkan, Metal, or DX12 driver) the comparison is
//! skipped with a message, since the CPU fallback is then what runs.
#![cfg(feature = "gpu")]

mod common;

use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use aether_motion::scene3d::backend::{set_request, status, Request};
use common::scratch;

const SCENE: &str = "<div data-composition-id='g' data-width='480' data-height='270' data-duration='2' style='background:#0b0f18'>\
 <div class='clip' data-start='0' data-duration='2' data-track-index='1' data-hf-3d data-hf-3d-antialias='3' style='position:absolute;left:0;top:0;width:480px;height:270px'>\
 <hf-light type='ambient' color='#8899bb' intensity='0.3'></hf-light>\
 <hf-light type='directional' direction='0.5 0.9 -0.6' color='#ffe4c0' intensity='1.1'></hf-light>\
 <hf-camera position='0 -250 800' target='0 0 0' data-hf-keyframes='[{\"at\":0,\"x\":-400},{\"at\":2,\"x\":400,\"easing\":\"smooth\"}]'></hf-camera>\
 <hf-mesh shape='plane' size='1600 1600' position='0 100 0' color='#2a3350'></hf-mesh>\
 <hf-node data-hf-keyframes='[{\"at\":0,\"rotateY\":0},{\"at\":2,\"rotateY\":90}]'>\
 <hf-mesh shape='box' size='140 200 140' position='-200 0 0' color='#d08c5a'></hf-mesh>\
 <hf-mesh shape='sphere' radius='80' detail='2' position='0 20 0' color='#7fb3d5'></hf-mesh>\
 <hf-mesh shape='cylinder' radius='45' height='180' position='200 10 0' color='#9bd18b'></hf-mesh>\
 <hf-mesh shape='cone' radius='60' height='160' position='0 20 -200' color='#c97bb6'></hf-mesh>\
 </hf-node></div></div>";

#[test]
fn gpu_output_matches_the_cpu_within_tolerance() {
    let dir = scratch("scene3d-gpu");
    let path = dir.join("index.html");
    std::fs::write(&path, SCENE).unwrap();
    let composition = parse_file(&path).unwrap();
    let render = |request: Request, time: f64| {
        set_request(request);
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        renderer.render_at(time).unwrap().to_vec()
    };
    let _ = render(Request::Gpu, 0.0);
    let (backend, reason) = status();
    if backend != "gpu" {
        eprintln!("skipping: no GPU adapter ({reason:?}); the CPU fallback was used");
        let cpu = render(Request::Cpu, 0.5);
        assert_eq!(
            cpu,
            render(Request::Gpu, 0.5),
            "the fallback must equal the CPU"
        );
        return;
    }
    for time in [0.0, 0.7, 1.4] {
        let cpu = render(Request::Cpu, time);
        let gpu = render(Request::Gpu, time);
        let pixels = cpu.len() / 4;
        let mut far = 0;
        let mut total = 0_u64;
        let mut peak = 0;
        for (a, b) in cpu.chunks_exact(4).zip(gpu.chunks_exact(4)) {
            let d = (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap();
            total += u64::from(d);
            peak = peak.max(d);
            if d > 2 {
                far += 1;
            }
        }
        let mean = total as f64 / pixels as f64;
        eprintln!("t={time}: mean {mean:.4} levels, {far}/{pixels} pixels > 2 levels, peak {peak}");
        // Documented tolerance: mean error under 0.25 levels, at most 0.2% of pixels more than
        // 2 levels apart (edge samples the hardware covers differently).
        assert!(mean < 0.25, "t={time}: mean difference {mean}");
        assert!(
            far * 500 <= pixels,
            "t={time}: {far} of {pixels} pixels differ by more than 2 levels (peak {peak})"
        );
        // Determinism of the GPU path itself on one device.
        assert_eq!(
            gpu,
            render(Request::Gpu, time),
            "t={time}: GPU output changed between draws"
        );
    }
    set_request(Request::Cpu);
    std::fs::remove_dir_all(dir).unwrap();
}
