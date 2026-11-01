//! PNG writing, freedesktop thumbnail cache and 3MF thumbnail embedding.

use crate::render::Image;
use crate::threemf::THUMB_REL;
use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use percent_encoding::{AsciiSet, CONTROLS};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub fn encode_png(img: &Image, text: &[(&str, String)]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, img.w, img.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Best);
        for (k, v) in text {
            enc.add_text_chunk(k.to_string(), v.clone())?;
        }
        let mut w = enc.write_header()?;
        w.write_image_data(&img.px)?;
    }
    Ok(buf)
}

/// Atomically write bytes (temp file + rename), so file managers never read half a PNG.
pub fn write_atomic(path: &Path, data: &[u8], mode: Option<u32>) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    {
        let mut f = BufWriter::new(
            File::create(&tmp).with_context(|| format!("cannot write {}", tmp.display()))?,
        );
        f.write_all(data)?;
        f.flush()?;
    }
    if let Some(m) = mode {
        fs::set_permissions(&tmp, fs::Permissions::from_mode(m))?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// Decode an embedded (PNG/JPEG) thumbnail and fit it into size×size.
pub fn decode_thumb(data: &[u8], size: u32) -> Option<Image> {
    let im = image::load_from_memory(data).ok()?;
    let im = if im.width() > size || im.height() > size {
        im.resize(size, size, image::imageops::FilterType::Lanczos3)
    } else {
        im
    };
    let rgba = im.to_rgba8();
    Some(Image {
        w: rgba.width(),
        h: rgba.height(),
        px: rgba.into_raw(),
    })
}

pub fn shrink(img: &Image, size: u32) -> Image {
    if img.w <= size && img.h <= size {
        return Image {
            w: img.w,
            h: img.h,
            px: img.px.clone(),
        };
    }
    let buf = image::RgbaImage::from_raw(img.w, img.h, img.px.clone()).unwrap();
    let r = image::DynamicImage::ImageRgba8(buf)
        .resize(size, size, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    Image {
        w: r.width(),
        h: r.height(),
        px: r.into_raw(),
    }
}

// ---------------------------------------------------------------- freedesktop thumbnail cache
pub const CACHE_SIZES: &[(&str, u32)] = &[
    ("normal", 128),
    ("large", 256),
    ("x-large", 512),
    ("xx-large", 1024),
];

// characters GLib's g_filename_to_uri() leaves unescaped: unreserved + !$&'()*+,;=:@/
const URI_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

pub fn file_uri(path: &Path) -> String {
    let abs = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    use std::os::unix::ffi::OsStrExt;
    let bytes = abs.as_os_str().as_bytes();
    let s: String = percent_encoding::percent_encode(bytes, URI_SET).collect();
    format!("file://{s}")
}

pub fn cache_dir() -> PathBuf {
    match std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d).join("thumbnails"),
        None => home().join(".cache/thumbnails"),
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/root".into())
}

pub fn cache_path(path: &Path, flavor: &str) -> PathBuf {
    let h = Md5::digest(file_uri(path).as_bytes());
    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
    cache_dir().join(flavor).join(format!("{hex}.png"))
}

pub fn cache_is_fresh(path: &Path, flavor: &str) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Ok(f) = File::open(cache_path(path, flavor)) else {
        return false;
    };
    let dec = png::Decoder::new(BufReader::new(f));
    let Ok(r) = dec.read_info() else { return false };
    r.info()
        .uncompressed_latin1_text
        .iter()
        .any(|t| t.keyword == "Thumb::MTime" && t.text == meta.mtime().to_string())
}

pub fn write_cache(path: &Path, img: &Image, flavor: &str, px: u32) -> Result<PathBuf> {
    let meta = fs::metadata(path)?;
    let im = shrink(img, px);
    let png = encode_png(
        &im,
        &[
            ("Thumb::URI", file_uri(path)),
            ("Thumb::MTime", meta.mtime().to_string()),
            ("Thumb::Size", meta.len().to_string()),
            ("Software", "modelthumb".into()),
        ],
    )?;
    let cp = cache_path(path, flavor);
    let dir = cp.parent().unwrap();
    fs::create_dir_all(dir)?;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    write_atomic(&cp, &png, Some(0o600))?;
    Ok(cp)
}

// ---------------------------------------------------------------- embed thumbnail into 3MF
pub const THUMB_PATH: &str = "Metadata/thumbnail.png";

pub fn embed_3mf(path: &Path, png_bytes: &[u8]) -> Result<()> {
    let mut z = zip::ZipArchive::new(BufReader::new(File::open(path)?))?;
    let read = |z: &mut zip::ZipArchive<BufReader<File>>, n: &str| -> Option<String> {
        let mut f = z.by_name(n).ok()?;
        let mut s = String::new();
        f.read_to_string(&mut s).ok()?;
        Some(s)
    };
    let rels_xml = read(&mut z, "_rels/.rels").unwrap_or_else(|| {
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>".into()
    });
    let ctypes = read(&mut z, "[Content_Types].xml");
    if !rels_xml.contains("</Relationships>") {
        bail!("unexpected _rels/.rels format");
    }
    // drop existing thumbnail relationships
    let mut new_rels = String::new();
    let mut rest = rels_xml.as_str();
    while let Some(i) = rest.find("<Relationship ") {
        let end = rest[i..]
            .find("/>")
            .map(|e| i + e + 2)
            .unwrap_or(rest.len());
        new_rels.push_str(&rest[..i]);
        if !rest[i..end].contains(THUMB_REL) {
            new_rels.push_str(&rest[i..end]);
        }
        rest = &rest[end..];
    }
    new_rels.push_str(rest);
    let mut rid = 0;
    while new_rels.contains(&format!("Id=\"rel-thumb{rid}\"")) {
        rid += 1;
    }
    let new_rels = new_rels.replacen(
        "</Relationships>",
        &format!("<Relationship Target=\"/{THUMB_PATH}\" Id=\"rel-thumb{rid}\" Type=\"{THUMB_REL}\"/></Relationships>"),
        1,
    );
    let ctypes = ctypes.map(|c| {
        if c.to_ascii_lowercase().contains("extension=\"png\"") {
            c
        } else {
            c.replacen(
                "</Types>",
                "<Default Extension=\"png\" ContentType=\"image/png\"/></Types>",
                1,
            )
        }
    });

    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let tmp = dir.join(format!(
        ".{}.modelthumb.tmp",
        path.file_name().unwrap().to_string_lossy()
    ));
    let res = (|| -> Result<()> {
        let mut w = zip::ZipWriter::new(BufWriter::new(File::create(&tmp)?));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        if let Some(c) = &ctypes {
            w.start_file("[Content_Types].xml", opts)?;
            w.write_all(c.as_bytes())?;
        }
        w.start_file("_rels/.rels", opts)?;
        w.write_all(new_rels.as_bytes())?;
        for i in 0..z.len() {
            let f = z.by_index_raw(i)?;
            let name = f.name().to_string();
            if name == THUMB_PATH || name == "_rels/.rels" || name == "[Content_Types].xml" {
                continue;
            }
            w.raw_copy_file(f)?;
        }
        w.start_file(
            THUMB_PATH,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )?;
        w.write_all(png_bytes)?;
        w.finish()?.flush()?;
        Ok(())
    })();
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    let meta = fs::metadata(path)?;
    fs::set_permissions(&tmp, meta.permissions())?;
    fs::rename(&tmp, path)?;
    Ok(())
}
