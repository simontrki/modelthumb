//! STEP / IGES / BREP via the gmsh C API (OpenCASCADE kernel), loaded at runtime.
//! The AppImage ships libgmsh in usr/lib; a system libgmsh is used as a fallback.

use crate::mesh::Mesh;
use anyhow::{anyhow, bail, Result};
use libloading::os::unix::{Library, Symbol, RTLD_GLOBAL, RTLD_NOW};
use std::ffi::{c_char, c_int, c_void, CString};
use std::path::{Path, PathBuf};

fn lib_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("lib"))
}

/// libgmsh needs libGLU/libXcursor/... which minimal desktops may lack: preload bundled copies.
fn preload_fallbacks() {
    let Some(dir) = lib_dir().map(|d| d.join("fallback")) else {
        return;
    };
    for so in [
        "libXrender.so.1",
        "libXfixes.so.3",
        "libXcursor.so.1",
        "libXinerama.so.1",
        "libXft.so.2",
        "libGLU.so.1",
    ] {
        unsafe {
            if let Ok(l) = Library::open(Some(so), RTLD_NOW | RTLD_GLOBAL) {
                std::mem::forget(l);
                continue;
            }
            if let Ok(l) = Library::open(Some(dir.join(so)), RTLD_NOW | RTLD_GLOBAL) {
                std::mem::forget(l);
            }
        }
    }
}

fn open_gmsh() -> Result<Library> {
    preload_fallbacks();
    let mut cands: Vec<PathBuf> = vec![];
    if let Ok(p) = std::env::var("MODELTHUMB_GMSH") {
        cands.push(p.into());
    }
    if let Some(d) = lib_dir() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            let mut v: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("libgmsh.so"))
                })
                .collect();
            v.sort();
            cands.extend(v);
        }
    }
    cands.push("libgmsh.so".into());
    cands.push("libgmsh.so.4".into());
    let mut last = String::new();
    for c in cands {
        match unsafe { Library::open(Some(&c), RTLD_NOW | RTLD_GLOBAL) } {
            Ok(l) => return Ok(l),
            Err(e) => last = e.to_string(),
        }
    }
    Err(anyhow!(
        "STEP support needs libgmsh (bundled in the AppImage): {last}"
    ))
}

type FnInit = unsafe extern "C" fn(c_int, *mut *mut c_char, c_int, c_int, *mut c_int);
type FnIerr = unsafe extern "C" fn(*mut c_int);
type FnOpen = unsafe extern "C" fn(*const c_char, *mut c_int);
type FnSetNum = unsafe extern "C" fn(*const c_char, f64, *mut c_int);
type FnBBox = unsafe extern "C" fn(
    c_int,
    c_int,
    *mut f64,
    *mut f64,
    *mut f64,
    *mut f64,
    *mut f64,
    *mut f64,
    *mut c_int,
);
type FnGen = unsafe extern "C" fn(c_int, *mut c_int);
type FnFree = unsafe extern "C" fn(*mut c_void);
type FnNodes = unsafe extern "C" fn(
    *mut *mut usize,
    *mut usize,
    *mut *mut f64,
    *mut usize,
    *mut *mut f64,
    *mut usize,
    c_int,
    c_int,
    c_int,
    c_int,
    *mut c_int,
);
type FnElems = unsafe extern "C" fn(
    *mut *mut c_int,
    *mut usize,
    *mut *mut *mut usize,
    *mut *mut usize,
    *mut usize,
    *mut *mut *mut usize,
    *mut *mut usize,
    *mut usize,
    c_int,
    c_int,
    *mut c_int,
);

struct Gmsh<'a> {
    set: Symbol<FnSetNum>,
    fin: Symbol<FnIerr>,
    free: Symbol<FnFree>,
    _l: &'a Library,
}

impl Gmsh<'_> {
    fn opt(&self, k: &str, v: f64) {
        let k = CString::new(k).unwrap();
        let mut e = 0;
        unsafe { (self.set)(k.as_ptr(), v, &mut e) };
    }
}

impl Drop for Gmsh<'_> {
    fn drop(&mut self) {
        let mut e = 0;
        unsafe { (self.fin)(&mut e) };
    }
}

fn ck(e: c_int, what: &str) -> Result<()> {
    if e != 0 {
        bail!("gmsh: {what} failed ({e})");
    }
    Ok(())
}

pub fn load(path: &Path, quality: f64) -> Result<Mesh> {
    let lib = open_gmsh()?;
    let quality = quality.clamp(0.1, 10.0);
    unsafe {
        let init: Symbol<FnInit> = lib.get(b"gmshInitialize")?;
        let open: Symbol<FnOpen> = lib.get(b"gmshOpen")?;
        let bbox: Symbol<FnBBox> = lib.get(b"gmshModelGetBoundingBox")?;
        let gen: Symbol<FnGen> = lib.get(b"gmshModelMeshGenerate")?;
        let nodes: Symbol<FnNodes> = lib.get(b"gmshModelMeshGetNodes")?;
        let elems: Symbol<FnElems> = lib.get(b"gmshModelMeshGetElements")?;
        let mut e: c_int = 0;
        let mut argv0 = CString::new("modelthumb").unwrap().into_raw();
        init(1, &mut argv0, 0, 0, &mut e);
        drop(CString::from_raw(argv0));
        ck(e, "initialize")?;
        let g = Gmsh {
            set: lib.get(b"gmshOptionSetNumber")?,
            fin: lib.get(b"gmshFinalize")?,
            free: lib.get(b"gmshFree")?,
            _l: &lib,
        };
        g.opt("General.Terminal", 0.0);
        g.opt("General.Verbosity", 0.0);
        g.opt("Geometry.OCCImportLabels", 0.0);
        g.opt(
            "General.NumThreads",
            std::thread::available_parallelism()
                .map(|n| n.get().min(4))
                .unwrap_or(1) as f64,
        );

        let cpath = CString::new(path.to_string_lossy().as_bytes())?;
        open(cpath.as_ptr(), &mut e);
        ck(e, "open")?;
        let (mut x0, mut y0, mut z0, mut x1, mut y1, mut z1) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        bbox(
            -1, -1, &mut x0, &mut y0, &mut z0, &mut x1, &mut y1, &mut z1, &mut e,
        );
        let diag = ((x1 - x0).powi(2) + (y1 - y0).powi(2) + (z1 - z0).powi(2)).sqrt();
        let diag = if diag.is_finite() && diag > 0.0 {
            diag
        } else {
            1.0
        };
        // coarse but visually smooth surface tessellation: only the look matters
        g.opt("Mesh.MeshSizeMax", diag / (25.0 * quality));
        g.opt("Mesh.MeshSizeMin", diag / (400.0 * quality));
        g.opt("Mesh.MeshSizeFromCurvature", (10.0 * quality).round());
        g.opt("Mesh.MeshSizeExtendFromBoundary", 1.0);
        g.opt("Mesh.Algorithm", 1.0);
        gen(2, &mut e);
        if e != 0 {
            g.opt("Mesh.Algorithm", 6.0);
            e = 0;
            gen(2, &mut e);
            ck(e, "meshing")?;
        }

        // nodes
        let (mut tags, mut ntags, mut coord, mut ncoord, mut par, mut npar) = (
            std::ptr::null_mut(),
            0usize,
            std::ptr::null_mut(),
            0usize,
            std::ptr::null_mut(),
            0usize,
        );
        nodes(
            &mut tags,
            &mut ntags,
            &mut coord,
            &mut ncoord,
            &mut par,
            &mut npar,
            -1,
            -1,
            0,
            0,
            &mut e,
        );
        ck(e, "getNodes")?;
        let tags_s = std::slice::from_raw_parts(tags, ntags);
        let coord_s = std::slice::from_raw_parts(coord, ncoord);
        let maxtag = tags_s.iter().copied().max().unwrap_or(0);
        let mut idx = vec![u32::MAX; maxtag + 1];
        let mut mesh = Mesh::default();
        for (i, &t) in tags_s.iter().enumerate() {
            idx[t] = i as u32;
            mesh.verts.push([
                coord_s[3 * i] as f32,
                coord_s[3 * i + 1] as f32,
                coord_s[3 * i + 2] as f32,
            ]);
        }
        for p in [
            tags as *mut c_void,
            coord as *mut c_void,
            par as *mut c_void,
        ] {
            if !p.is_null() {
                (g.free)(p);
            }
        }

        // 2D elements
        let (mut types, mut ntypes) = (std::ptr::null_mut(), 0usize);
        let (mut etags, mut etags_n, mut etags_nn) =
            (std::ptr::null_mut(), std::ptr::null_mut(), 0usize);
        let (mut ntag, mut ntag_n, mut ntag_nn) =
            (std::ptr::null_mut(), std::ptr::null_mut(), 0usize);
        elems(
            &mut types,
            &mut ntypes,
            &mut etags,
            &mut etags_n,
            &mut etags_nn,
            &mut ntag,
            &mut ntag_n,
            &mut ntag_nn,
            2,
            -1,
            &mut e,
        );
        ck(e, "getElements")?;
        let types_s = std::slice::from_raw_parts(types, ntypes);
        let get = |t: usize| idx.get(t).copied().unwrap_or(u32::MAX);
        for (k, &etype) in types_s.iter().enumerate() {
            let n = *ntag_n.add(k);
            let nt = std::slice::from_raw_parts(*ntag.add(k), n);
            match etype {
                2 => mesh.faces.extend(
                    nt.as_chunks::<3>()
                        .0
                        .iter()
                        .map(|c| [get(c[0]), get(c[1]), get(c[2])]),
                ),
                3 => {
                    for c in nt.as_chunks::<4>().0 {
                        mesh.faces.push([get(c[0]), get(c[1]), get(c[2])]);
                        mesh.faces.push([get(c[0]), get(c[2]), get(c[3])]);
                    }
                }
                _ => {}
            }
            (g.free)(*ntag.add(k) as *mut c_void);
            (g.free)(*etags.add(k) as *mut c_void);
        }
        for p in [
            types as *mut c_void,
            etags as *mut c_void,
            etags_n as *mut c_void,
            ntag as *mut c_void,
            ntag_n as *mut c_void,
        ] {
            if !p.is_null() {
                (g.free)(p);
            }
        }
        mesh.faces.retain(|f| f.iter().all(|&i| i != u32::MAX));
        drop(g);
        std::mem::forget(lib); // unloading OCC at exit can crash; the process ends anyway
        if mesh.is_empty() {
            bail!("CAD file contains no surfaces");
        }
        Ok(mesh)
    }
}
