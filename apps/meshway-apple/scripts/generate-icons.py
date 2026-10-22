#!/usr/bin/env python3
"""Normalize the four approved source PNGs and regenerate the App asset catalog.

Requires Pillow 12.1.0. Run with --source-dir pointing at the original artwork;
--evidence-dir optionally writes masters, small PNGs and a contact sheet there.
The source-specific crop/mask follows the inside edge of each glass tile. It
removes the generated alpha debris outside that edge without repainting the mark.
"""

import argparse
import json
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw


ARTWORK = {
    "1-ink": ((34, 36, 993, 968), 224),
    "2-light": ((18, 20, 1006, 994), 238),
    "3-gradient": ((29, 31, 995, 993), 220),
    "4-mono": ((32, 32, 995, 986), 226),
}
INFO = {"author": "xcode", "version": 1}


def contents(directory, images):
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "Contents.json").write_text(
        json.dumps({"images": images, "info": INFO}, indent=2) + "\n"
    )


def normalize(source, bounds, radius):
    if source.size != (1024, 1024):
        raise ValueError("Expected the approved 1024×1024 artwork")
    tile = source.convert("RGBA").crop(bounds)
    # Supersampling gives a clean antialiased contour, not a feather or shadow.
    scale = 4
    mask = Image.new("L", (tile.width * scale, tile.height * scale))
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, mask.width - 1, mask.height - 1), radius=radius * scale, fill=255
    )
    mask = mask.resize(tile.size, Image.Resampling.LANCZOS)
    tile.putalpha(ImageChops.multiply(tile.getchannel("A"), mask))
    tile.thumbnail((824, 824), Image.Resampling.LANCZOS)
    master = Image.new("RGBA", (1024, 1024))
    master.paste(tile, ((1024 - tile.width) // 2, (1024 - tile.height) // 2))
    # Canonical transparent RGB avoids hidden source debris in the output.
    master.paste((0, 0, 0, 0), mask=master.getchannel("A").point(lambda a: 255 if a == 0 else 0))
    return master


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-dir", type=Path, required=True)
    parser.add_argument("--evidence-dir", type=Path)
    args = parser.parse_args()
    assets = Path(__file__).resolve().parents[1] / "App/Assets.xcassets"
    assets.mkdir(parents=True, exist_ok=True)
    (assets / "Contents.json").write_text(json.dumps({"info": INFO}, indent=2) + "\n")
    masters = {}
    for name, (bounds, radius) in ARTWORK.items():
        with Image.open(args.source_dir / f"variant-{name}.png") as source:
            master = normalize(source, bounds, radius)
        masters[name] = master
        directory = assets / f"AppIconChoice-{name}.imageset"
        contents(directory, [{"filename": "master.png", "idiom": "universal"}])
        master.save(directory / "master.png")
        print(f"{name}: source crop {bounds}, output alpha bounds {master.getbbox()}")

    directory = assets / "AppIcon.appiconset"
    images = []
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            filename = f"icon-{size}@{scale}x.png"
            images.append({"filename": filename, "idiom": "mac", "size": f"{size}x{size}", "scale": f"{scale}x"})
    contents(directory, images)
    for entry in images:
        pixels = int(entry["size"].split("x")[0]) * int(entry["scale"][0])
        masters["2-light"].resize((pixels, pixels), Image.Resampling.LANCZOS).save(directory / entry["filename"])

    if args.evidence_dir:
        args.evidence_dir.mkdir(parents=True, exist_ok=True)
        sheet = Image.new("RGB", (1440, 460), "#e6e7eb")
        draw = ImageDraw.Draw(sheet)
        for index, (name, master) in enumerate(masters.items()):
            master.save(args.evidence_dir / f"processed-{name}.png")
            sheet.paste(master.resize((360, 360), Image.Resampling.LANCZOS), (index * 360, 0), master.resize((360, 360), Image.Resampling.LANCZOS))
            draw.text((index * 360 + 24, 360), name, fill="#2a2b33")
            x = index * 360 + 24
            for size in (16, 32, 64, 128):
                small = master.resize((size, size), Image.Resampling.LANCZOS)
                small.save(args.evidence_dir / f"processed-{name}-{size}.png")
                if size <= 64:
                    sheet.paste(small, (x, 390), small)
                    x += size + 16
        sheet.save(args.evidence_dir / "processed-contact-sheet.png")


if __name__ == "__main__":
    main()
