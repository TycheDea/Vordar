# One-off base-model bake-off (A5b): same ortho depth maps, same subject, same
# seed -- only the image model changes. Scores texture-fitness, which is what
# the texturing stage actually needs: depth adherence, FLAT albedo (baked
# lighting is a defect -- prop_texture.py emit-bakes these straight into
# basecolor), material detail, and view-to-view agreement.
#
# Kept past the 2026-07-20 ruling (Z-Image) because Qwen-Image is retained as
# the documented fallback: this harness plus the metrics in
# tasks/ai-pipeline/research/a5b-bakeoff-results.md are what make revisiting it
# cheap. Not pipeline code -- it never runs as part of asset generation.
#
# Usage: blender --background --python run.py -- <mesh.glb> <out_dir>
#            [--views N] [--models sdxl,zimage,qwen]
import json
import shutil
import sys
import time
from pathlib import Path

import bpy

BAKEOFF_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(BAKEOFF_DIR.parent))
import prop_texture as pt  # noqa: E402  (guarded main; safe to import)
import comfy_run  # noqa: E402

WORKFLOWS = {
    "sdxl": pt.MV_WORKFLOW,
    "zimage": BAKEOFF_DIR / "wf_zimage.json",
    "qwen": BAKEOFF_DIR / "wf_qwen.json",
    # cfg-1 rewrite of the SDXL prompt: materials named in the positive, kept
    # short. Prompt verbosity trades against geometric fidelity on a distilled
    # base, so the long variants that established that scored worse and are gone.
    "zimage_short": BAKEOFF_DIR / "wf_zimage_short.json",
}
# The shipped candelabra's own subject string, so this reproduces a real
# asset's generation with the model as the only variable.
SUBJECT = ("wrought iron candelabra shrine, standing votive candle stand with "
           "melted wax candles, near-black weathered dark iron, stone base, "
           "semi-realistic dark fantasy")
SEED = 2


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    positional, views, models = [], len(pt.MV_VIEWS), list(WORKFLOWS)
    i = 0
    while i < len(argv):
        if argv[i] == "--views":
            i += 1
            views = int(argv[i])
        elif argv[i] == "--models":
            i += 1
            models = argv[i].split(",")
        else:
            positional.append(argv[i])
        i += 1
    if len(positional) != 2:
        pt.fail("usage: run.py -- <mesh.glb> <out_dir> [--views N] [--models a,b]")
    # Blender resolves relative render paths against its own notion of cwd, not
    # the shell's -- every path handed to bpy has to be absolute.
    return Path(positional[0]).resolve(), Path(positional[1]).resolve(), views, models


def render_depths(mesh, out_dir, n_views):
    """Reuse the pipeline's own rig + depth renderer so the conditioning
    images are byte-identical to what the real stage would feed. The scene
    setup below has to match prop_texture.main()'s: without the empty-scene
    reset the startup cube renders into the depth map, and without Cycles the
    emission depth ramp is not what the real stage produces."""
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 1
    scene.cycles.use_denoising = False

    clean = pt.import_glb(mesh)
    hires = pt.import_glb(mesh)  # render_depth_views hides this one
    views, rig = pt.mv_camera_rig(clean)
    views = views[:n_views]
    pt.render_depth_views(clean, hires, views, rig, out_dir)
    return views


def run_model(model, views, out_dir):
    template = json.loads(WORKFLOWS[model].read_text(encoding="utf-8"))
    model_dir = out_dir / model
    timings = []
    for i, view in enumerate(views):
        gen = model_dir / f"view_{i}.png"
        if gen.exists():
            print(f"  {model} view {i}: cached")
            continue
        depth_png = out_dir / f"depth_{i}.png"
        input_name = f"bakeoff_{pt.sha256_file(depth_png)[:8]}_{i}.png"
        shutil.copyfile(depth_png, pt.COMFY_INPUT_DIR / input_name)
        wf = json.loads(json.dumps(template))
        for node in wf.values():
            inputs = node.get("inputs", {})
            for key, value in inputs.items():
                if isinstance(value, str):
                    inputs[key] = (value.replace("{subject}", SUBJECT)
                                   .replace("{view_hint}", view["hint"])
                                   .replace("{depth_image}", input_name))
                elif key == "seed":
                    inputs[key] = SEED * 100 + i
        started = time.monotonic()
        manifest = comfy_run.run_workflow(wf, model_dir / f"_run_{i}", wait_timeout=1800)
        elapsed = time.monotonic() - started
        pngs = [o for o in manifest["outputs"] if o["filename"].endswith(".png")]
        if len(pngs) != 1:
            pt.fail(f"{model} view {i}: expected 1 PNG, got {len(pngs)}")
        shutil.copyfile(pngs[0]["saved_as"], gen)
        (pt.COMFY_INPUT_DIR / input_name).unlink()
        timings.append(elapsed)
        print(f"  {model} view {i}: {elapsed:.1f}s")
    return timings


def main():
    mesh, out_dir, n_views, models = parse_args()
    out_dir.mkdir(parents=True, exist_ok=True)
    views = render_depths(mesh, out_dir, n_views)
    print(f"depth: {len(views)} view(s) -> {out_dir}")

    proc = pt.start_comfy()
    report = {}
    try:
        for model in models:
            print(f"[{model}]")
            report[model] = run_model(model, views, out_dir)
    finally:
        proc.kill()
        proc.wait()

    (out_dir / "timings.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps({m: round(sum(t) / len(t), 1) if t else None
                      for m, t in report.items()}))


if __name__ == "__main__":
    main()
