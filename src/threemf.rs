//! 3MF reader (core spec + production extension multi-file parts) and embedded thumbnail access.

use crate::mesh::Mesh;
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

pub const THUMB_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail";
const MODEL_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

/// Row-vector affine transform as in the 3MF spec: p' = p.x*r0 + p.y*r1 + p.z*r2 + r3
type Tf = [[f64; 3]; 4];
const IDENT: Tf = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [0., 0., 0.]];

fn parse_tf(s: Option<String>) -> Tf {
    let Some(s) = s else { return IDENT };
    let v: Vec<f64> = s.split_ascii_whitespace().filter_map(|x| x.parse().ok()).collect();
    if v.len() != 12 {
        return IDENT;
    }
    [[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]], [v[9], v[10], v[11]]]
}

/// apply a, then b
fn compose(a: &Tf, b: &Tf) -> Tf {
    let mut o = [[0.0; 3]; 4];
    for i in 0..4 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum::<f64>() + if i == 3 { b[3][j] } else { 0.0 };
        }
    }
    o
}

fn apply(t: &Tf, p: [f64; 3]) -> [f32; 3] {
    let mut o = [0f32; 3];
    for (j, oj) in o.iter_mut().enumerate() {
        *oj = (p[0] * t[0][j] + p[1] * t[1][j] + p[2] * t[2][j] + t[3][j]) as f32;
    }
    o
}

enum Obj {
    Mesh(Vec<[f64; 3]>, Vec<[u32; 3]>),
    Comps(Vec<(String, String, Tf)>), // (part path, object id, transform)
}

fn local(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&c| c == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

fn attrs(e: &BytesStart) -> HashMap<Vec<u8>, String> {
    e.attributes()
        .flatten()
        .map(|a| (local(a.key.as_ref()).to_vec(), a.unescape_value().map(|v| v.into_owned()).unwrap_or_default()))
        .collect()
}

fn norm_path(p: &str) -> String {
    p.trim_start_matches('/').to_string()
}

struct Part {
    objects: HashMap<String, Obj>,
    items: Vec<(String, Tf, String)>,
}

fn parse_part(xml: &[u8], part: &str) -> Result<Part> {
    let mut rd = Reader::from_reader(xml);
    rd.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut objects = HashMap::new();
    let mut items = vec![];
    let mut cur_id: Option<String> = None;
    let mut verts: Vec<[f64; 3]> = vec![];
    let mut tris: Vec<[u32; 3]> = vec![];
    let mut comps: Vec<(String, String, Tf)> = vec![];
    let mut has_mesh = false;
    let num = |a: &HashMap<Vec<u8>, String>, k: &[u8]| a.get(k).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
    loop {
        let ev = rd.read_event_into(&mut buf).context("3MF XML error")?;
        match &ev {
            Event::Start(e) | Event::Empty(e) => match local(e.name().as_ref()) {
                b"object" => {
                    let a = attrs(e);
                    cur_id = a.get(b"id".as_slice()).cloned();
                    verts.clear();
                    tris.clear();
                    comps.clear();
                    has_mesh = false;
                }
                b"mesh" => has_mesh = true,
                b"vertex" => {
                    let a = attrs(e);
                    verts.push([num(&a, b"x"), num(&a, b"y"), num(&a, b"z")]);
                }
                b"triangle" => {
                    let a = attrs(e);
                    let g = |k: &[u8]| a.get(k).and_then(|s| s.trim().parse::<u32>().ok());
                    if let (Some(a1), Some(a2), Some(a3)) = (g(b"v1"), g(b"v2"), g(b"v3")) {
                        tris.push([a1, a2, a3]);
                    }
                }
                b"component" => {
                    let mut a = attrs(e);
                    let p = a.remove(b"path".as_slice()).map(|p| norm_path(&p)).unwrap_or_else(|| part.to_string());
                    let id = a.remove(b"objectid".as_slice()).unwrap_or_default();
                    comps.push((p, id, parse_tf(a.remove(b"transform".as_slice()))));
                }
                b"item" => {
                    let mut a = attrs(e);
                    let p = a.remove(b"path".as_slice()).map(|p| norm_path(&p)).unwrap_or_else(|| part.to_string());
                    let id = a.remove(b"objectid".as_slice()).unwrap_or_default();
                    items.push((id, parse_tf(a.remove(b"transform".as_slice())), p));
                }
                _ => {}
            },
            Event::End(e) if local(e.name().as_ref()) == b"object" => {
                if let Some(id) = cur_id.take() {
                    let o = if has_mesh {
                        Obj::Mesh(std::mem::take(&mut verts), std::mem::take(&mut tris))
                    } else {
                        Obj::Comps(std::mem::take(&mut comps))
                    };
                    objects.insert(id, o);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(Part { objects, items })
}

fn read_entry(z: &mut zip::ZipArchive<BufReader<File>>, name: &str) -> Option<Vec<u8>> {
    let mut f = z.by_name(name).ok()?;
    let mut v = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut v).ok()?;
    Some(v)
}

/// (Type, Target) pairs from a .rels part
pub fn rels(xml: &[u8]) -> Vec<(String, String)> {
    let mut rd = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut out = vec![];
    while let Ok(ev) = rd.read_event_into(&mut buf) {
        match ev {
            Event::Start(e) | Event::Empty(e) if local(e.name().as_ref()) == b"Relationship" => {
                let a = attrs(&e);
                out.push((
                    a.get(b"Type".as_slice()).cloned().unwrap_or_default(),
                    norm_path(a.get(b"Target".as_slice()).map(String::as_str).unwrap_or("")),
                ));
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Raw bytes of the thumbnail embedded in a 3MF package (slicers like PrusaSlicer, Bambu Studio,
/// Cura and Windows 3D Builder write one), if any.
pub fn thumbnail(path: &Path) -> Option<Vec<u8>> {
    let mut z = zip::ZipArchive::new(BufReader::new(File::open(path).ok()?)).ok()?;
    let mut cands: Vec<String> = read_entry(&mut z, "_rels/.rels")
        .map(|r| rels(&r).into_iter().filter(|(t, _)| t == THUMB_REL).map(|(_, p)| p).collect())
        .unwrap_or_default();
    for c in ["Metadata/thumbnail.png", "Metadata/plate_1.png", "Metadata/top_1.png", "Metadata/thumbnail.jpg"] {
        cands.push(c.into());
    }
    cands.iter().find_map(|c| read_entry(&mut z, c))
}

pub fn load(path: &Path) -> Result<Mesh> {
    let mut z = zip::ZipArchive::new(BufReader::new(File::open(path)?)).map_err(|e| anyhow!("not a 3MF (zip) file: {e}"))?;
    let names: Vec<String> = z.file_names().map(String::from).collect();
    let root = read_entry(&mut z, "_rels/.rels")
        .and_then(|r| rels(&r).into_iter().find(|(t, _)| t == MODEL_REL).map(|(_, p)| p))
        .filter(|p| names.contains(p))
        .or_else(|| names.iter().find(|n| n.starts_with("3D/") && n.to_lowercase().ends_with(".model")).cloned())
        .ok_or_else(|| anyhow!("3MF without model part"))?;

    let mut parts: HashMap<String, Part> = HashMap::new();
    fn ensure(z: &mut zip::ZipArchive<BufReader<File>>, parts: &mut HashMap<String, Part>, p: &str) -> bool {
        if parts.contains_key(p) {
            return true;
        }
        let Some(xml) = read_entry(z, p) else { return false };
        match parse_part(&xml, p) {
            Ok(part) => {
                parts.insert(p.to_string(), part);
                true
            }
            Err(_) => false,
        }
    }
    if !ensure(&mut z, &mut parts, &root) {
        bail!("cannot parse 3MF model part");
    }
    let mut items = parts[&root].items.clone();
    if items.is_empty() {
        items = parts[&root].objects.keys().map(|k| (k.clone(), IDENT, root.clone())).collect();
    }

    let mut mesh = Mesh::default();
    let mut stack: Vec<(String, String, Tf, u32)> = items.into_iter().map(|(id, t, p)| (p, id, t, 0)).collect();
    while let Some((p, id, t, depth)) = stack.pop() {
        if depth > 32 || !ensure(&mut z, &mut parts, &p) {
            continue;
        }
        match parts[&p].objects.get(&id) {
            Some(Obj::Mesh(v, f)) => mesh.append(v.iter().map(|q| apply(&t, *q)), f.iter().copied()),
            Some(Obj::Comps(cs)) => {
                for (cp, cid, ct) in cs {
                    stack.push((cp.clone(), cid.clone(), compose(ct, &t), depth + 1));
                }
            }
            None => {}
        }
    }
    if mesh.is_empty() {
        bail!("3MF contains no mesh");
    }
    Ok(mesh)
}
