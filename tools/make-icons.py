#!/usr/bin/env python3
"""Sealwax app icons from the vector master (the fork owner's choice, 2026-10-20: a modern
mail icon, a green envelope on a navy tile; ADR-010 amendment).

    python3 tools/make-icons.py packaging/icons

reads packaging/icons/source/sealwax-icon.svg (512 px viewBox) and writes
hicolor/{16,24,32,48,64,128,256,512}x…/apps/<id>.png, each rendered from the vector at its own
size, and hicolor/scalable/apps/<id>.svg, the master itself: the same drawing at every size,
for docks and launchers that prefer a scalable icon (Omarchy's shell indexes SVGs before PNGs).
Needs librsvg's `rsvg-convert`."""

import pathlib
import shutil
import subprocess
import sys

APP_ID = "io.github.pixdevsapps.Sealwax"
SIZES = (16, 24, 32, 48, 64, 128, 256, 512)


def main():
    out = pathlib.Path(sys.argv[1])
    master = out / "source" / "sealwax-icon.svg"
    if not master.is_file():
        sys.exit(f"{master} is missing")
    for size in SIZES:
        path = out / "hicolor" / f"{size}x{size}" / "apps" / f"{APP_ID}.png"
        path.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["rsvg-convert", "--width", str(size), "--height", str(size), "--keep-aspect-ratio",
             "--output", str(path), str(master)],
            check=True,
        )
        print(path)
    scalable = out / "hicolor" / "scalable" / "apps" / f"{APP_ID}.svg"
    scalable.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(master, scalable)
    print(scalable)


if __name__ == "__main__":
    main()
