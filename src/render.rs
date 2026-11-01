//! CPU software rasterizer: no GPU / display server needed, so it works inside thumbnailer sandboxes.

use crate::mesh::Mesh;
use std::collections::HashMap;

pub struct Options {
    pub size: u32,
    pub color: [f32; 3],
    pub background: Option<[u8; 4]>,
    pub azimuth: f64,
    pub elevation: f64,
    pub margin: f64,
    pub outline: bool,
    pub smooth: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            size: 256,
            color: [0.36, 0.58, 0.86],
            background: None,
            azimuth: -35.0,
            elevation: 28.0,
            margin: 0.06,
            outline: true,
            smooth: true,
        }
    }
}

/// RGBA8 image
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

type V3 = [f64; 3];
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn dot3len(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 0.0 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// Merge vertices with identical positions (STL never shares them) so normals can be smoothed.
fn weld(m: &Mesh) -> (Vec<V3>, Vec<[u32; 3]>) {
    let mut map: HashMap<[u32; 3], u32> = HashMap::with_capacity(m.verts.len() / 4);
    let mut remap = Vec::with_capacity(m.verts.len());
    let mut verts = Vec::new();
    for v in &m.verts {
        let key = [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()];
        let id = *map.entry(key).or_insert_with(|| {
            verts.push([v[0] as f64, v[1] as f64, v[2] as f64]);
            (verts.len() - 1) as u32
        });
        remap.push(id);
    }
    let faces = m.faces.iter().map(|f| [remap[f[0] as usize], remap[f[1] as usize], remap[f[2] as usize]]).collect();
    (verts, faces)
}

const CREASE_COS: f64 = 0.848; // cos(32°): sharper edges stay crisp, curved surfaces get smooth

pub fn render(mesh: &Mesh, o: &Options) -> Image {
    let size = o.size.clamp(16, 4096) as usize;
    let (verts, mut faces) = if o.smooth {
        weld(mesh)
    } else {
        (mesh.verts.iter().map(|v| [v[0] as f64, v[1] as f64, v[2] as f64]).collect(), mesh.faces.clone())
    };
    faces.retain(|f| f.iter().all(|&i| verts[i as usize].iter().all(|c| c.is_finite())));
    let mut ss = if size <= 128 { 4 } else if size <= 384 { 3 } else { 2 };
    if faces.len() > 2_000_000 && ss > 2 {
        ss -= 1;
    }
    let n = size * ss;

    // view rotation (Z-up world, camera on a sphere around the model)
    let (a, e) = (o.azimuth.to_radians(), o.elevation.to_radians());
    let d = [e.cos() * a.sin(), -e.cos() * a.cos(), e.sin()];
    let right = norm(cross([0.0, 0.0, 1.0], d));
    let up = cross(d, right);

    let mut used = vec![false; verts.len()];
    for f in &faces {
        for &i in f {
            used[i as usize] = true;
        }
    }
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for (v, _) in verts.iter().zip(&used).filter(|(_, u)| **u) {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    if faces.is_empty() {
        return Image { w: size as u32, h: size as u32, px: vec![0; size * size * 4] };
    }
    let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let p: Vec<V3> = verts
        .iter()
        .map(|v| {
            let q = sub(*v, c);
            [dot(q, right), dot(q, up), dot(q, d)]
        })
        .collect();
    let (mut l2, mut h2) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (q, _) in p.iter().zip(&used).filter(|(_, u)| **u) {
        for k in 0..2 {
            l2[k] = l2[k].min(q[k]);
            h2[k] = h2[k].max(q[k]);
        }
    }
    let ext = (h2[0] - l2[0]).max(h2[1] - l2[1]).max(1e-12);
    let scale = n as f64 * (1.0 - 2.0 * o.margin) / ext;
    let mid = [(l2[0] + h2[0]) / 2.0, (l2[1] + h2[1]) / 2.0];
    let s: Vec<V3> = p
        .iter()
        .map(|q| [(q[0] - mid[0]) * scale + n as f64 / 2.0, n as f64 / 2.0 - (q[1] - mid[1]) * scale, q[2] * scale])
        .collect();

    // ---------------------------------------------------------------- rasterize into z / face-id buffers
    let mut zbuf = vec![f64::NEG_INFINITY; n * n];
    let mut fbuf = vec![u32::MAX; n * n];
    for (fi, f) in faces.iter().enumerate() {
        let (a, b, cc) = (s[f[0] as usize], s[f[1] as usize], s[f[2] as usize]);
        let area = (b[0] - a[0]) * (cc[1] - a[1]) - (cc[0] - a[0]) * (b[1] - a[1]);
        if area.abs() < 1e-12 {
            continue;
        }
        let x0 = (a[0].min(b[0]).min(cc[0]) - 0.5).floor().max(0.0) as i64;
        let x1 = ((a[0].max(b[0]).max(cc[0]) - 0.5).ceil() as i64).min(n as i64 - 1);
        let y0 = (a[1].min(b[1]).min(cc[1]) - 0.5).floor().max(0.0) as i64;
        let y1 = ((a[1].max(b[1]).max(cc[1]) - 0.5).ceil() as i64).min(n as i64 - 1);
        if x0 > x1 || y0 > y1 {
            continue;
        }
        let inv = 1.0 / area;
        // barycentric weights as linear functions of (x, y)
        let ea = [(b[1] - cc[1]) * inv, (cc[0] - b[0]) * inv, (b[0] * cc[1] - cc[0] * b[1]) * inv];
        let eb = [(cc[1] - a[1]) * inv, (a[0] - cc[0]) * inv, (cc[0] * a[1] - a[0] * cc[1]) * inv];
        let ec = [(a[1] - b[1]) * inv, (b[0] - a[0]) * inv, (a[0] * b[1] - b[0] * a[1]) * inv];
        let zl = [
            ea[0] * a[2] + eb[0] * b[2] + ec[0] * cc[2],
            ea[1] * a[2] + eb[1] * b[2] + ec[1] * cc[2],
            ea[2] * a[2] + eb[2] * b[2] + ec[2] * cc[2],
        ];
        const EPS: f64 = -1e-7;
        for y in y0..=y1 {
            let cy = y as f64 + 0.5;
            let row = y as usize * n;
            for x in x0..=x1 {
                let cx = x as f64 + 0.5;
                if ea[0] * cx + ea[1] * cy + ea[2] < EPS
                    || eb[0] * cx + eb[1] * cy + eb[2] < EPS
                    || ec[0] * cx + ec[1] * cy + ec[2] < EPS
                {
                    continue;
                }
                let z = zl[0] * cx + zl[1] * cy + zl[2];
                let i = row + x as usize;
                if z > zbuf[i] {
                    zbuf[i] = z;
                    fbuf[i] = fi as u32;
                }
            }
        }
    }

    // ---------------------------------------------------------------- normals
    let fnrm: Vec<V3> = faces
        .iter()
        .map(|f| norm(cross(sub(p[f[1] as usize], p[f[0] as usize]), sub(p[f[2] as usize], p[f[0] as usize]))))
        .collect();
    // per-corner normals for visible faces only: average the neighbour faces that meet this face
    // at less than the crease angle. Orientation-agnostic, because CAD tessellators and broken
    // STLs do not wind all triangles consistently.
    let mut corner: HashMap<u32, [V3; 3]> = HashMap::new();
    if o.smooth {
        let mut visible = vec![false; faces.len()];
        for &f in &fbuf {
            if f != u32::MAX {
                visible[f as usize] = true;
            }
        }
        let mut start = vec![0u32; p.len() + 1];
        for f in &faces {
            for &i in f {
                start[i as usize + 1] += 1;
            }
        }
        for i in 0..p.len() {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut adj = vec![0u32; faces.len() * 3];
        for (fi, f) in faces.iter().enumerate() {
            for &i in f {
                adj[fill[i as usize] as usize] = fi as u32;
                fill[i as usize] += 1;
            }
        }
        let area: Vec<f64> = faces
            .iter()
            .map(|f| dot3len(cross(sub(p[f[1] as usize], p[f[0] as usize]), sub(p[f[2] as usize], p[f[0] as usize]))))
            .collect();
        for (fi, f) in faces.iter().enumerate() {
            if !visible[fi] {
                continue;
            }
            let nf = fnrm[fi];
            let mut cn = [[0.0; 3]; 3];
            for k in 0..3 {
                let v = f[k] as usize;
                let mut acc = [0.0; 3];
                for &g in &adj[start[v] as usize..start[v + 1] as usize] {
                    let ng = fnrm[g as usize];
                    let d = dot(ng, nf);
                    if d.abs() >= CREASE_COS {
                        let w = area[g as usize] * d.signum();
                        for j in 0..3 {
                            acc[j] += w * ng[j];
                        }
                    }
                }
                cn[k] = norm(acc);
            }
            corner.insert(fi as u32, cn);
        }
    }

    let lights: [(V3, f64); 3] = [
        (norm([-0.45, 0.55, 0.70]), 0.70), // key, upper left
        (norm([0.70, -0.10, 0.55]), 0.30), // fill, right
        (norm([0.00, -0.80, 0.30]), 0.12), // bounce from below
    ];
    let halfs: Vec<V3> = lights.iter().map(|(l, _)| norm([l[0], l[1], l[2] + 1.0])).collect();

    let mut rgb = vec![0f32; n * n * 3];
    let mut alpha = vec![0u8; n * n];
    for i in 0..n * n {
        let fi = fbuf[i];
        if fi == u32::MAX {
            continue;
        }
        alpha[i] = 1;
        let fi = fi as usize;
        let fn_ = fnrm[fi];
        let mut nr = fn_;
        if o.smooth {
            let f = faces[fi];
            let (a, b, cc) = (s[f[0] as usize], s[f[1] as usize], s[f[2] as usize]);
            let (cx, cy) = ((i % n) as f64 + 0.5, (i / n) as f64 + 0.5);
            let area = (b[0] - a[0]) * (cc[1] - a[1]) - (cc[0] - a[0]) * (b[1] - a[1]);
            let wa = ((b[0] - cx) * (cc[1] - cy) - (cc[0] - cx) * (b[1] - cy)) / area;
            let wb = ((cc[0] - cx) * (a[1] - cy) - (a[0] - cx) * (cc[1] - cy)) / area;
            let w = [wa.clamp(0.0, 1.0), wb.clamp(0.0, 1.0), (1.0 - wa - wb).clamp(0.0, 1.0)];
            let mut acc = [0.0; 3];
            let cn = corner[&(fi as u32)];
            for k in 0..3 {
                let c = cn[k];
                for j in 0..3 {
                    acc[j] += w[k] * c[j];
                }
            }
            let a2 = norm(acc);
            if dot(a2, a2) > 0.5 {
                nr = a2;
            }
        }
        if fn_[2] < 0.0 {
            // back side visible (open / inconsistently wound mesh): light it like the front
            nr = [-nr[0], -nr[1], -nr[2]];
        }
        let mut diff = 0.28;
        let mut spec = 0.0;
        for ((l, k), h) in lights.iter().zip(&halfs) {
            diff += k * dot(nr, *l).max(0.0);
            spec += 0.35 * k * dot(nr, *h).max(0.0).powi(32);
        }
        for j in 0..3 {
            rgb[i * 3 + j] = (o.color[j] as f64 * diff + spec).clamp(0.0, 1.0) as f32;
        }
    }

    // ---------------------------------------------------------------- outlines at depth discontinuities
    if o.outline {
        let (mut zmin, mut zmax) = (f64::MAX, f64::MIN);
        for i in 0..n * n {
            if alpha[i] != 0 {
                zmin = zmin.min(zbuf[i]);
                zmax = zmax.max(zbuf[i]);
            }
        }
        let zr = (zmax - zmin).max(1e-9);
        let far = zmin - 10.0 * zr;
        let zz = |i: usize| if alpha[i] != 0 { zbuf[i] } else { far };
        let thr = zr * 0.04;
        let mut edge = vec![false; n * n];
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                for j in [if x + 1 < n { Some(i + 1) } else { None }, if y + 1 < n { Some(i + n) } else { None }]
                    .into_iter()
                    .flatten()
                {
                    let (a, b) = (zz(i), zz(j));
                    if (a - b).abs() > thr {
                        edge[if a < b { i } else { j }] = true; // line on the farther pixel
                    }
                }
            }
        }
        let k = (ss / 2).max(1);
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                if alpha[i] == 0 {
                    continue;
                }
                let mut hit = false;
                'o: for dy in 0..k {
                    for dx in 0..k {
                        if y >= dy && x >= dx && edge[(y - dy) * n + (x - dx)] {
                            hit = true;
                            break 'o;
                        }
                    }
                }
                if hit {
                    for j in 0..3 {
                        rgb[i * 3 + j] *= 0.45;
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- downsample (premultiplied box filter)
    let mut out = vec![0u8; size * size * 4];
    let inv = 1.0 / (ss * ss) as f32;
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let i = (y * ss + sy) * n + x * ss + sx;
                    if alpha[i] != 0 {
                        acc[0] += rgb[i * 3];
                        acc[1] += rgb[i * 3 + 1];
                        acc[2] += rgb[i * 3 + 2];
                        acc[3] += 1.0;
                    }
                }
            }
            let a = acc[3] * inv;
            let o4 = (y * size + x) * 4;
            let mut px = [0f32; 4];
            if acc[3] > 0.0 {
                for j in 0..3 {
                    px[j] = acc[j] / acc[3];
                }
            }
            px[3] = a;
            if let Some(bg) = o.background {
                for j in 0..3 {
                    px[j] = px[j] * a + bg[j] as f32 / 255.0 * (1.0 - a);
                }
                px[3] = 1.0;
            }
            for j in 0..4 {
                out[o4 + j] = (px[j] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            }
        }
    }
    Image { w: size as u32, h: size as u32, px: out }
}
