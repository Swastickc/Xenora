// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_metal::texture_pipeline::{Rgba8TextureDesc, StagingBudget, plan_staged_upload};
use xenora_metal::{MetalRgba8Texture, Rgba8MipLevel};
use xenora_texture::{Bc1Upload, Bc1UploadDesc, TiledBc1};

use super::context;

#[test]
fn samples_selected_bc1_layer_and_mip() {
    // Each 9×5 layer has two tiles for mip zero and one for mip one.
    // Color words are BC1 endpoint zero except for the transparent block,
    // where the ascending endpoints and selector three give transparent black.
    let red = [0, 248, 0, 0, 0, 0, 0, 0];
    let green = [224, 7, 0, 0, 0, 0, 0, 0];
    let blue = [31, 0, 0, 0, 0, 0, 0, 0];
    let white = [255, 255, 0, 0, 0, 0, 0, 0];
    let clear = [0, 0, 255, 255, 255, 255, 255, 255];
    let yellow = [224, 255, 0, 0, 0, 0, 0, 0];
    let black = [0; 8];
    let blocks = [
        black, black, black, black, black, black, black, black, black, black, black, black, red,
        green, white, clear, blue, black, yellow, black, green, black, black, black,
    ];
    let texture = TiledBc1::new(blocks.as_flattened(), 9, 5, 2, 2).unwrap();
    let context = context();
    let pixels = context.read_tiled_bc1(&texture, 1, 0).unwrap();
    let upper = [
        0xff00_00ff,
        0xff00_00ff,
        0xff00_00ff,
        0xff00_00ff,
        0xff00_ff00,
        0xff00_ff00,
        0xff00_ff00,
        0xff00_ff00,
        0xffff_0000,
    ];
    let lower = [
        0xffff_ffff,
        0xffff_ffff,
        0xffff_ffff,
        0xffff_ffff,
        0,
        0,
        0,
        0,
        0xff00_ffff,
    ];
    let expected: Vec<_> = upper.repeat(4).into_iter().chain(lower).collect();
    assert_eq!(pixels, expected);
    assert_eq!(
        context.read_tiled_bc1(&texture, 1, 1).unwrap(),
        [0xff00_ff00; 8]
    );
    assert_eq!(
        context.read_tiled_bc1(&texture, 0, 1).unwrap(),
        [0xff00_0000; 8]
    );
    assert_eq!(
        context
            .read_tiled_bc1(&texture, 2, 0)
            .unwrap_err()
            .operation,
        "texture subresource"
    );
    assert_eq!(
        context
            .read_tiled_bc1(&texture, 0, 2)
            .unwrap_err()
            .operation,
        "texture subresource"
    );
    println!(
        "device={} checked 61 RGBA8 texels from three subresources",
        context.device_name()
    );
}

#[test]
fn samples_pitched_bc1_upload_across_mips_and_layers() {
    let red = [0, 248, 0, 0, 0, 0, 0, 0];
    let green = [224, 7, 0, 0, 0, 0, 0, 0];
    let blue = [31, 0, 0, 0, 0, 0, 0, 0];
    let white = [255, 255, 0, 0, 0, 0, 0, 0];
    let clear = [0, 0, 255, 255, 255, 255, 255, 255];
    // Seven physical blocks per subresource. A base level consumes the
    // first two blocks of both rows, and mip 1 consumes only block zero.
    let pad = red;
    let blocks = [
        [blue, blue, pad, blue, blue, pad, pad],
        [green, pad, pad, pad, pad, pad, pad],
        [red, green, pad, blue, white, pad, pad],
        [clear, pad, pad, pad, pad, pad, pad],
    ];
    let upload = Bc1Upload::new(
        blocks.as_flattened().as_flattened(),
        Bc1UploadDesc {
            width: 5,
            height: 5,
            mips: 2,
            layers: 2,
            format: 1,
            usage: 1,
            row_pitch: 24,
            slice_pitch: 56,
        },
    )
    .unwrap();
    let gpu = context();
    let upper = [
        0xff00_00ff,
        0xff00_00ff,
        0xff00_00ff,
        0xff00_00ff,
        0xff00_ff00,
    ];
    let bottom = [
        0xffff_0000,
        0xffff_0000,
        0xffff_0000,
        0xffff_0000,
        0xffff_ffff,
    ];
    let expected: Vec<_> = upper.repeat(4).into_iter().chain(bottom).collect();
    assert_eq!(gpu.read_bc1_upload(&upload, 1, 0).unwrap(), expected);
    assert_eq!(gpu.read_bc1_upload(&upload, 1, 1).unwrap(), [0; 4]);
    assert_eq!(
        gpu.read_bc1_upload(&upload, 0, 1).unwrap(),
        [0xff00_ff00; 4]
    );
    assert_eq!(
        gpu.read_bc1_upload(&upload, 2, 0).unwrap_err().operation,
        "texture subresource"
    );
    println!("checked 33 RGBA8 texels from pitched BC1 subresources");
}

/// Texel bytes chosen so neighbouring texels, rows and levels all differ, built
/// without any Xenora code so the readback has an independent expectation.
fn pattern_texel(level: u32, x: usize, y: usize) -> [u8; 4] {
    let level = level as usize;
    [
        (x * 7 + level * 50) as u8,
        (y * 13 + level) as u8,
        ((x ^ y) * 3) as u8,
        (255 - level * 9) as u8,
    ]
}

fn tight_pattern(level: u32, width: usize, height: usize) -> Vec<u8> {
    (0..height)
        .flat_map(|y| (0..width).flat_map(move |x| pattern_texel(level, x, y)))
        .collect()
}

/// Each row padded by `pad` bytes of 0xEE, which must never reach the texture.
fn padded_pattern(level: u32, width: usize, height: usize, pad: usize) -> Vec<u8> {
    let tight = tight_pattern(level, width, height);
    tight
        .chunks(width * 4)
        .flat_map(|row| row.iter().copied().chain(std::iter::repeat_n(0xEE, pad)))
        .collect()
}

fn padded_levels(desc: Rgba8TextureDesc, pad: usize) -> Vec<(usize, usize, Vec<u8>)> {
    (0..desc.mips)
        .map(|level| {
            let (w, h) = desc.mip_extent(level).unwrap();
            (w, h, padded_pattern(level, w, h, pad))
        })
        .collect()
}

fn mip_views(levels: &[(usize, usize, Vec<u8>)], pad: usize) -> Vec<Rgba8MipLevel<'_>> {
    levels
        .iter()
        .enumerate()
        .map(|(level, (w, h, bytes))| Rgba8MipLevel {
            level: level as u32,
            width: *w,
            height: *h,
            row_pitch: w * 4 + pad,
            bytes,
        })
        .collect()
}

#[test]
fn staged_upload_matches_the_source_under_every_budget() {
    let gpu = context();
    // 37×19 has six levels, down to 1×1, and odd sizes at every level.
    let desc = Rgba8TextureDesc::new(37, 19, 6).unwrap();
    let pad = 12;
    let levels = padded_levels(desc, pad);
    let mips = mip_views(&levels, pad);
    let total: usize = levels.iter().map(|(w, h, _)| w * h * 4).sum();
    let widest_row = 37 * 4;
    // One submission, several bands, a single row per band, and a budget that
    // leaves a remainder at every level.
    for budget in [total, 3 * widest_row, widest_row, widest_row + 4] {
        let budget = StagingBudget::new(budget).unwrap();
        let plan = plan_staged_upload(&desc, &mips, budget).unwrap();
        let (texture, report) =
            MetalRgba8Texture::upload_staged(gpu.device(), gpu.queue(), desc, &mips, budget)
                .unwrap();
        assert_eq!(report.submissions, plan.len());
        assert!(report.peak_staging_bytes <= budget.max_bytes());
        assert_eq!(report.bytes_copied, total);
        if budget.max_bytes() == total {
            assert_eq!(report.submissions, 1);
        } else {
            assert!(report.submissions > 1);
        }
        for level in 0..desc.mips {
            let (w, h) = desc.mip_extent(level).unwrap();
            let actual = texture.read_back(gpu.device(), gpu.queue(), level).unwrap();
            assert_eq!(
                actual,
                tight_pattern(level, w, h),
                "budget {} level {level}",
                budget.max_bytes()
            );
        }
    }
}

#[test]
fn staged_and_direct_uploads_produce_the_same_texels() {
    let gpu = context();
    let desc = Rgba8TextureDesc::new(16, 9, 5).unwrap();
    let levels = padded_levels(desc, 0);
    let mips = mip_views(&levels, 0);
    let direct = MetalRgba8Texture::upload(gpu.device(), desc, &mips).unwrap();
    let budget = StagingBudget::new(16 * 4 * 2).unwrap();
    let (staged, _) =
        MetalRgba8Texture::upload_staged(gpu.device(), gpu.queue(), desc, &mips, budget).unwrap();
    for level in 0..desc.mips {
        let (w, h) = desc.mip_extent(level).unwrap();
        let expected = tight_pattern(level, w, h);
        assert_eq!(
            direct.read_back(gpu.device(), gpu.queue(), level).unwrap(),
            expected
        );
        assert_eq!(
            staged.read_back(gpu.device(), gpu.queue(), level).unwrap(),
            expected
        );
    }
}

#[test]
fn staged_upload_handles_a_single_texel() {
    let gpu = context();
    let desc = Rgba8TextureDesc::new(1, 1, 1).unwrap();
    let texel = pattern_texel(0, 0, 0);
    let mips = [Rgba8MipLevel::new_packed(0, 1, 1, &texel)];
    let budget = StagingBudget::new(4).unwrap();
    let (texture, report) =
        MetalRgba8Texture::upload_staged(gpu.device(), gpu.queue(), desc, &mips, budget).unwrap();
    assert_eq!((report.submissions, report.peak_staging_bytes), (1, 4));
    assert_eq!(
        texture.read_back(gpu.device(), gpu.queue(), 0).unwrap(),
        texel
    );
}

#[test]
fn staged_upload_rejects_bad_input_before_touching_the_gpu() {
    let gpu = context();
    let desc = Rgba8TextureDesc::new(8, 4, 2).unwrap();
    let levels = padded_levels(desc, 0);
    let mips = mip_views(&levels, 0);
    let small = StagingBudget::new(28).unwrap();
    let error = MetalRgba8Texture::upload_staged(gpu.device(), gpu.queue(), desc, &mips, small)
        .unwrap_err();
    assert!(!error.is_capability_unavailable());
    assert!(
        error.to_string().contains("smaller than one 32-byte row"),
        "{error}"
    );
    let budget = StagingBudget::new(1024).unwrap();
    let missing =
        MetalRgba8Texture::upload_staged(gpu.device(), gpu.queue(), desc, &mips[..1], budget)
            .unwrap_err();
    assert!(
        missing.to_string().contains("mip level 1 is missing"),
        "{missing}"
    );

    // A Private texture refuses CPU writes instead of failing inside Metal.
    let (texture, _) =
        MetalRgba8Texture::upload_staged(gpu.device(), gpu.queue(), desc, &mips, budget).unwrap();
    let refused = texture.upload_mip(&mips[0]).unwrap_err();
    assert!(refused.to_string().contains("Private-storage"), "{refused}");
}
