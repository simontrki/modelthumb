# Third-party components

modelthumb itself is licensed under the **GNU General Public License v3.0 or later** (see `LICENSE`).

The AppImage release additionally bundles the following components. Their license texts are shipped
inside the AppImage under `usr/share/licenses/modelthumb/`.

## Gmsh (libgmsh) — STEP / IGES / BREP import

- Version: 4.15.2 (binary taken from the official `gmsh` wheel on PyPI)
- License: GNU GPL v2 or later, with an exception permitting combination with OpenCASCADE,
  Netgen, METIS and ParaView (see `gmsh-LICENSE.txt`)
- Homepage: https://gmsh.info
- Source code: https://gmsh.info/src/gmsh-4.15.2-source.tgz

libgmsh statically contains further libraries; they are listed with their licenses in Gmsh's
`CREDITS.txt`. The most important one is:

### Open CASCADE Technology (OCCT)

- Version: 7.8.1
- License: GNU LGPL v2.1 with the Open CASCADE exception
- Homepage: https://dev.opencascade.org
- Source code: https://github.com/Open-Cascade-SAS/OCCT/releases/tag/V7_8_1

To meet the source-code obligations of the GPL, every GitHub release of modelthumb attaches the
exact Gmsh and OCCT source archives used for that release.

## Fallback libraries (loaded only when the system does not provide them)

Taken from Ubuntu 20.04 packages. Their `copyright` files are shipped in the AppImage.

| Library | Package | License |
|---|---|---|
| libGLU.so.1 | libglu1-mesa 9.0.1 | SGI Free Software License B 2.0 (MIT-style) |
| libXcursor.so.1 | libxcursor1 1.2.0 | MIT/X11 |
| libXinerama.so.1 | libxinerama1 1.1.4 | MIT/X11 |
| libXft.so.2 | libxft2 2.3.3 | MIT/X11 |
| libXfixes.so.3 | libxfixes3 5.0.3 | MIT/X11 |
| libXrender.so.1 | libxrender1 0.9.10 | MIT/X11 |

## Rust crates (statically linked)

All crates in `Cargo.lock` are under licenses compatible with the GPL v3 (MIT, Apache-2.0,
BSD, ISC, Zlib, Unicode-3.0, Unlicense). A full list can be generated with:

    cargo install cargo-about && cargo about generate about.hbs
