use super::*;

fn vertex(x: f32, y: f32) -> GrVertex {
    GrVertex { x, y, r: 255.0, g: 128.0, b: 0.0, a: 255.0, ooz: 100.0, oow: 1.0, ..Default::default() }
}

fn open() -> Context {
    let mut c = Context::new(Config::default());
    c.win_open(640, 480, ColorFormat::Argb, Origin::UpperLeft);
    c
}

fn draws(c: &mut Context) -> Vec<DrawBatch> {
    c.take_commands()
        .into_iter()
        .filter_map(|cmd| match cmd {
            Command::Draw(d) => Some(d),
            _ => None,
        })
        .collect()
}

#[test]
fn triangles_with_equal_state_share_a_batch() {
    let mut c = open();
    c.draw_triangle(&vertex(0.0, 0.0), &vertex(10.0, 0.0), &vertex(0.0, 10.0));
    c.draw_triangle(&vertex(0.0, 0.0), &vertex(20.0, 0.0), &vertex(0.0, 20.0));
    c.constant_color(0xff00ff00);
    c.draw_triangle(&vertex(0.0, 0.0), &vertex(20.0, 0.0), &vertex(0.0, 20.0));
    let d = draws(&mut c);
    assert_eq!(d.len(), 2);
    assert_eq!(d[0].vertices.len(), 6);
    assert_eq!(d[1].vertices.len(), 3);
}

#[test]
fn culling_follows_the_sdk_sign() {
    let mut c = open();
    // Clockwise on screen (y down): area = (0-10)*(0-10) - (10-0)*(0-0) = 100 > 0.
    let (a, b, cc) = (vertex(0.0, 0.0), vertex(10.0, 0.0), vertex(0.0, 10.0));
    c.cull_mode(CullMode::Positive);
    c.draw_triangle(&a, &b, &cc);
    assert!(draws(&mut c).is_empty());
    c.cull_mode(CullMode::Negative);
    c.draw_triangle(&a, &b, &cc);
    assert_eq!(draws(&mut c).len(), 1);
    assert_eq!(c.tri_stats(), (2, 1));
}

#[test]
fn lower_left_origin_flips_y() {
    let mut c = open();
    c.sst_origin(Origin::LowerLeft);
    c.draw_triangle(&vertex(0.0, 0.0), &vertex(10.0, 0.0), &vertex(0.0, 10.0));
    let d = draws(&mut c);
    assert_eq!(d[0].vertices[0].pos, [0.0, 480.0]);
    c.clip_window(0, 0, 100, 50);
    assert_eq!(c.scissor(), Rect { x: 0, y: 430, width: 100, height: 50 });
}

#[test]
fn texture_download_invalidates_cached_texture() {
    let mut c = open();
    let info = TexInfo { small_lod: Lod::L8, large_lod: Lod::L8, aspect: Aspect::A1x1, format: TextureFormat::Rgb565 };
    c.tex_download_mipmap(Tmu::Tmu0, 0, level_mask::BOTH, info, &[0xff; 128]);
    c.tex_source(Tmu::Tmu0, 0, level_mask::BOTH, info);
    c.gu_tex_combine_function(Tmu::Tmu0, TextureCombineFnc::Decal);
    c.gu_color_combine_function(ColorCombineFnc::DecalTexture);
    c.draw_triangle(&vertex(0.0, 0.0), &vertex(10.0, 0.0), &vertex(0.0, 10.0));
    let commands = c.take_commands();
    assert!(matches!(commands[0], Command::CreateTexture { .. }));
    let Command::Draw(d) = &commands[1] else { panic!("draw expected") };
    assert!(d.textures[0].is_some());
    assert_eq!(d.state.flags[0] & flag::TMU0, flag::TMU0);
    // Overwriting the memory drops the decoded copy.
    c.tex_download_mipmap(Tmu::Tmu0, 64, level_mask::BOTH, info, &[0; 128]);
    assert!(matches!(c.take_commands()[..], [Command::DestroyTexture(_)]));
}

#[test]
fn write_lock_reports_written_pixels() {
    let mut c = open();
    let lock = c.lfb_lock_write(Buffer::Back, LfbWriteMode::W565, Origin::UpperLeft, false);
    assert_eq!(lock.stride, 1280);
    // SAFETY: the lock buffer holds stride * height bytes.
    unsafe {
        let p = lock.data.add(lock.stride as usize * 10 + 5 * 2) as *mut u16;
        p.write_unaligned(0xffff);
    }
    assert!(c.lfb_unlock(true));
    let commands = c.take_commands();
    let Command::LfbWrite { rect, mask, rgba, .. } = &commands[0] else { panic!("lfb write expected") };
    assert_eq!(*rect, Rect { x: 5, y: 10, width: 1, height: 1 });
    assert_eq!(mask[..], [255]);
    assert_eq!(rgba[..], [255, 255, 255, 255]);
}

#[test]
fn state_round_trips() {
    let mut c = open();
    c.gu_color_combine_function(ColorCombineFnc::TextureTimesItrgb);
    c.fog_mode(FogMode(FogMode::WITH_TABLE));
    c.depth_bias_level(-5);
    let mut blob = [0u8; 312];
    c.get_state(&mut blob);
    let saved = c.shader_state(&[None, None]);
    c.gu_color_combine_function(ColorCombineFnc::One);
    c.fog_mode(FogMode(0));
    c.set_state(&blob);
    assert_eq!(c.shader_state(&[None, None]), saved);
}
