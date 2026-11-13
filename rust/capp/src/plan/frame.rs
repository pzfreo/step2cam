//! Vector arithmetic and setup frames: which setup an axis belongs to, and each setup's coordinates.

use super::*;

pub(super) fn dot(a: V3, b: V3) -> f64 {
    let mut sum = 0.;
    for i in 0..3 {
        sum += a[i] * b[i];
    }
    sum
}
pub(super) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(super) fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub(super) fn rotate(rows: &Rows, p: V3) -> V3 {
    [dot(rows[0], p), dot(rows[1], p), dot(rows[2], p)]
}
pub(super) fn unit(v: V3) -> Result<V3, String> {
    let n = dot(v, v).sqrt();
    if n == 0. || !n.is_finite() {
        return Err(format!("axis {} has no direction", show3(v)));
    }
    Ok([v[0] / n, v[1] / n, v[2] / n])
}

pub(super) fn setup_for(axis: V3) -> Result<Option<&'static str>, String> {
    let a = unit(axis)?;
    Ok(SETUPS.iter().find(|(_, d, _)| dot(a, *d) > 1. - AXIS_TOLERANCE).map(|(n, _, _)| *n))
}
pub(super) fn rows_of(name: &str) -> &'static Rows {
    &SETUPS.iter().find(|(n, _, _)| *n == name).expect("known setup").2
}
pub(super) fn box_corners(lo: V3, hi: V3) -> [V3; 8] {
    let mut out = [[0.; 3]; 8];
    let mut n = 0;
    for x in [lo[0], hi[0]] {
        for y in [lo[1], hi[1]] {
            for z in [lo[2], hi[2]] {
                out[n] = [x, y, z];
                n += 1;
            }
        }
    }
    out
}
pub(super) fn bounds(points: &[V3]) -> (V3, V3) {
    let mut lo = points[0];
    let mut hi = points[0];
    for p in points {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    (lo, hi)
}

/// One setup's coordinates: model point -> rotate -> translate to the setup's zero.
pub(super) struct Frame {
    pub(super) rows: &'static Rows,
    pub(super) zeroed_on: &'static str,
    shift: V3,
    pub(super) stock: (V3, V3),
    origin_model: V3,
    triangles: OnceCell<Vec<Tri>>,
}

impl Frame {
    pub(super) fn new(name: &str, box_lo: V3, box_hi: V3, zeroed_on: &'static str) -> Self {
        let rows = rows_of(name);
        let (lo, hi) = bounds(&box_corners(box_lo, box_hi).map(|p| rotate(rows, p)));
        let shift = [-lo[0], -lo[1], -hi[2]];
        let point = |p: V3| add(rotate(rows, p), shift);
        let stock = bounds(&box_corners(box_lo, box_hi).map(point));
        // The rows are orthonormal, so the inverse is the transpose.
        let back = [-shift[0], -shift[1], -shift[2]];
        let mut origin_model = [0.; 3];
        for j in 0..3 {
            let mut sum = 0.;
            for i in 0..3 {
                sum += rows[i][j] * back[i];
            }
            origin_model[j] = sum;
        }
        Self { rows, zeroed_on, shift, stock, origin_model, triangles: OnceCell::new() }
    }
    pub(super) fn point(&self, p: V3) -> V3 {
        add(rotate(self.rows, p), self.shift)
    }
    pub(super) fn triangles(&self, mesh: &[Tri]) -> &[Tri] {
        self.triangles.get_or_init(|| mesh.iter().map(|t| t.map(|p| self.point(p))).collect())
    }
    pub(super) fn describe(&self, id: &str, operations: Vec<model::Operation>) -> model::Setup {
        model::Setup {
            id: id.into(),
            zero: Some(self.zeroed_on.into()),
            rotation: Some(self.rows.map(|row| row.map(|v| v as i8))),
            origin_model: Some(rp(self.origin_model)),
            stock: model::Box3 { min: rp(self.stock.0), max: rp(self.stock.1) },
            operations,
            machine: None,
            stock_note: None,
            flip_role: None,
            part_surface: None,
            clamping: None,
            flip_seating: None,
            before_setup: None,
            grip_surface_model: None,
        }
    }
}

/// Rigid transform from one recorded setup frame (its rotation and origin) to another. Uses the
/// same rotation and origin that Frame::describe records for operations and stock.
/// R_to * transpose(R_from), R_to * (origin_from - origin_to).
pub fn between_setups(from: (Option<[[i8; 3]; 3]>, Option<V3>), to: (Option<[[i8; 3]; 3]>, Option<V3>)) -> Result<(Rows, V3), String> {
    let missing = || "expected a list of numbers".to_string();
    let read_rows = |rows: Option<[[i8; 3]; 3]>| rows.map(|rows| rows.map(|row| row.map(f64::from))).ok_or_else(missing);
    let a = read_rows(from.0)?;
    let b = read_rows(to.0)?;
    let rotation = std::array::from_fn(|i| std::array::from_fn(|j| dot(b[i], a[j])));
    let offset = add(from.1.ok_or_else(missing)?, scale(to.1.ok_or_else(missing)?, -1.));
    Ok((rotation, rotate(&b, offset)))
}
