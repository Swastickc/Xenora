// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent CPU reference for depth-tested offscreen triangle rendering.
//!
//! A pixel is covered when its centre lies strictly inside a triangle. Inputs
//! whose pixel centres lie exactly on an edge are rejected rather than resolved
//! by a fill rule, so the reference never depends on rasteriser tie-breaking.

use std::fmt;

const MAX_TARGET_PIXELS: usize = 4096;

/// Depth comparison of the fragment depth against the stored depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompareFunction {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

impl CompareFunction {
    pub const ALL: [Self; 8] = [
        Self::Never,
        Self::Less,
        Self::Equal,
        Self::LessEqual,
        Self::Greater,
        Self::NotEqual,
        Self::GreaterEqual,
        Self::Always,
    ];

    #[must_use]
    pub fn passes(self, fragment: f32, stored: f32) -> bool {
        match self {
            Self::Never => false,
            Self::Less => fragment < stored,
            Self::Equal => fragment == stored,
            Self::LessEqual => fragment <= stored,
            Self::Greater => fragment > stored,
            Self::NotEqual => fragment != stored,
            Self::GreaterEqual => fragment >= stored,
            Self::Always => true,
        }
    }
}

/// Colour source of one reference draw.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefFill {
    Flat([u8; 4]),
    /// Nearest texel of the pass texture at the pixel centre.
    Texture,
}

/// One triangle with clip-space positions (`w` must be 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefDraw {
    pub positions: [[f32; 4]; 3],
    pub fill: RefFill,
    pub compare: CompareFunction,
    pub write: bool,
}

/// Decoded RGBA8 texels in row-major order.
#[derive(Clone, Copy, Debug)]
pub struct RefTexture<'a> {
    pub width: usize,
    pub height: usize,
    pub texels: &'a [[u8; 4]],
}

#[derive(Debug, Eq, PartialEq)]
pub enum DepthRefError {
    Extent,
    /// The clear depth is not finite or lies outside 0..=1.
    InvalidClearDepth,
    NonUnitW,
    /// A position is non-finite or its depth lies outside 0..=1: (draw).
    Unclipped(usize),
    /// A pixel centre lies on a triangle edge: (draw, x, y).
    EdgeTie(usize, usize, usize),
    MissingTexture,
    /// Without a depth attachment every draw must be `Always` without writes.
    DepthStateWithoutAttachment(usize),
}

impl fmt::Display for DepthRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for DepthRefError {}

/// Colour (row-major, top row first) and, with an attachment, stored depth.
pub type DepthRefOutput = (Vec<[u8; 4]>, Option<Vec<f32>>);

/// Render `draws` in order over a cleared target with viewport depth range 0..1.
///
/// # Errors
///
/// Returns an error for a target extent outside 1..=4096 pixels, an invalid
/// texture extent, a clear depth outside 0..=1, non-unit `w`, a non-finite
/// position or vertex depth outside 0..=1, a pixel centre on an edge, a
/// textured draw without texture, or a depth state without attachment.
pub fn render_depth(
    [width, height]: [usize; 2],
    clear_color: [u8; 4],
    clear_depth: Option<f32>,
    draws: &[RefDraw],
    texture: Option<RefTexture<'_>>,
) -> Result<DepthRefOutput, DepthRefError> {
    let pixels = width
        .checked_mul(height)
        .filter(|n| (1..=MAX_TARGET_PIXELS).contains(n))
        .ok_or(DepthRefError::Extent)?;
    if let Some(texture) = texture
        && texture.width.checked_mul(texture.height).filter(|&n| n > 0)
            != Some(texture.texels.len())
    {
        return Err(DepthRefError::Extent);
    }
    if let Some(clear) = clear_depth
        && !(0.0..=1.0).contains(&clear)
    {
        return Err(DepthRefError::InvalidClearDepth);
    }
    let mut color = vec![clear_color; pixels];
    let mut depth = clear_depth.map(|value| vec![value; pixels]);
    for (index, draw) in draws.iter().enumerate() {
        if depth.is_none() && (draw.compare != CompareFunction::Always || draw.write) {
            return Err(DepthRefError::DepthStateWithoutAttachment(index));
        }
        if draw.fill == RefFill::Texture && texture.is_none() {
            return Err(DepthRefError::MissingTexture);
        }
        if draw.positions.iter().any(|p| p[3] != 1.0) {
            return Err(DepthRefError::NonUnitW);
        }
        // The reference does not clip, so depth must already lie in the
        // viewport range 0..=1 (this also rejects non-finite depth).
        if draw
            .positions
            .iter()
            .any(|p| !p.iter().all(|v| v.is_finite()) || !(0.0..=1.0).contains(&p[2]))
        {
            return Err(DepthRefError::Unclipped(index));
        }
        // Viewport transform: NDC x, y in -1..1 map to pixels, y down.
        let screen = draw.positions.map(|[x, y, z, _]| {
            [
                (f64::from(x) + 1.0) * 0.5 * width as f64,
                (1.0 - f64::from(y)) * 0.5 * height as f64,
                f64::from(z),
            ]
        });
        let area = edge(screen[0], screen[1], screen[2]);
        if area == 0.0 {
            continue;
        }
        for y in 0..height {
            for x in 0..width {
                let centre = [x as f64 + 0.5, y as f64 + 0.5, 0.0];
                let weights = [
                    edge(screen[1], screen[2], centre),
                    edge(screen[2], screen[0], centre),
                    edge(screen[0], screen[1], centre),
                ];
                if weights.contains(&0.0) {
                    let others_inside = weights
                        .iter()
                        .filter(|&&w| w != 0.0)
                        .all(|&w| (w > 0.0) == (area > 0.0));
                    if others_inside {
                        return Err(DepthRefError::EdgeTie(index, x, y));
                    }
                    continue;
                }
                if !weights.iter().all(|&w| (w > 0.0) == (area > 0.0)) {
                    continue;
                }
                let z = (weights[0] * screen[0][2]
                    + weights[1] * screen[1][2]
                    + weights[2] * screen[2][2])
                    / area;
                let z = z as f32;
                let slot = y * width + x;
                if let Some(stored) = depth.as_mut() {
                    if !draw.compare.passes(z, stored[slot]) {
                        continue;
                    }
                    if draw.write {
                        stored[slot] = z;
                    }
                }
                color[slot] = match (draw.fill, texture) {
                    (RefFill::Flat(rgba), _) => rgba,
                    (RefFill::Texture, Some(t)) => {
                        let u = ((x as f64 + 0.5) / width as f64 * t.width as f64) as usize;
                        let v = ((y as f64 + 0.5) / height as f64 * t.height as f64) as usize;
                        t.texels[v.min(t.height - 1) * t.width + u.min(t.width - 1)]
                    }
                    (RefFill::Texture, None) => return Err(DepthRefError::MissingTexture),
                };
            }
        }
    }
    Ok((color, depth))
}

fn edge(a: [f64; 3], b: [f64; 3], p: [f64; 3]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];

    fn draw(positions: [[f32; 4]; 3], compare: CompareFunction, write: bool) -> RefDraw {
        RefDraw {
            positions,
            fill: RefFill::Flat(RED),
            compare,
            write,
        }
    }

    // A 2x2 target; this triangle covers only the centre of pixel (0, 1).
    const LOWER_LEFT: [[f32; 4]; 3] = [
        [-1.0, -1.0, 0.25, 1.0],
        [0.5, -1.0, 0.25, 1.0],
        [-1.0, 0.5, 0.25, 1.0],
    ];

    #[test]
    fn accepts_depth_endpoints_and_maximum_extent() {
        let clear_color = [4, 8, 12, 255];
        for clear_depth in [0.0, 1.0] {
            let (color, depth) =
                render_depth([64, 64], clear_color, Some(clear_depth), &[], None).unwrap();
            assert_eq!(color, vec![clear_color; 4096]);
            assert_eq!(depth, Some(vec![clear_depth; 4096]));
        }
    }

    #[test]
    fn rejects_invalid_clear_depths() {
        for clear_depth in [-0.25, 1.25, f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
            assert!(
                matches!(
                    render_depth([1, 1], [0; 4], Some(clear_depth), &[], None),
                    Err(DepthRefError::InvalidClearDepth)
                ),
                "accepted clear depth {clear_depth}"
            );
        }
    }

    #[test]
    fn rejects_invalid_target_extents() {
        for extent in [[4097, 1], [0, 4], [4, 0], [usize::MAX, 2]] {
            assert!(
                matches!(
                    render_depth(extent, [0; 4], None, &[], None),
                    Err(DepthRefError::Extent)
                ),
                "accepted invalid extent {extent:?}"
            );
        }
    }

    #[test]
    fn covers_by_pixel_centre_and_writes_depth() {
        let (color, depth) = render_depth(
            [2, 2],
            [0; 4],
            Some(1.0),
            &[draw(LOWER_LEFT, CompareFunction::Less, true)],
            None,
        )
        .unwrap();
        assert_eq!(color, [[0; 4], [0; 4], RED, [0; 4]]);
        assert_eq!(depth.unwrap(), [1.0, 1.0, 0.25, 1.0]);
    }

    #[test]
    fn compare_functions_follow_metal_semantics() {
        let expect = [false, true, false, true, false, true, false, true];
        let equal = [false, false, true, true, false, false, true, true];
        for (function, (&less, &eq)) in CompareFunction::ALL.iter().zip(expect.iter().zip(&equal)) {
            assert_eq!(function.passes(0.25, 0.5), less, "{function:?} less");
            assert_eq!(function.passes(0.5, 0.5), eq, "{function:?} equal");
            assert_eq!(
                function.passes(0.75, 0.5),
                !matches!(
                    function,
                    CompareFunction::Never
                        | CompareFunction::Less
                        | CompareFunction::Equal
                        | CompareFunction::LessEqual
                ),
                "{function:?} greater"
            );
        }
    }

    #[test]
    fn rejects_edge_ties_and_depth_without_attachment() {
        let tie = [
            [-1.0, -1.0, 0.5, 1.0],
            [1.0, 1.0, 0.5, 1.0],
            [-1.0, 1.0, 0.5, 1.0],
        ];
        assert!(matches!(
            render_depth(
                [2, 2],
                [0; 4],
                Some(1.0),
                &[draw(tie, CompareFunction::Always, true)],
                None
            ),
            Err(DepthRefError::EdgeTie(0, _, _))
        ));
        let mut far = LOWER_LEFT;
        far[0][2] = 1.25;
        assert_eq!(
            render_depth(
                [2, 2],
                [0; 4],
                Some(1.0),
                &[draw(far, CompareFunction::Always, true)],
                None
            ),
            Err(DepthRefError::Unclipped(0))
        );
        assert_eq!(
            render_depth(
                [2, 2],
                [0; 4],
                None,
                &[draw(LOWER_LEFT, CompareFunction::Less, false)],
                None
            ),
            Err(DepthRefError::DepthStateWithoutAttachment(0))
        );
    }
}
