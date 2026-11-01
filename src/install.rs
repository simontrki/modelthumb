//! Register / unregister the thumbnailer (per user or system-wide).

use crate::output::{cache_dir, home};
use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const NAME: &str = "3dthumb";

/// (mime type, description, globs, aliases used by various distributions)
const MIME_TYPES: &[(&str, &str, &[&str], &[&str])] = &[
    ("model/stl", "STL 3D model", &["*.stl"],
     &["model/x.stl-binary", "model/x.stl-ascii", "application/sla", "application/vnd.ms-pki.stl", "model/x-stl", "application/x-stl"]),
    ("model/3mf", "3MF 3D model", &["*.3mf"],
     &["application/vnd.ms-3mfdocument", "application/vnd.ms-package.3dmanufacturing-3dmodel+xml"]),
    ("model/step", "STEP CAD model", &["*.step", "*.stp", "*.p21"],
     &["application/step", "application/x-step", "model/x-step", "application/stp"]),
    ("model/iges", "IGES CAD model", &["*.iges", "*.igs"], &["application/iges", "application/x-iges", "model/x-iges"]),
    ("model/x-brep", "OpenCASCADE BREP model", &["*.brep", "*.brp"], &[]),
    ("model/obj", "Wavefront OBJ model", &["*.obj"], &["application/x-wavefront-obj", "model/x-obj"]),
    ("model/x-ply", "PLY 3D model", &["*.ply"], &["application/x-ply", "model/ply"]),
    ("model/gltf-binary", "glTF binary model", &["*.glb"], &[]),
    ("model/gltf+json", "glTF model", &["*.gltf"], &[]),
    ("model/x-off", "OFF 3D model", &["*.off"], &["application/x-off"]),
];

pub fn mime_xml() -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
    );
    for (t, c, globs, aliases) in MIME_TYPES {
        s += &format!("  <mime-type type=\"{t}\">\n    <comment>{c}</comment>\n    <generic-icon name=\"application-x-object\"/>\n");
        for g in *globs {
            s += &format!("    <glob pattern=\"{g}\" weight=\"80\"/>\n");
        }
        for a in *aliases {
            s += &format!("    <alias type=\"{a}\"/>\n");
        }
        s += "  </mime-type>\n";
    }
    s + "</mime-info>\n"
}

pub fn thumbnailer_entry(bin: &Path) -> String {
    let mut mimes: Vec<&str> = vec![];
    for (t, _, _, aliases) in MIME_TYPES {
        mimes.push(t);
        mimes.extend(aliases.iter());
    }
    format!(
        "[Thumbnailer Entry]\nTryExec={b}\nExec={b} thumbnail -s %s %i %o\nMimeType={m};\n",
        b = bin.display(),
        m = mimes.join(";")
    )
}

struct Paths {
    app: PathBuf,
    bin: PathBuf,
    thumbnailer: PathBuf,
    mimedir: PathBuf,
}

fn paths(system: bool) -> Paths {
    if system {
        return Paths {
            app: format!("/usr/local/lib/{NAME}").into(),
            bin: format!("/usr/local/bin/{NAME}").into(),
            thumbnailer: format!("/usr/share/thumbnailers/{NAME}.thumbnailer").into(),
            mimedir: "/usr/share/mime".into(),
        };
    }
    let data = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"));
    Paths {
        app: data.join(NAME).join("app"),
        bin: home().join(".local/bin").join(NAME),
        thumbnailer: data.join("thumbnailers").join(format!("{NAME}.thumbnailer")),
        mimedir: data.join("mime"),
    }
}

fn is_root() -> bool {
    // effective uid without libc: /proc/self/status "Uid: real effective ..."
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("Uid:")).and_then(|l| l.split_whitespace().nth(2)).map(|u| u == "0"))
        .unwrap_or(false)
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let t = e.file_type()?;
        let to = dst.join(e.file_name());
        if t.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(e.path())?, &to)?;
        } else if t.is_dir() {
            copy_tree(&e.path(), &to)?;
        } else {
            fs::copy(e.path(), &to).with_context(|| format!("copy {}", e.path().display()))?;
        }
    }
    Ok(())
}

fn update_mime(dir: &Path) {
    match Command::new("update-mime-database").arg(dir).status() {
        Ok(_) => {}
        Err(_) => eprintln!("  ! update-mime-database not found (install shared-mime-info)"),
    }
}

pub fn install(system: bool) -> Result<()> {
    let appdir = std::env::var_os("APPDIR").map(PathBuf::from).filter(|d| d.join("AppRun").exists());
    let Some(appdir) = appdir else {
        bail!("install must be run from the AppImage (APPDIR not set)");
    };
    if system && !is_root() {
        bail!("system-wide install needs root:  sudo ./3D-Thumbnailer-x86_64.AppImage install --system");
    }
    let p = paths(system);
    println!("Installing {NAME} ({})", if system { "system-wide" } else { "for the current user" });
    if p.app.exists() {
        fs::remove_dir_all(&p.app)?;
    }
    copy_tree(&appdir, &p.app)?;
    println!("  runtime      -> {}", p.app.display());

    fs::create_dir_all(p.bin.parent().unwrap())?;
    let _ = fs::remove_file(&p.bin);
    fs::write(&p.bin, format!("#!/bin/sh\nexec \"{}/usr/bin/3dthumb\" \"$@\"\n", p.app.display()))?;
    fs::set_permissions(&p.bin, fs::Permissions::from_mode(0o755))?;
    println!("  launcher     -> {}", p.bin.display());

    fs::create_dir_all(p.thumbnailer.parent().unwrap())?;
    fs::write(&p.thumbnailer, thumbnailer_entry(&p.bin))?;
    println!("  thumbnailer  -> {}", p.thumbnailer.display());

    let pk = p.mimedir.join("packages").join(format!("{NAME}.xml"));
    fs::create_dir_all(pk.parent().unwrap())?;
    fs::write(&pk, mime_xml())?;
    println!("  mime types   -> {}", pk.display());
    update_mime(&p.mimedir);

    if !system {
        // forget earlier "could not create thumbnail" markers so files get retried
        if let Ok(rd) = fs::read_dir(cache_dir().join("fail")) {
            for e in rd.flatten() {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
    println!("\nDone. Restart the file manager to see 3D previews:");
    println!("  COSMIC: pkill cosmic-files   GNOME: nautilus -q   Thunar: thunar -q   Nemo: nemo -q   Caja: caja -q");
    if !system {
        println!("\nNote: GNOME Files (Nautilus) runs thumbnailers in a sandbox without access to your home");
        println!("directory. If Nautilus shows no previews, install system-wide instead:");
        println!("  sudo ./3D-Thumbnailer-x86_64.AppImage install --system");
    }
    Ok(())
}

pub fn uninstall(system: bool) -> Result<()> {
    if system && !is_root() {
        bail!("needs root: sudo 3dthumb uninstall --system");
    }
    let p = paths(system);
    for f in [&p.bin, &p.thumbnailer] {
        if fs::symlink_metadata(f).is_ok() {
            fs::remove_file(f)?;
            println!("  removed {}", f.display());
        }
    }
    let pk = p.mimedir.join("packages").join(format!("{NAME}.xml"));
    if pk.exists() {
        fs::remove_file(&pk)?;
        println!("  removed {}", pk.display());
        update_mime(&p.mimedir);
    }
    if p.app.exists() {
        fs::remove_dir_all(&p.app)?;
        println!("  removed {}", p.app.display());
        if !system {
            if let Some(parent) = p.app.parent() {
                let _ = fs::remove_dir(parent); // ~/.local/share/3dthumb, only if empty
            }
        }
    }
    println!("Uninstalled.");
    Ok(())
}
