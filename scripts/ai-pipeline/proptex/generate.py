"""The ComfyUI canvas for the multiview retexture: pairs opposite views into
a shared conditioning canvas, runs one pair through the ControlNet-depth
workflow, and splits the decoded canvas back into per-slot crops
(prop_texture.py's basecolor path, proptex.albedo consumes the crops).

This module imports no bpy: a Cycles render inside the comfy_run.server()
block its caller holds open would violate the VRAM sequencing that block
itself refuses to share, a 10.6-12.3 GiB Hi3DGen peak that cannot coexist with
the ComfyUI process. Keeping Blender out of the file makes that structural
rather than remembered.

`import comfy_run` resolves only because the entry points (prop_texture.py,
proptex/coverage.py) put scripts/ai-pipeline -- one level above this package
-- on sys.path before importing anything under proptex; this module does not
repeat that insert.
"""

import hashlib
import json
import shutil
import tempfile
from pathlib import Path

import cv2

import comfy_run

MV_WORKFLOW = Path(__file__).resolve().parents[1] / "workflows" / "prop_multiview.json"


class GenerateError(Exception):
    pass


def view_pairs(n, n_extra=0):
    """Opposite-azimuth pairs under even spacing (front+back, left+right);
    an odd base count leaves its last view solo. The n_extra picked views at
    the tail are not part of that even spacing, so they must not interleave
    with the base pairing — they pair among themselves in pick order."""
    base = n - n_extra
    half = base // 2
    pairs = [(i, i + half) for i in range(half)]
    if base % 2:
        pairs.append((base - 1,))
    for i in range(base, n, 2):
        pairs.append((i, i + 1) if i + 1 < n else (i,))
    return pairs


def canvas_outputs(pair):
    """Declared output filenames of one canvas: the decoded canvas, its
    per-slot crops, and comfy_run's record of the run that produced them."""
    return ("canvas.png", "manifest.json",
            *(f"gen_{slot}.png" for slot in range(len(pair))))


def workflow_inputs():
    """{input key -> sha256} of everything the conditioning graph is: the
    workflow json and every model file it names, the latter read from the
    models.sha256 manifest that is their single source of truth."""
    template = json.loads(MV_WORKFLOW.read_text(encoding="utf-8"))
    inputs = {f"workflow:{MV_WORKFLOW.name}":
              hashlib.sha256(MV_WORKFLOW.read_bytes()).hexdigest()}
    for model in comfy_run.extract_models(template):
        inputs[f"model:{model['filename']}"] = model["sha256"]
    return inputs


def _canvas_hint(views, pair):
    if len(pair) == 1:
        return views[pair[0]]["hint"]
    return (f"two views of the same object side by side, "
            f"left: {views[pair[0]]['hint']}, "
            f"right: {views[pair[1]]['hint']}")


def render_canvas(views, pair, depth_pngs, canvas_seed, view_res, subject, out_dir):
    """One ControlNet-depth pass: the pair's depth maps tiled side by side
    into a single conditioning canvas, the decoded canvas split back into
    per-slot crops. Requires a comfy_run.server() the caller holds open."""
    template = json.loads(MV_WORKFLOW.read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory() as tmpdir:
        cond = Path(tmpdir) / "depth.png"
        cv2.imwrite(str(cond), cv2.hconcat([cv2.imread(str(p)) for p in depth_pngs]))
        input_name = f"vordar_mv_{hashlib.sha256(cond.read_bytes()).hexdigest()[:8]}.png"
        shutil.copyfile(cond, comfy_run.COMFY_INPUT_DIR / input_name)

    hint = _canvas_hint(views, pair)
    for node in template.values():
        inputs = node.get("inputs", {})
        # keyed by class_type: node-id keying silently broke once already
        if node.get("class_type") == "EmptySD3LatentImage":
            inputs["width"] = view_res * len(pair)
            inputs["height"] = view_res
        for key, value in inputs.items():
            if isinstance(value, str):
                inputs[key] = (value.replace("{subject}", subject)
                               .replace("{view_hint}", hint)
                               .replace("{depth_image}", input_name))
            elif key == "seed":
                inputs[key] = canvas_seed
    try:
        manifest = comfy_run.run_workflow(template, out_dir, wait_timeout=300)
    finally:
        (comfy_run.COMFY_INPUT_DIR / input_name).unlink()

    pngs = [o for o in manifest["outputs"] if o["filename"].endswith(".png")]
    if len(pngs) != 1:
        raise GenerateError(f"expected exactly 1 PNG output, got {len(pngs)}")
    canvas = out_dir / "canvas.png"
    Path(pngs[0]["saved_as"]).replace(canvas)
    decoded = cv2.imread(str(canvas))
    if decoded.shape[:2] != (view_res, view_res * len(pair)):
        raise GenerateError(f"got {decoded.shape[1]}x{decoded.shape[0]}, "
                            f"expected {view_res * len(pair)}x{view_res}")
    for slot in range(len(pair)):
        cv2.imwrite(str(out_dir / f"gen_{slot}.png"),
                    decoded[:, slot * view_res:(slot + 1) * view_res])
