"""check_matte's gate must agree with the Hi3DGen bbox test it feeds.

Requires the Hi3DGen venv (prop_hi3dgen imports torch/hi3dgen at module
scope); skipped under a plain interpreter:
C:\\tools\\Hi3DGen\\venv\\Scripts\\python.exe -m unittest discover -s scripts/tests -t .
"""
import sys
import unittest
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "ai-pipeline"))

try:
    import prop_hi3dgen
    from hi3dgen.pipelines import Hi3DGenPipeline
except ImportError as e:  # pragma: no cover - environment probe
    prop_hi3dgen = None
    IMPORT_ERROR = e


def matte(alpha_value):
    """A 256x256 matte with a 128x128 object at the given alpha."""
    arr = np.zeros((256, 256, 4), np.uint8)
    arr[:, :, :3] = 128
    arr[64:192, 64:192, 3] = alpha_value
    return Image.fromarray(arr, "RGBA")


@unittest.skipIf(prop_hi3dgen is None, "needs the Hi3DGen venv")
class CheckMatteThreshold(unittest.TestCase):
    def test_soft_matte_rejected_by_both_gates(self):
        """Alpha 127 clears the old 0.1*255 gate but no pixel reaches the
        bbox threshold, so preprocess_image has nothing to crop."""
        soft = matte(127)
        with self.assertRaises(prop_hi3dgen.DegenerateMatteError):
            prop_hi3dgen.check_matte(soft)
        with self.assertRaises(ValueError):
            Hi3DGenPipeline.preprocess_image(None, soft, resolution=1024)

    def test_opaque_matte_accepted_by_both_gates(self):
        opaque = matte(255)
        self.assertAlmostEqual(prop_hi3dgen.check_matte(opaque), 0.25, places=6)
        out = Hi3DGenPipeline.preprocess_image(None, opaque, resolution=1024)
        self.assertEqual(out.size, (1024, 1024))

    def test_full_frame_matte_rejected(self):
        arr = np.full((256, 256, 4), 255, np.uint8)
        with self.assertRaises(prop_hi3dgen.DegenerateMatteError):
            prop_hi3dgen.check_matte(Image.fromarray(arr, "RGBA"))


@unittest.skipIf(prop_hi3dgen is None, "needs the Hi3DGen venv")
class PreprocessBackground(unittest.TestCase):
    def test_transparent_area_carries_no_black(self):
        """Straight colour under a coverage alpha: nothing in the returned
        image is darkened by the background it will be composited onto."""
        arr = np.zeros((256, 256, 4), np.uint8)
        arr[:, :, :3] = (255, 255, 255)
        arr[64:192, 64:192, 3] = 255
        out = np.asarray(Hi3DGenPipeline.preprocess_image(
            None, Image.fromarray(arr, "RGBA"), resolution=1024))
        edge = (out[:, :, 3] > 2) & (out[:, :, 3] < 253)
        self.assertGreater(int(edge.sum()), 0, "expected a resampled silhouette band")
        self.assertEqual(int(out[:, :, :3][edge].min()), 255,
                         "silhouette colour was dimmed by an implicit background")


if __name__ == "__main__":
    unittest.main()
