//! Triangle mesh container and loaders. All loaders return Z-up geometry.

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::io::Read;
use std::path::Path;

#[derive(Default, Clone)]
pub struct Mesh {
    pub verts: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
}

impl Mesh {
    pub fn append(
        &mut self,
        verts: impl IntoIterator<Item = [f32; 3]>,
        faces: impl IntoIterator<Item = [u32; 3]>,
    ) {
        let off = self.verts.len() as u32;
        self.verts.extend(verts);
        let n = self.verts.len() as u32;
        self.faces.extend(
            faces
                .into_iter()
                .map(|f| [f[0] + off, f[1] + off, f[2] + off])
                .filter(|f| f[0] < n && f[1] < n && f[2] < n),
        );
    }
    pub fn y_up_to_z_up(&mut self) {
        for v in &mut self.verts {
            *v = [v[0], -v[2], v[1]];
        }
    }
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Stl,
    ThreeMf,
    Cad,
    Obj,
    Gltf,
    Ply,
    Off,
}

pub const EXTENSIONS: &[&str] = &[
    "stl", "3mf", "step", "stp", "p21", "iges", "igs", "brep", "brp", "obj", "ply", "glb", "gltf",
    "off",
];

pub fn sniff(path: &Path) -> Result<Kind> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let k = match ext.as_str() {
        "stl" => Some(Kind::Stl),
        "3mf" => Some(Kind::ThreeMf),
        "step" | "stp" | "p21" | "iges" | "igs" | "brep" | "brp" => Some(Kind::Cad),
        "obj" => Some(Kind::Obj),
        "glb" | "gltf" => Some(Kind::Gltf),
        "ply" => Some(Kind::Ply),
        "off" => Some(Kind::Off),
        _ => None,
    };
    if let Some(k) = k {
        return Ok(k);
    }
    let mut head = [0u8; 512];
    let n = fs::File::open(path)?.read(&mut head)?;
    let h = &head[..n];
    Ok(if h.starts_with(b"PK") {
        Kind::ThreeMf
    } else if h.starts_with(b"ISO-10303-21") {
        Kind::Cad
    } else if h.starts_with(b"glTF") {
        Kind::Gltf
    } else if h.starts_with(b"ply") {
        Kind::Ply
    } else if h.starts_with(b"OFF") {
        Kind::Off
    } else if n >= 84 || h.trim_ascii_start().starts_with(b"solid") {
        Kind::Stl
    } else {
        bail!("unknown file type")
    })
}

pub fn load(path: &Path, quality: f64) -> Result<Mesh> {
    let m = match sniff(path)? {
        Kind::Stl => load_stl(path)?,
        Kind::ThreeMf => crate::threemf::load(path)?,
        Kind::Cad => crate::cad::load(path, quality)?,
        Kind::Obj => load_obj(path)?,
        Kind::Gltf => load_gltf(path)?,
        Kind::Ply => load_ply(path)?,
        Kind::Off => load_off(path)?,
    };
    if m.is_empty() {
        bail!("model contains no triangles");
    }
    Ok(m)
}

// ------------------------------------------------------------------ STL
fn stl_binary(data: &[u8], n: usize) -> Mesh {
    let mut m = Mesh::default();
    m.verts.reserve(n * 3);
    m.faces.reserve(n);
    for i in 0..n {
        let o = 84 + i * 50 + 12;
        for k in 0..3 {
            let p = o + k * 12;
            let f =
                |j: usize| f32::from_le_bytes(data[p + j * 4..p + j * 4 + 4].try_into().unwrap());
            m.verts.push([f(0), f(1), f(2)]);
        }
        let b = (i * 3) as u32;
        m.faces.push([b, b + 1, b + 2]);
    }
    m
}

pub fn load_stl(path: &Path) -> Result<Mesh> {
    let data = fs::read(path)?;
    if data.len() >= 84 {
        let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
        if n > 0 && 84 + n * 50 == data.len() {
            return Ok(stl_binary(&data, n));
        }
    }
    // ASCII
    let mut m = Mesh::default();
    let text = String::from_utf8_lossy(&data);
    let mut it = text.split_ascii_whitespace();
    while let Some(tok) = it.next() {
        if tok.eq_ignore_ascii_case("vertex") {
            let mut v = [0f32; 3];
            for c in &mut v {
                *c = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            }
            m.verts.push(v);
        }
    }
    if m.verts.len() >= 3 {
        let n = m.verts.len() / 3;
        m.verts.truncate(n * 3);
        m.faces = (0..n as u32)
            .map(|i| [i * 3, i * 3 + 1, i * 3 + 2])
            .collect();
        return Ok(m);
    }
    if data.len() > 84 {
        // binary STL with a wrong triangle count in the header
        return Ok(stl_binary(&data, (data.len() - 84) / 50));
    }
    bail!("no triangles in STL")
}

// ------------------------------------------------------------------ OBJ
pub fn load_obj(path: &Path) -> Result<Mesh> {
    let opts = tobj::LoadOptions {
        triangulate: true,
        single_index: false,
        ignore_points: true,
        ignore_lines: true,
    };
    let (models, _) = tobj::load_obj(path, &opts).context("OBJ parse error")?;
    let mut m = Mesh::default();
    for model in models {
        let p = &model.mesh.positions;
        let verts = p.chunks_exact(3).map(|c| [c[0], c[1], c[2]]);
        let faces: Vec<[u32; 3]> = model
            .mesh
            .indices
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        m.append(verts, faces);
    }
    m.y_up_to_z_up();
    Ok(m)
}

// ------------------------------------------------------------------ glTF / GLB
type Mat4 = [[f32; 4]; 4];

fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    // column-major (glTF): (a*b)[c][r] = sum_k a[k][r]*b[c][k]
    let mut o = [[0f32; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            o[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    o
}

pub fn load_gltf(path: &Path) -> Result<Mesh> {
    let (doc, buffers, _) = gltf::import(path).context("glTF parse error")?;
    let mut m = Mesh::default();
    let ident: Mat4 = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];
    let scene = doc.default_scene().or_else(|| doc.scenes().next());
    let roots: Vec<gltf::Node> = match &scene {
        Some(s) => s.nodes().collect(),
        None => doc.nodes().collect(),
    };
    let mut stack: Vec<(gltf::Node, Mat4, u32)> =
        roots.into_iter().map(|n| (n, ident, 0)).collect();
    while let Some((node, parent, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let tf = mat_mul(&parent, &node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let r = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let Some(pos) = r.read_positions() else {
                    continue;
                };
                let verts: Vec<[f32; 3]> = pos
                    .map(|p| {
                        let mut o = [0f32; 3];
                        for (i, oi) in o.iter_mut().enumerate() {
                            *oi = tf[0][i] * p[0] + tf[1][i] * p[1] + tf[2][i] * p[2] + tf[3][i];
                        }
                        o
                    })
                    .collect();
                let idx: Vec<u32> = match r.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..verts.len() as u32).collect(),
                };
                m.append(
                    verts,
                    idx.chunks_exact(3)
                        .map(|c| [c[0], c[1], c[2]])
                        .collect::<Vec<_>>(),
                );
            }
        }
        for ch in node.children() {
            stack.push((ch, tf, depth + 1));
        }
    }
    m.y_up_to_z_up();
    Ok(m)
}

// ------------------------------------------------------------------ OFF
pub fn load_off(path: &Path) -> Result<Mesh> {
    let text = fs::read_to_string(path)?;
    let mut toks = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .flat_map(|l| l.split_ascii_whitespace())
        .peekable();
    let head = toks.next().ok_or_else(|| anyhow!("empty OFF"))?;
    if !head.ends_with("OFF") {
        bail!("not an OFF file");
    }
    let mut num = || -> Result<f64> {
        toks.next()
            .ok_or_else(|| anyhow!("truncated OFF"))?
            .parse()
            .map_err(Into::into)
    };
    let (nv, nf, _) = (num()? as usize, num()? as usize, num()?);
    let mut m = Mesh::default();
    for _ in 0..nv {
        m.verts.push([num()? as f32, num()? as f32, num()? as f32]);
    }
    for _ in 0..nf {
        let k = num()? as usize;
        let idx: Vec<u32> = (0..k)
            .map(|_| num().map(|x| x as u32))
            .collect::<Result<_>>()?;
        for i in 1..k.saturating_sub(1) {
            m.faces.push([idx[0], idx[i], idx[i + 1]]);
        }
    }
    let n = m.verts.len() as u32;
    m.faces.retain(|f| f.iter().all(|&i| i < n));
    Ok(m)
}

// ------------------------------------------------------------------ PLY
#[derive(Clone, Copy)]
enum PT {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

fn ptype(s: &str) -> Result<PT> {
    Ok(match s {
        "char" | "int8" => PT::I8,
        "uchar" | "uint8" => PT::U8,
        "short" | "int16" => PT::I16,
        "ushort" | "uint16" => PT::U16,
        "int" | "int32" => PT::I32,
        "uint" | "uint32" => PT::U32,
        "float" | "float32" => PT::F32,
        "double" | "float64" => PT::F64,
        _ => bail!("unknown PLY type {s}"),
    })
}

enum Prop {
    Scalar(String, PT),
    List(String, PT, PT),
}

struct Elem {
    name: String,
    count: usize,
    props: Vec<Prop>,
}

struct PlyReader<'a> {
    data: &'a [u8],
    pos: usize,
    le: bool,
    ascii: Option<std::str::SplitAsciiWhitespace<'a>>,
}

impl PlyReader<'_> {
    fn read(&mut self, t: PT) -> Result<f64> {
        if let Some(it) = &mut self.ascii {
            return it
                .next()
                .ok_or_else(|| anyhow!("truncated PLY"))?
                .parse()
                .map_err(Into::into);
        }
        let sz = match t {
            PT::I8 | PT::U8 => 1,
            PT::I16 | PT::U16 => 2,
            PT::I32 | PT::U32 | PT::F32 => 4,
            PT::F64 => 8,
        };
        let b = self
            .data
            .get(self.pos..self.pos + sz)
            .ok_or_else(|| anyhow!("truncated PLY"))?;
        self.pos += sz;
        let mut a = [0u8; 8];
        a[..sz].copy_from_slice(b);
        if !self.le {
            a[..sz].reverse();
        }
        Ok(match t {
            PT::I8 => a[0] as i8 as f64,
            PT::U8 => a[0] as f64,
            PT::I16 => i16::from_le_bytes([a[0], a[1]]) as f64,
            PT::U16 => u16::from_le_bytes([a[0], a[1]]) as f64,
            PT::I32 => i32::from_le_bytes(a[..4].try_into()?) as f64,
            PT::U32 => u32::from_le_bytes(a[..4].try_into()?) as f64,
            PT::F32 => f32::from_le_bytes(a[..4].try_into()?) as f64,
            PT::F64 => f64::from_le_bytes(a),
        })
    }
}

pub fn load_ply(path: &Path) -> Result<Mesh> {
    let data = fs::read(path)?;
    let end = data
        .windows(10)
        .position(|w| w == b"end_header")
        .ok_or_else(|| anyhow!("PLY header missing"))?;
    let header = String::from_utf8_lossy(&data[..end]).to_string();
    let mut body = end + 10;
    while body < data.len() && (data[body] == b'\r' || data[body] == b'\n') {
        body += 1;
        if data[body - 1] == b'\n' {
            break;
        }
    }
    let mut format = "ascii";
    let mut elems: Vec<Elem> = vec![];
    for line in header.lines() {
        let t: Vec<&str> = line.split_ascii_whitespace().collect();
        match t.as_slice() {
            ["format", f, ..] => {
                format = if f.contains("big") {
                    "be"
                } else if f.contains("little") {
                    "le"
                } else {
                    "ascii"
                }
            }
            ["element", name, n] => elems.push(Elem {
                name: name.to_string(),
                count: n.parse()?,
                props: vec![],
            }),
            ["property", "list", c, i, name] => {
                if let Some(e) = elems.last_mut() {
                    e.props
                        .push(Prop::List(name.to_string(), ptype(c)?, ptype(i)?))
                }
            }
            ["property", ty, name] => {
                if let Some(e) = elems.last_mut() {
                    e.props.push(Prop::Scalar(name.to_string(), ptype(ty)?))
                }
            }
            _ => {}
        }
    }
    let ascii_text;
    let mut r = PlyReader {
        data: &data,
        pos: body,
        le: format != "be",
        ascii: None,
    };
    if format == "ascii" {
        ascii_text = String::from_utf8_lossy(&data[body..]).to_string();
        r.ascii = Some(ascii_text.split_ascii_whitespace());
    }
    let mut m = Mesh::default();
    for e in &elems {
        for _ in 0..e.count {
            let mut v = [0f32; 3];
            for p in &e.props {
                match p {
                    Prop::Scalar(name, t) => {
                        let x = r.read(*t)?;
                        if e.name == "vertex" {
                            match name.as_str() {
                                "x" => v[0] = x as f32,
                                "y" => v[1] = x as f32,
                                "z" => v[2] = x as f32,
                                _ => {}
                            }
                        }
                    }
                    Prop::List(name, ct, it) => {
                        let k = r.read(*ct)? as usize;
                        let mut idx = Vec::with_capacity(k);
                        for _ in 0..k {
                            idx.push(r.read(*it)? as u32);
                        }
                        if e.name == "face" && (name == "vertex_indices" || name == "vertex_index")
                        {
                            for i in 1..k.saturating_sub(1) {
                                m.faces.push([idx[0], idx[i], idx[i + 1]]);
                            }
                        }
                    }
                }
            }
            if e.name == "vertex" {
                m.verts.push(v);
            }
        }
    }
    let n = m.verts.len() as u32;
    m.faces.retain(|f| f.iter().all(|&i| i < n));
    Ok(m)
}
