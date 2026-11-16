//! Runs a slang shader preset through librashader on our wgpu device.
#![cfg(feature = "shaders")]

use librashader::runtime::wgpu::{FilterChain, WgpuOutputView};
use librashader::runtime::{Size, Viewport};
use rustyglide_wgpu::{Gpu, wgpu};

#[test]
fn scanline_preset_darkens_every_other_line() {
    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(e) if std::env::var("RUSTYGLIDE_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU ({e})");
            return;
        }
        Err(e) => panic!("no GPU although RUSTYGLIDE_REQUIRE_GPU=1: {e}"),
    };
    let (device, queue) = (&gpu.device, &gpu.queue);
    let preset = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/shaders/scanlines.slangp");
    let mut chain =
        FilterChain::load_from_path(&preset, librashader::presets::ShaderFeatures::NONE, device, queue, None)
            .expect("preset loads");

    let texture = |size: (u32, u32), usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage,
            view_formats: &[],
        })
    };
    // A white 4x4 game frame, shown at 8x8.
    let input = texture((4, 4), wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
    queue.write_texture(
        input.as_image_copy(),
        &[255u8; 4 * 4 * 4],
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(16), rows_per_image: None },
        input.size(),
    );
    let output = texture((8, 8), wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    let view = output.create_view(&Default::default());
    let size = Size { width: 8, height: 8 };
    let viewport = Viewport {
        x: 0.0,
        y: 0.0,
        mvp: None,
        output: WgpuOutputView::new_from_raw(&view, size, wgpu::TextureFormat::Rgba8Unorm),
        size,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    chain.frame(&input, &viewport, &mut encoder, 1, None).expect("frame renders");
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256 * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: None },
        },
        output.size(),
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("maps"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let data = buffer.slice(..).get_mapped_range().expect("mapped");
    let red = |row: usize| data[row * 256];
    let rows: Vec<u8> = (0..8).map(red).collect();
    // Row 0 is either the top or the bottom line depending on the backend's
    // origin, so check that rows alternate between full and half brightness.
    for pair in rows.chunks(2) {
        let (bright, dark) = (pair[0].max(pair[1]), pair[0].min(pair[1]));
        assert!(bright > 240 && (120..=136).contains(&dark), "rows {rows:?}");
    }
}
