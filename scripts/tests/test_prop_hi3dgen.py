"""check_matte's gate must agree with the Hi3DGen bbox test it feeds,
check_mesh must drop zero-area faces while still refusing broken geometry, and
every sweep axis the CLI exposes must reach the sampler it names.

Requires the Hi3DGen venv (prop_hi3dgen imports torch/hi3dgen at module
scope); skipped under a plain interpreter:
C:\\tools\\Hi3DGen\\venv\\Scripts\\python.exe -m unittest discover -s scripts/tests -t .
"""
import inspect
import json
import sys
import types
import unittest
from pathlib import Path

import numpy as np
import trimesh
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "ai-pipeline"))

try:
    import prop_hi3dgen
    from hi3dgen import headless
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


def box_with_degenerate_faces(n_degenerate):
    """A 12-face box plus n_degenerate zero-area faces built from a vertex
    duplicated at the same position, the shape the GPU extractor emits."""
    box = trimesh.creation.box()
    vertices = np.vstack([box.vertices, box.vertices[:1]])
    dup = len(box.vertices)
    faces = np.vstack([box.faces] + [[0, dup, 0]] * n_degenerate)
    return trimesh.Trimesh(vertices=vertices, faces=faces, process=False)


@unittest.skipIf(prop_hi3dgen is None, "needs the Hi3DGen venv")
class CheckMeshDegenerateFaces(unittest.TestCase):
    def setUp(self):
        self.result = types.SimpleNamespace(success=True)

    def test_zero_area_faces_are_dropped_and_counted(self):
        mesh = box_with_degenerate_faces(2)
        stats = prop_hi3dgen.check_mesh(self.result, mesh)
        self.assertEqual(stats["degenerate_face_count"], 2)
        self.assertEqual(stats["face_count"], 12)
        self.assertEqual(len(mesh.faces), 12)
        self.assertEqual(int((mesh.area_faces <= 0).sum()), 0)
        self.assertEqual(stats["vertex_count"], len(mesh.vertices))

    def test_clean_mesh_untouched(self):
        mesh = box_with_degenerate_faces(0)
        stats = prop_hi3dgen.check_mesh(self.result, mesh)
        self.assertEqual(stats["degenerate_face_count"], 0)
        self.assertEqual(stats["face_count"], 12)

    def test_all_faces_degenerate_still_refused(self):
        mesh = trimesh.Trimesh(
            vertices=np.zeros((3, 3)), faces=np.array([[0, 1, 2]]), process=False)
        with self.assertRaises(prop_hi3dgen.DegenerateMeshError):
            prop_hi3dgen.check_mesh(self.result, mesh)

    def test_non_finite_vertices_still_refused(self):
        mesh = box_with_degenerate_faces(0)
        mesh.vertices[0] = np.nan
        with self.assertRaises(prop_hi3dgen.DegenerateMeshError):
            prop_hi3dgen.check_mesh(self.result, mesh)

    def test_failed_extraction_still_refused(self):
        with self.assertRaises(prop_hi3dgen.DegenerateMeshError):
            prop_hi3dgen.check_mesh(
                types.SimpleNamespace(success=False), box_with_degenerate_faces(0))


@unittest.skipIf(prop_hi3dgen is None, "needs the Hi3DGen venv")
class SweepAxisPlumbing(unittest.TestCase):
    """Each knob has to travel CLI -> sample_kwargs -> Session.sample -> the
    sampler dict, which is the whole path a sweep arm varies."""

    def parse(self, *argv):
        return prop_hi3dgen.build_parser().parse_args(["concept.png", "--out", "batch", *argv])

    def merged(self, kwargs, stage):
        """The dict the named stage's sampler would run with, built by the
        same helper Session.sample uses."""
        spec = json.loads((headless.GEOMETRY_WEIGHTS / "pipeline.json").read_text())
        base = spec["args"][f"{stage}_sampler"]["params"]
        prefix = "ss" if stage == "sparse_structure" else "slat"
        return headless.sampler_params(
            base, steps=kwargs[f"{prefix}_steps"], cfg=kwargs[f"{prefix}_cfg"],
            cfg_interval_lo=kwargs[f"{prefix}_cfg_interval_lo"],
            rescale_t=kwargs[f"{prefix}_rescale_t"])

    def test_every_kwarg_is_a_sample_parameter(self):
        kwargs = prop_hi3dgen.sample_kwargs(self.parse())
        inspect.signature(headless.Session.sample).bind(
            headless.Session, 0, **kwargs)

    def test_unflagged_run_keeps_checkpoint_sampler_values(self):
        kwargs = prop_hi3dgen.sample_kwargs(self.parse())
        for stage in ("sparse_structure", "slat"):
            merged = self.merged(kwargs, stage)
            self.assertEqual(merged["cfg_interval"], [0.5, 1.0], stage)
            self.assertEqual(merged["rescale_t"], 3.0, stage)
        self.assertEqual(kwargs["occupancy_threshold"], headless.OCCUPANCY_THRESHOLD_DEFAULT)

    def test_flagged_run_overrides_only_the_named_stage(self):
        kwargs = prop_hi3dgen.sample_kwargs(self.parse(
            "--ss-cfg-interval-lo", "0.0", "--slat-rescale-t", "1.0",
            "--occupancy-threshold", "-0.5"))
        ss = self.merged(kwargs, "sparse_structure")
        slat = self.merged(kwargs, "slat")
        self.assertEqual(ss["cfg_interval"], (0.0, 1.0))
        self.assertEqual(ss["rescale_t"], 3.0)
        self.assertEqual(slat["cfg_interval"], [0.5, 1.0])
        self.assertEqual(slat["rescale_t"], 1.0)
        self.assertEqual(kwargs["occupancy_threshold"], -0.5)


@unittest.skipIf(prop_hi3dgen is None, "needs the Hi3DGen venv")
class MultiViewCli(unittest.TestCase):
    """The positional image is view 0 and each --view appends after it, in the
    order the conditioning encoder receives them."""

    def parse(self, *argv):
        return prop_hi3dgen.build_parser().parse_args(["concept.png", "--out", "batch", *argv])

    def test_extra_views_collected_in_order(self):
        args = self.parse("--view", "back.png", "--view", "side.png")
        self.assertEqual(args.extra_views, [Path("back.png"), Path("side.png")])

    def test_single_view_run_has_no_extra_views(self):
        args = self.parse()
        self.assertEqual(args.extra_views, [])
        self.assertEqual(args.mv_mode, "multidiffusion")

    def test_unknown_mv_mode_refused(self):
        with self.assertRaises(SystemExit):
            self.parse("--mv-mode", "bogus")

    def test_mv_mode_reaches_sample_kwargs(self):
        kwargs = prop_hi3dgen.sample_kwargs(self.parse("--mv-mode", "stochastic"))
        self.assertEqual(kwargs["mv_mode"], "stochastic")


@unittest.skipIf(prop_hi3dgen is None, "needs the Hi3DGen venv")
class DeterministicFlag(unittest.TestCase):
    def parse(self, *argv):
        return prop_hi3dgen.build_parser().parse_args(["concept.png", "--out", "batch", *argv])

    def test_defaults_false(self):
        self.assertFalse(self.parse().deterministic)

    def test_flag_sets_true(self):
        self.assertTrue(self.parse("--deterministic").deterministic)


if __name__ == "__main__":
    unittest.main()
