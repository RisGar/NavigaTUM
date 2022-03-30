import os
from pathlib import Path

from PIL import Image


def convert_to_webp(source: Path):
    """Convert image(s) to WebP.

    Args:
        source (pathlib.Path): Path to source image(s)

    Returns:
        pathlib.Path: path to new image(s)
    """
    if source.is_dir():
        for img_path in os.listdir(source):
            img_path = Path(source, img_path)
            if img_path.suffix not in ['.webp', ".yaml"]:
                convert_to_webp(img_path)
        return source

    destination = source.with_suffix(".webp")

    image = Image.open(source)
    image.save(destination, format="webp")
    os.remove(source)
    return str(destination)
