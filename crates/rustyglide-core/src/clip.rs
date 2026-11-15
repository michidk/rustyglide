//! Polygon clipping against the clip window for the `gu*WithClip` calls.
//!
//! Every attribute of a `GrVertex` is linear in screen space (texture
//! coordinates are already divided by w), so clipped vertices interpolate all
//! fields linearly.

use crate::vertex::GrVertex;

fn lerp(a: &GrVertex, b: &GrVertex, t: f32) -> GrVertex {
    let fa: &[f32; 15] = bytemuck::cast_ref(a);
    let fb: &[f32; 15] = bytemuck::cast_ref(b);
    let mut out = [0.0f32; 15];
    for i in 0..15 {
        out[i] = fa[i] + (fb[i] - fa[i]) * t;
    }
    bytemuck::cast(out)
}

/// Clips a convex polygon to `[min_x, min_y, max_x, max_y]` (max exclusive).
pub fn clip_polygon(polygon: &[GrVertex], [min_x, min_y, max_x, max_y]: [f32; 4]) -> Vec<GrVertex> {
    let mut poly = polygon.to_vec();
    // Each plane: the coordinate it bounds, its limit, and which side is inside.
    type Plane = (fn(&GrVertex) -> f32, f32, bool);
    let planes: [Plane; 4] =
        [(|v| v.x, min_x, true), (|v| v.x, max_x, false), (|v| v.y, min_y, true), (|v| v.y, max_y, false)];
    for (coord, limit, keep_above) in planes {
        let inside = |v: &GrVertex| if keep_above { coord(v) >= limit } else { coord(v) <= limit };
        let mut out = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let (a, b) = (&poly[i], &poly[(i + 1) % poly.len()]);
            if inside(a) {
                out.push(*a);
            }
            if inside(a) != inside(b) {
                let t = (limit - coord(a)) / (coord(b) - coord(a));
                out.push(lerp(a, b, t));
            }
        }
        poly = out;
        if poly.len() < 3 {
            return Vec::new();
        }
    }
    poly
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32, r: f32) -> GrVertex {
        GrVertex { x, y, r, ..Default::default() }
    }

    #[test]
    fn clips_and_interpolates() {
        let out =
            clip_polygon(&[v(-10.0, 0.0, 0.0), v(10.0, 0.0, 200.0), v(10.0, 10.0, 200.0)], [0.0, 0.0, 100.0, 100.0]);
        assert!(out.iter().all(|p| p.x >= 0.0));
        let cut = out.iter().find(|p| p.x == 0.0 && p.y == 0.0).expect("edge crossing");
        assert_eq!(cut.r, 100.0);
    }

    #[test]
    fn outside_polygons_vanish() {
        assert!(
            clip_polygon(&[v(-5.0, 0.0, 0.0), v(-1.0, 0.0, 0.0), v(-1.0, 5.0, 0.0)], [0.0, 0.0, 10.0, 10.0]).is_empty()
        );
    }
}
