mod cad;
mod install;
mod mesh;
mod output;
mod render;
mod threemf;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use render::{Image, Options};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "3dthumb", version, about = "3D model previews (STL, 3MF, STEP, IGES, OBJ, PLY, glTF) for Linux file managers")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Args, Clone)]
struct View {
    /// output size in pixels
    #[arg(short, long, default_value_t = 512)]
    size: u32,
    /// model color #RRGGBB
    #[arg(long, value_parser = parse_color)]
    color: Option<[u8; 3]>,
    /// background color #RRGGBB (default: transparent)
    #[arg(long, value_parser = parse_color)]
    background: Option<[u8; 3]>,
    #[arg(long, default_value_t = -35.0, allow_hyphen_values = true)]
    azimuth: f64,
    #[arg(long, default_value_t = 28.0, allow_hyphen_values = true)]
    elevation: f64,
    /// STEP/IGES tessellation detail (default 1.0)
    #[arg(long, default_value_t = 1.0)]
    quality: f64,
}

impl Default for View {
    fn default() -> Self {
        View { size: 256, color: None, background: None, azimuth: -35.0, elevation: 28.0, quality: 1.0 }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// (called by file managers) write a PNG preview of INPUT to OUTPUT
    Thumbnail {
        #[arg(short, long, default_value_t = 256)]
        size: u32,
        input: String,
        output: PathBuf,
    },
    /// render a model to a PNG file
    Render {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[command(flatten)]
        view: View,
    },
    /// store a preview inside 3MF files (Metadata/thumbnail.png, read by Windows Explorer, slicers, this tool)
    Embed {
        files: Vec<PathBuf>,
        #[arg(short, long)]
        recursive: bool,
        /// replace an existing thumbnail
        #[arg(short, long)]
        force: bool,
        #[command(flatten)]
        view: View,
    },
    /// pre-generate previews for files/folders into ~/.cache/thumbnails
    Cache {
        paths: Vec<PathBuf>,
        #[arg(short, long)]
        recursive: bool,
        #[arg(short, long)]
        force: bool,
        /// comma separated: normal,large,x-large,xx-large
        #[arg(long, default_value = "normal,large,x-large")]
        sizes: String,
    },
    /// register as thumbnailer for the current user (or --system, with sudo)
    Install {
        #[arg(long)]
        system: bool,
    },
    /// remove the thumbnailer
    Uninstall {
        #[arg(long)]
        system: bool,
    },
    /// list supported formats
    Formats,
}

fn parse_color(s: &str) -> Result<[u8; 3], String> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return Err("expected #RRGGBB".into());
    }
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string());
    Ok([p(0)?, p(2)?, p(4)?])
}

fn local_path(s: &str) -> PathBuf {
    match s.strip_prefix("file://") {
        Some(rest) => {
            let rest = rest.find('/').map(|i| &rest[i..]).unwrap_or(rest); // drop host part
            PathBuf::from(std::ffi::OsString::from(
                <std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(
                    percent_encoding::percent_decode_str(rest).collect(),
                ),
            ))
        }
        None => PathBuf::from(s),
    }
}

/// Build the preview image: embedded 3MF thumbnail if present (and allowed), otherwise render.
fn make_image(path: &Path, v: &View, prefer_embedded: bool) -> Result<Image> {
    if prefer_embedded && mesh::sniff(path)? == mesh::Kind::ThreeMf {
        if let Some(img) = threemf::thumbnail(path).and_then(|d| output::decode_thumb(&d, v.size)) {
            return Ok(img);
        }
    }
    let m = mesh::load(path, v.quality)?;
    let mut o = Options { size: v.size, ..Default::default() };
    if let Some(c) = v.color {
        o.color = [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0];
    }
    o.background = v.background.map(|c| [c[0], c[1], c[2], 255]);
    o.azimuth = v.azimuth;
    o.elevation = v.elevation;
    Ok(render::render(&m, &o))
}

fn iter_files(paths: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    fn ok(p: &Path) -> bool {
        let n = p.to_string_lossy().to_lowercase();
        mesh::EXTENSIONS.iter().any(|e| n.ends_with(&format!(".{e}")))
    }
    fn walk(d: &Path, rec: bool, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        for p in v {
            if p.is_dir() {
                if rec {
                    walk(&p, rec, out);
                }
            } else if ok(&p) {
                out.push(p);
            }
        }
    }
    let mut out = vec![];
    for p in paths {
        if p.is_dir() {
            walk(p, recursive, &mut out);
        } else {
            out.push(p.clone());
        }
    }
    out
}

fn gui_first_run() -> Result<()> {
    let text = "3D-Thumbnailer zeigt Vorschaubilder für STL, 3MF, STEP, IGES, OBJ, PLY und glTF im Dateimanager.\n\n\
                Für den aktuellen Benutzer installieren?";
    let ask = |cmd: &str, args: &[&str]| Command::new(cmd).args(args).status().map(|s| s.success()).ok();
    let ok = ask("zenity", &["--question", "--title=3D-Thumbnailer", &format!("--text={text}")])
        .or_else(|| ask("kdialog", &["--title", "3D-Thumbnailer", "--yesno", text]));
    if ok == Some(true) {
        let r = install::install(false);
        let msg = match &r {
            Ok(_) => "Installiert. Bitte den Dateimanager neu starten.".to_string(),
            Err(e) => format!("Installation fehlgeschlagen: {e}"),
        };
        let _ = Command::new("notify-send").args(["3D-Thumbnailer", &msg]).status();
        r?;
    }
    Ok(())
}

fn run() -> Result<i32> {
    if std::env::args().len() == 1 && !std::io::stdin().is_terminal() && !std::io::stdout().is_terminal() {
        gui_first_run()?;
        return Ok(0);
    }
    let cli = Cli::parse();
    let Some(cmd) = cli.cmd else {
        use clap::CommandFactory;
        Cli::command().print_help()?;
        return Ok(0);
    };
    match cmd {
        Cmd::Thumbnail { size, input, output } => {
            let v = View { size, ..Default::default() };
            let img = make_image(&local_path(&input), &v, true)?;
            output::write_atomic(&output, &output::encode_png(&img, &[])?, None)?;
        }
        Cmd::Render { input, output, view } => {
            let out = output.unwrap_or_else(|| PathBuf::from(format!("{}.png", input.file_stem().unwrap_or_default().to_string_lossy())));
            let t = Instant::now();
            let img = make_image(&input, &view, false)?;
            output::write_atomic(&out, &output::encode_png(&img, &[])?, None)?;
            println!("{}  ({}x{}, {:.2}s)", out.display(), img.w, img.h, t.elapsed().as_secs_f64());
        }
        Cmd::Embed { files, recursive, force, view } => {
            let mut rc = 0;
            for p in iter_files(&files, recursive) {
                match mesh::sniff(&p) {
                    Ok(mesh::Kind::ThreeMf) => {}
                    _ => {
                        println!("skip   {}: only 3MF has a thumbnail field - use '3dthumb cache' instead", p.display());
                        continue;
                    }
                }
                if !force && threemf::thumbnail(&p).is_some() {
                    println!("keep   {}: already has a thumbnail (--force replaces it)", p.display());
                    continue;
                }
                let r = make_image(&p, &view, false).and_then(|img| output::embed_3mf(&p, &output::encode_png(&img, &[])?));
                match r {
                    Ok(()) => println!("embed  {}", p.display()),
                    Err(e) => {
                        eprintln!("error  {}: {e:#}", p.display());
                        rc = 1;
                    }
                }
            }
            return Ok(rc);
        }
        Cmd::Cache { paths, recursive, force, sizes } => {
            let mut flavors = vec![];
            for s in sizes.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                match output::CACHE_SIZES.iter().find(|(n, _)| *n == s) {
                    Some(f) => flavors.push(*f),
                    None => bail!("unknown size '{s}' (normal, large, x-large, xx-large)"),
                }
            }
            let big = flavors.iter().map(|f| f.1).max().unwrap_or(256);
            let (mut rc, mut n) = (0, 0);
            for p in iter_files(&paths, recursive) {
                if !force && flavors.iter().all(|(f, _)| output::cache_is_fresh(&p, f)) {
                    continue;
                }
                let v = View { size: big, ..Default::default() };
                match make_image(&p, &v, true) {
                    Ok(img) => {
                        for (f, px) in &flavors {
                            output::write_cache(&p, &img, f, *px)?;
                        }
                        n += 1;
                        println!("cached {}", p.display());
                    }
                    Err(e) => {
                        eprintln!("error  {}: {e:#}", p.display());
                        rc = 1;
                    }
                }
            }
            println!("{n} preview(s) written to {}", output::cache_dir().display());
            return Ok(rc);
        }
        Cmd::Install { system } => install::install(system)?,
        Cmd::Uninstall { system } => install::uninstall(system)?,
        Cmd::Formats => {
            println!("STL (binary/ASCII), 3MF, STEP/STP, IGES/IGS, BREP, OBJ, PLY, GLB/glTF, OFF");
            println!("3MF files that already contain a slicer thumbnail show that image.");
        }
    }
    Ok(0)
}

fn main() {
    let code = match run() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("3dthumb: {e:#}");
            1
        }
    };
    std::process::exit(code);
}
