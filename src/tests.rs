//! Unit tests. Fixtures are generated on the fly, so no binary test files live in the repository.

use crate::mesh::{self, Kind, Mesh};
use crate::{output, render, threemf};
use std::io::Write;
use std::path::{Path, PathBuf};

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("modelthumb-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// unit cube, 8 vertices / 12 triangles, outward winding
fn cube() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let v = vec![
        [0., 0., 0.],
        [1., 0., 0.],
        [1., 1., 0.],
        [0., 1., 0.],
        [0., 0., 1.],
        [1., 0., 1.],
        [1., 1., 1.],
        [0., 1., 1.],
    ];
    let f = vec![
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    (v, f)
}

fn bbox(m: &Mesh) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for f in &m.faces {
        for &i in f {
            let v = m.verts[i as usize];
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
    }
    (lo, hi)
}

fn write_binary_stl(path: &Path) {
    let (v, f) = cube();
    let mut b = vec![0u8; 80];
    b.extend((f.len() as u32).to_le_bytes());
    for t in &f {
        b.extend([0u8; 12]);
        for &i in t {
            for c in v[i as usize] {
                b.extend(c.to_le_bytes());
            }
        }
        b.extend([0u8; 2]);
    }
    std::fs::write(path, b).unwrap();
}

fn write_3mf(path: &Path, with_components: bool) {
    let (v, f) = cube();
    let verts: String = v
        .iter()
        .map(|p| format!("<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", p[0], p[1], p[2]))
        .collect();
    let tris: String = f
        .iter()
        .map(|t| {
            format!(
                "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>",
                t[0], t[1], t[2]
            )
        })
        .collect();
    let (extra, item) = if with_components {
        (
            r#"<object id="2" type="model"><components>
                 <component objectid="1"/>
                 <component objectid="1" transform="1 0 0 0 1 0 0 0 1 3 0 0"/>
               </components></object>"#,
            r#"<item objectid="2" transform="2 0 0 0 2 0 0 0 2 0 0 10"/>"#,
        )
    } else {
        ("", r#"<item objectid="1"/>"#)
    };
    let model = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
<resources><object id="1" type="model"><mesh><vertices>{verts}</vertices><triangles>{tris}</triangles></mesh></object>{extra}</resources>
<build>{item}</build></model>"#
    );
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("[Content_Types].xml", o).unwrap();
    z.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#).unwrap();
    z.start_file("_rels/.rels", o).unwrap();
    z.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#).unwrap();
    z.start_file("3D/3dmodel.model", o).unwrap();
    z.write_all(model.as_bytes()).unwrap();
    z.finish().unwrap();
}

#[test]
fn stl_binary() {
    let d = tmpdir("stlb");
    let p = d.join("cube.stl");
    write_binary_stl(&p);
    let m = mesh::load(&p, 1.0).unwrap();
    assert_eq!(m.faces.len(), 12);
    assert_eq!(bbox(&m), ([0.; 3], [1.; 3]));
}

#[test]
fn stl_ascii() {
    let d = tmpdir("stla");
    let p = d.join("tri.stl");
    std::fs::write(
        &p,
        "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 2 0 0\nvertex 0 3 0\nendloop\nendfacet\nendsolid t\n",
    )
    .unwrap();
    let m = mesh::load(&p, 1.0).unwrap();
    assert_eq!(m.faces.len(), 1);
    assert_eq!(bbox(&m).1, [2., 3., 0.]);
}

#[test]
fn stl_garbage_is_error() {
    let d = tmpdir("stlg");
    let p = d.join("bad.stl");
    std::fs::write(&p, "not a model").unwrap();
    assert!(mesh::load(&p, 1.0).is_err());
}

#[test]
fn threemf_simple() {
    let d = tmpdir("3mf");
    let p = d.join("cube.3mf");
    write_3mf(&p, false);
    let m = threemf::load(&p).unwrap();
    assert_eq!(m.faces.len(), 12);
    assert_eq!(bbox(&m), ([0.; 3], [1.; 3]));
}

#[test]
fn threemf_components_and_transforms() {
    let d = tmpdir("3mfc");
    let p = d.join("comp.3mf");
    write_3mf(&p, true);
    let m = threemf::load(&p).unwrap();
    assert_eq!(m.faces.len(), 24);
    // two cubes (x 0..1 and 3..4), then scaled by 2 and lifted by 10
    assert_eq!(bbox(&m), ([0., 0., 10.], [8., 2., 12.]));
}

#[test]
fn threemf_embed_roundtrip() {
    let d = tmpdir("embed");
    let p = d.join("cube.3mf");
    write_3mf(&p, false);
    assert!(threemf::thumbnail(&p).is_none());
    let m = threemf::load(&p).unwrap();
    let img = render::render(
        &m,
        &render::Options {
            size: 64,
            ..Default::default()
        },
    );
    let png = output::encode_png(&img, &[]).unwrap();
    output::embed_3mf(&p, &png).unwrap();
    output::embed_3mf(&p, &png).unwrap(); // replacing must not duplicate the relationship
    assert_eq!(threemf::thumbnail(&p).unwrap(), png);
    // model must still load and the package must contain exactly one thumbnail relationship
    assert_eq!(threemf::load(&p).unwrap().faces.len(), 12);
    let mut z = zip::ZipArchive::new(std::fs::File::open(&p).unwrap()).unwrap();
    let mut rels = String::new();
    std::io::Read::read_to_string(&mut z.by_name("_rels/.rels").unwrap(), &mut rels).unwrap();
    assert_eq!(rels.matches(threemf::THUMB_REL).count(), 1);
}

#[test]
fn off_and_ply() {
    let d = tmpdir("offply");
    let off = d.join("q.off");
    std::fs::write(&off, "OFF\n4 1 0\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n4 0 1 2 3\n").unwrap();
    assert_eq!(mesh::load(&off, 1.0).unwrap().faces.len(), 2);

    let ply = d.join("q.ply");
    std::fs::write(
        &ply,
        "ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\n\
         element face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n4 0 1 2 3\n",
    )
    .unwrap();
    let m = mesh::load(&ply, 1.0).unwrap();
    assert_eq!(m.faces.len(), 2);

    // binary little endian PLY
    let blp = d.join("b.ply");
    let mut b = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar uint vertex_indices\nend_header\n".to_vec();
    for v in [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
        for c in v {
            b.extend(c.to_le_bytes());
        }
    }
    b.push(3);
    for i in [0u32, 1, 2] {
        b.extend(i.to_le_bytes());
    }
    std::fs::write(&blp, b).unwrap();
    assert_eq!(mesh::load(&blp, 1.0).unwrap().faces.len(), 1);
}

#[test]
fn sniff_by_content() {
    let d = tmpdir("sniff");
    let p = d.join("noext");
    std::fs::write(&p, "ISO-10303-21;\nHEADER;").unwrap();
    assert_eq!(mesh::sniff(&p).unwrap(), Kind::Cad);
}

#[test]
fn render_cube_has_coverage_and_transparency() {
    let (v, f) = cube();
    let m = Mesh { verts: v, faces: f };
    let img = render::render(
        &m,
        &render::Options {
            size: 128,
            ..Default::default()
        },
    );
    assert_eq!((img.w, img.h), (128, 128));
    let opaque = img.px.chunks(4).filter(|p| p[3] == 255).count();
    let clear = img.px.chunks(4).filter(|p| p[3] == 0).count();
    assert!(
        opaque > 128 * 128 / 5,
        "model should cover a good part of the image"
    );
    assert!(clear > 128 * 128 / 10, "background should be transparent");
    assert_eq!(img.px[3], 0, "corner must be transparent");
}

#[test]
fn render_with_background_is_opaque() {
    let (v, f) = cube();
    let m = Mesh { verts: v, faces: f };
    let o = render::Options {
        size: 32,
        background: Some([255, 255, 255, 255]),
        ..Default::default()
    };
    let img = render::render(&m, &o);
    assert!(img.px.chunks(4).all(|p| p[3] == 255));
    assert_eq!(&img.px[..3], &[255, 255, 255]);
}

#[test]
fn freedesktop_cache_naming() {
    // the spec names cache files md5(URI).png; check URI escaping and hash against known values
    let d = tmpdir("uri");
    let p = d.join("a b#c.stl");
    std::fs::write(&p, "x").unwrap();
    let uri = output::file_uri(&p);
    assert!(uri.starts_with("file:///"));
    assert!(uri.ends_with("/a%20b%23c.stl"), "{uri}");
    let cp = output::cache_path(Path::new("/tmp/t/a.stl"), "normal");
    assert!(
        cp.ends_with("normal/1009eced91b64850db2578b5cc2ad9ab.png"),
        "{}",
        cp.display()
    );
}
