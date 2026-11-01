# 3D-Thumbnailer (Rust)

Zeigt in Linux-Dateimanagern Vorschaubilder für 3D-Dateien. Entwickelt für **Pop!_OS mit COSMIC**,
funktioniert aber auch mit GNOME Files (Nautilus), Nemo, Caja, Thunar und anderen Dateimanagern,
die den freedesktop-Thumbnailer-Standard (`*.thumbnailer`) unterstützen.

| Format | Endungen | Hinweis |
|---|---|---|
| STL | `.stl` | binär + ASCII |
| 3MF | `.3mf`, `.gcode.3mf` | ein eingebettetes Slicer-Vorschaubild wird direkt verwendet, sonst wird das Mesh gerendert (auch Mehrteil-Projekte aus Bambu/Prusa) |
| STEP | `.step`, `.stp` | über OpenCASCADE (mitgelieferte `libgmsh`) |
| IGES, BREP | `.iges`, `.igs`, `.brep` | wie STEP |
| OBJ, PLY, OFF | `.obj`, `.ply`, `.off` | |
| glTF | `.glb`, `.gltf` | |

Gerendert wird komplett auf der CPU (eigener Rasterizer in Rust). Es braucht also weder GPU noch
Display-Server, und es läuft auch in Sandboxen. Typische Zeiten für ein 256-px-Bild liegen zwischen
0,05 und 0,4 s.

## Installation

```bash
chmod +x 3D-Thumbnailer-x86_64.AppImage
./3D-Thumbnailer-x86_64.AppImage install            # für den aktuellen Benutzer (COSMIC, Nemo, Thunar, Caja …)
sudo ./3D-Thumbnailer-x86_64.AppImage install --system   # systemweit, nötig für GNOME/Nautilus
```

Danach den Dateimanager neu starten (COSMIC: `pkill cosmic-files`). Wird die AppImage ohne Terminal
per Doppelklick gestartet, fragt ein Dialog nach der Installation. Dafür muss zenity oder kdialog
installiert sein.

Die Installation kopiert die Laufzeit nach `~/.local/share/3dthumb` bzw. `/usr/local/lib/3dthumb`.
Die AppImage-Datei kann danach gelöscht werden. Thumbnailer werden in kurzlebigen Prozessen
gestartet, und ein FUSE-Mount für jedes Vorschaubild wäre langsam und fehleranfällig.

Deinstallieren: `3dthumb uninstall` bzw. `sudo 3dthumb uninstall --system`.

## Vorschau in die Datei einbetten

* **3MF**: Ja. `3dthumb embed datei.3mf` (oder `-r ordner/`) schreibt `Metadata/thumbnail.png`
  inklusive Package-Relationship. Das ist das Standardfeld, das Windows Explorer, Slicer und dieser
  Thumbnailer lesen. Vorhandene Slicer-Bilder bleiben erhalten, außer man gibt `--force` an.
* **STL, STEP, OBJ, PLY**: Nicht möglich. Diese Formate haben kein Feld für Vorschaubilder, das ein
  Dateimanager liest. Unter Linux liegen Vorschauen im gemeinsamen Cache `~/.cache/thumbnails`, den alle
  Dateimanager nutzen. Mit `3dthumb cache -r ~/3D-Modelle` lässt er sich für ganze Ordner im Voraus
  füllen, sodass die Vorschauen sofort da sind.

## Weitere Befehle

```bash
3dthumb render teil.step -s 1024 -o teil.png --background "#ffffff" --color "#e07020"
3dthumb render teil.stl --azimuth 45 --elevation 20
3dthumb formats
```

## Bauen

```bash
pip install cargo-zigbuild ziglang     # optional, ergibt ein Binary für glibc >= 2.17
./build-appimage.sh                    # -> dist/3D-Thumbnailer-x86_64.AppImage
```

Das Skript lädt `libgmsh` (OpenCASCADE-STEP-Import) von PyPI sowie kleine X11/GLU-Ersatzbibliotheken.
Diese werden nur geladen, wenn sie auf dem System fehlen. Anschließend packt `appimagetool` alles zusammen.
