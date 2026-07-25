#!/usr/bin/env python3
"""Prop generation chain assembly (Phase A3.9): concept -> geometry ->
cleanup -> texture -> preprocess+bake -> turntable -> chained manifest. One
invocation = one candidate under <out>/cand_<seed>/. Plain system Python --
this script only subprocess-orchestrates the per-stage tools, each of which
runs under its own venv/interpreter (Hi3DGen venv, Blender, node, cargo).

Every stage is skipped if its output already exists, so re-running the same
command resumes rather than restarts. Any stage's non-zero exit aborts the
whole chain with that stage named -- no silent fallbacks.

Run:
  python scripts/ai-pipeline/gen_prop.py "<subject prompt>" --out <dir> --seed N [--skip-concept <image.png>]

Every ComfyUI stage owns its server lifecycle (comfy_run.server()): the
concept stage and the multiview texture strategy (inside prop_texture.py)
each start a headless server and stop it before returning, so the chain
runs unattended and ComfyUI is never up while the geometry stage runs
(A3.4's measured 11.5 GiB Hi3DGen peak). An external ComfyUI server is
refused, not reused: the chain can't stop somebody else's server before
geometry.
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

import comfy_run

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent

PROP_CONCEPT_WORKFLOW = SCRIPT_DIR / "workflows" / "prop_concept.json"
PROP_HI3DGEN = SCRIPT_DIR / "prop_hi3dgen.py"
PROP_CLEANUP = SCRIPT_DIR / "prop_cleanup.py"
PROP_TEXTURE = SCRIPT_DIR / "prop_texture.py"
PREPROCESS_PROP_MJS = SCRIPT_DIR / "preprocess_prop.mjs"
BAKE_TEXTURES_MJS = REPO_ROOT / "scripts" / "asset-pipeline" / "bake_textures.mjs"

HI3DGEN_PYTHON = Path(r"C:\tools\Hi3DGen\venv\Scripts\python.exe")
HI3DGEN_REPO = Path(r"C:\tools\Hi3DGen\Hi3DGen")
BLENDER = Path(r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe")

TURNTABLE_ANGLES = 8
TURNTABLE_SIZE = "512x512"

STAGES = ["concept", "geometry", "cleanup", "texture", "preprocess", "turntable"]


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(cmd: list, cwd: Path = None) -> None:
    """Stream a subprocess to the console; abort the chain on non-zero exit."""
    proc = subprocess.run([str(c) for c in cmd], cwd=str(cwd) if cwd else None)
    if proc.returncode != 0:
        sys.exit(f"gen_prop: command failed (exit {proc.returncode}): {' '.join(str(c) for c in cmd)}")


def run_capture(cmd: list, cwd: Path = None) -> str:
    """Like run(), but captures stdout (for stages that print a JSON stats
    line rather than writing their own manifest file) and echoes both
    streams so the run stays visible."""
    proc = subprocess.run([str(c) for c in cmd], cwd=str(cwd) if cwd else None,
                          capture_output=True, text=True, encoding="utf-8", errors="replace")
    sys.stdout.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        sys.exit(f"gen_prop: command failed (exit {proc.returncode}): {' '.join(str(c) for c in cmd)}")
    return proc.stdout


def last_json_line(text: str) -> dict:
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("{"):
            return json.loads(line)
    sys.exit("gen_prop: expected a JSON stats line in stage output, found none")


def read_or_note(meta_path: Path) -> dict:
    """Stage provenance sidecar, or an honest placeholder if the stage's
    output pre-existed this script's involvement (e.g. seeded from another
    tool's run) and so was never captured."""
    if meta_path.exists():
        return json.loads(meta_path.read_text(encoding="utf-8"))
    return {"note": f"no stats captured ({meta_path.name} not found; output pre-existed this gen_prop.py run)"}


def stage_concept(cand_dir: Path, subject: str, seed: int, skip_concept: Path) -> dict:
    concept_png = cand_dir / "concept.png"
    meta_path = cand_dir / "concept_meta.json"
    if concept_png.exists():
        print(f"concept: skip (exists) -> {concept_png}")
        meta = read_or_note(meta_path)
        meta["concept_png_sha256"] = sha256_file(concept_png)
        return meta

    if skip_concept is not None:
        shutil.copyfile(skip_concept, concept_png)
        meta = {
            "mode": "skip-concept",
            "source_image": str(skip_concept),
            "source_image_sha256": sha256_file(skip_concept),
        }
        print(f"concept: skip-concept -> copied {skip_concept} -> {concept_png}")
    else:
        # A3.7 convention: prop_concept.json ships a {subject} placeholder.
        workflow = json.loads(PROP_CONCEPT_WORKFLOW.read_text(encoding="utf-8").replace("{subject}", subject))
        for node in workflow.values():
            inputs = node.get("inputs", {})
            for key in inputs:
                if key in ("seed", "noise_seed"):
                    inputs[key] = seed

        concept_raw = cand_dir / "concept_raw"
        with comfy_run.server():
            manifest = comfy_run.run_workflow(workflow, concept_raw)
        pngs = sorted(concept_raw.glob("*.png"))
        if len(pngs) != 1:
            sys.exit(f"gen_prop: concept stage produced {len(pngs)} PNG(s) in {concept_raw}, expected exactly 1")
        shutil.copyfile(pngs[0], concept_png)
        meta = {
            "mode": "generated",
            "subject": subject,
            "comfy_manifest": manifest,
        }
        print(f"concept: generated -> {concept_png}")

    meta_path.write_text(json.dumps(meta, indent=2), encoding="utf-8")
    meta["concept_png_sha256"] = sha256_file(concept_png)
    return meta


def stage_geometry(cand_dir: Path, seed: int) -> dict:
    raw_glb = cand_dir / "raw.glb"
    concept_rgba = cand_dir / "concept_rgba.png"
    hi3dgen_manifest_path = cand_dir / "hi3dgen_manifest.json"
    if raw_glb.exists() and concept_rgba.exists():
        print(f"geometry: skip (exists) -> {raw_glb}")
    else:
        concept_png = cand_dir / "concept.png"
        run([HI3DGEN_PYTHON, PROP_HI3DGEN, concept_png, "--out", cand_dir, "--seed", seed], cwd=HI3DGEN_REPO)
        # prop_hi3dgen.py writes <out>/generation_manifest.json -- move it
        # aside immediately so that filename stays reserved for this script's
        # own chained manifest (step 7); otherwise the aggregate step would
        # overwrite this stage's provenance and re-runs couldn't recover it.
        (cand_dir / "generation_manifest.json").replace(hi3dgen_manifest_path)
        print(f"geometry: generated -> {raw_glb}")
    meta = read_or_note(hi3dgen_manifest_path)
    meta["raw_glb_sha256"] = sha256_file(raw_glb)
    meta["concept_rgba_sha256"] = sha256_file(concept_rgba)
    return meta


def stage_cleanup(cand_dir: Path, symmetrize: bool, symmetrize_keep: str) -> dict:
    clean_glb = cand_dir / "clean.glb"
    hires_glb = cand_dir / "clean_hires.glb"
    meta_path = cand_dir / "cleanup_stats.json"
    if clean_glb.exists() and hires_glb.exists():
        print(f"cleanup: skip (exists) -> {clean_glb}")
    else:
        raw_glb = cand_dir / "raw.glb"
        cmd = [BLENDER, "--background", "--python", PROP_CLEANUP, "--", raw_glb, clean_glb]
        if symmetrize:
            cmd += ["--symmetrize", f"--symmetrize-keep={symmetrize_keep}"]
        out = run_capture(cmd)
        meta_path.write_text(json.dumps(last_json_line(out), indent=2), encoding="utf-8")
        print(f"cleanup: generated -> {clean_glb}")
    meta = read_or_note(meta_path)
    meta["clean_glb_sha256"] = sha256_file(clean_glb)
    meta["clean_hires_glb_sha256"] = sha256_file(hires_glb)
    return meta


def stage_texture(cand_dir: Path, strategy: str, subject: str, seed: int,
                  metallic: float, roughness: float, detail: bool, azimuths: str,
                  view_res: int, texture_size: int) -> dict:
    textured_glb = cand_dir / "textured.glb"
    meta_path = cand_dir / "texture_stats.json"
    if textured_glb.exists():
        print(f"texture: skip (exists) -> {textured_glb}")
    else:
        clean_glb = cand_dir / "clean.glb"
        hires_glb = cand_dir / "clean_hires.glb"
        concept_rgba = cand_dir / "concept_rgba.png"
        cmd = [BLENDER, "--background", "--python", PROP_TEXTURE, "--",
               clean_glb, hires_glb, concept_rgba, textured_glb]
        if strategy != "projection":
            cmd += ["--strategy", strategy, "--subject", subject, "--seed", seed]
        if metallic is not None:
            cmd += ["--metallic", metallic]
        if roughness is not None:
            cmd += ["--roughness", roughness]
        if detail:
            cmd += ["--detail"]
        if azimuths is not None:
            cmd += ["--azimuths", azimuths]
        if view_res is not None:
            cmd += ["--view-res", view_res]
        if texture_size is not None:
            cmd += ["--texture-size", texture_size]
        out = run_capture(cmd)
        meta_path.write_text(json.dumps(last_json_line(out), indent=2), encoding="utf-8")
        print(f"texture: generated -> {textured_glb}")
    meta = read_or_note(meta_path)
    meta["textured_glb_sha256"] = sha256_file(textured_glb)
    return meta


def stage_preprocess_bake(cand_dir: Path, max_dim: int = None, max_bytes: int = None) -> dict:
    textured_glb = cand_dir / "textured.glb"
    final_glb = cand_dir / "final.glb"
    bake_manifest = cand_dir / "final.textures" / "manifest.json"
    meta_path = cand_dir / "preprocess_stats.json"

    if final_glb.exists():
        print(f"preprocess: skip (exists) -> {final_glb}")
    else:
        preprocess_stats = {"textured_glb_bytes": textured_glb.stat().st_size}
        cmd = ["node", PREPROCESS_PROP_MJS, textured_glb, final_glb]
        if max_dim is not None:
            cmd += ["--max-dim", max_dim]
        if max_bytes is not None:
            cmd += ["--max-bytes", max_bytes]
        run(cmd)
        preprocess_stats["final_glb_bytes"] = final_glb.stat().st_size
        meta_path.write_text(json.dumps(preprocess_stats, indent=2), encoding="utf-8")
        print(f"preprocess: generated -> {final_glb}")

    if bake_manifest.exists():
        print(f"bake: skip (exists) -> {bake_manifest}")
    else:
        run(["node", BAKE_TEXTURES_MJS, "gltf", final_glb])
        print(f"bake: generated -> {bake_manifest}")

    return {
        "preprocess": read_or_note(meta_path),
        "bake": json.loads(bake_manifest.read_text(encoding="utf-8")),
        "final_glb_sha256": sha256_file(final_glb),
    }


def stage_turntable(cand_dir: Path) -> dict:
    contact_sheet = cand_dir / "contact_sheet.png"
    if contact_sheet.exists():
        print(f"turntable: skip (exists) -> {contact_sheet}")
    else:
        final_glb = cand_dir / "final.glb"
        run(["cargo", "run", "-p", "engine-renderer", "--release", "--features", "offscreen", "--bin", "turntable",
             "--", final_glb, "--out", cand_dir, "--angles", TURNTABLE_ANGLES, "--size", TURNTABLE_SIZE],
            cwd=REPO_ROOT)
        print(f"turntable: generated -> {contact_sheet}")
    frames = sorted(p.name for p in cand_dir.glob("frame_*.png"))
    return {"angles": TURNTABLE_ANGLES, "size": TURNTABLE_SIZE, "contact_sheet": contact_sheet.name, "frames": frames}


def main():
    # Line-buffer stdout: without this, CPython fully buffers stdout when it
    # isn't a tty, so this script's own stage-progress prints all land at
    # process exit instead of interleaving live with each subprocess's
    # inherited-fd output -- misleading during the multi-minute real runs
    # this same chain drives in A3.10.
    sys.stdout.reconfigure(line_buffering=True)

    parser = argparse.ArgumentParser(description="Generate one prop candidate through the full A3 chain.")
    parser.add_argument("subject", help="Object description substituted into prop_concept.json's {subject}")
    parser.add_argument("--out", type=Path, required=True, help="Batch directory; the candidate lands in <out>/cand_<seed>/")
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--skip-concept", type=Path, default=None, metavar="IMAGE",
                        help="Bypass concept generation with a provided image (re-roll geometry without re-rolling the concept)")
    parser.add_argument("--texture-strategy", choices=["projection", "multiview"], default="projection",
                        help="Basecolor strategy for prop_texture.py (multiview = ControlNet-depth retexture)")
    parser.add_argument("--metallic", type=float, default=None,
                        help="Declared metallic for prop_texture.py, both strategies (default 0)")
    parser.add_argument("--roughness", type=float, default=None,
                        help="Declared roughness for prop_texture.py, both strategies (default 0.8)")
    parser.add_argument("--detail", action="store_true",
                        help="Opt the material into the world-space triplanar detail overlay "
                             "for prop_texture.py -- masonry/stone classes only, never "
                             "alpha-masked foliage cards")
    parser.add_argument("--symmetrize", action="store_true",
                        help="Mirror one half of the cleaned mesh across its best-fit vertical plane")
    parser.add_argument("--symmetrize-keep", choices=["+x", "-x"], default="+x",
                        help="Half to mirror in prop_cleanup.py's plane-aligned frame")
    parser.add_argument("--azimuths", default=None, metavar="DEG,DEG,...",
                        help="Multiview camera azimuths for prop_texture.py")
    parser.add_argument("--view-res", type=int, default=None,
                        help="Multiview strategy only: prop_texture.py's per-view depth/"
                             "normal render and Z-Image + Fun ControlNet-Union generation "
                             "resolution (default 1024; the MaterialAnything estimator's "
                             "input stays pinned at 768x768 regardless)")
    parser.add_argument("--texture-size", type=int, default=None,
                        help="prop_texture.py's atlas bake resolution (default 1024); "
                             "safe to raise without re-running cleanup (island gutters "
                             "only grow)")
    parser.add_argument("--max-dim", type=int, default=None,
                        help="preprocess_prop.mjs's texture resize cap (default 1024) -- "
                             "raise alongside --texture-size, or the preprocess stage "
                             "silently downscales the bake back down")
    parser.add_argument("--max-bytes", type=int, default=None,
                        help="preprocess_prop.mjs's final.glb size assert (default 8 MB, "
                             "the prop cap)")
    parser.add_argument("--through", choices=STAGES, default="turntable",
                        help="Stop after this stage (batch triage: --through cleanup "
                             "sweeps geometry seeds without paying for texturing)")
    args = parser.parse_args()

    # Resolve once here: stage_geometry and stage_turntable run their
    # subprocess under cwd=HI3DGEN_REPO / cwd=REPO_ROOT respectively, so a
    # relative --out would otherwise resolve against the wrong directory.
    args.out = args.out.resolve()

    cand_dir = args.out / f"cand_{args.seed}"
    cand_dir.mkdir(parents=True, exist_ok=True)

    stop = STAGES.index(args.through)
    manifest = {
        "subject": args.subject,
        "seed": args.seed,
        "candidate_dir": str(cand_dir),
        "concept": stage_concept(cand_dir, args.subject, args.seed, args.skip_concept),
    }
    if stop >= STAGES.index("geometry"):
        manifest["geometry"] = stage_geometry(cand_dir, args.seed)
    if stop >= STAGES.index("cleanup"):
        manifest["cleanup"] = stage_cleanup(cand_dir, args.symmetrize, args.symmetrize_keep)
    if stop >= STAGES.index("texture"):
        manifest["texture"] = stage_texture(cand_dir, args.texture_strategy, args.subject, args.seed,
                                            args.metallic, args.roughness, args.detail, args.azimuths,
                                            args.view_res, args.texture_size)
    if stop >= STAGES.index("preprocess"):
        # final.glb, needed by the turntable stage below
        manifest.update(stage_preprocess_bake(cand_dir, args.max_dim, args.max_bytes))
    if stop >= STAGES.index("turntable"):
        manifest["turntable"] = stage_turntable(cand_dir)
    manifest_path = cand_dir / "generation_manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"OK: wrote {manifest_path}")


if __name__ == "__main__":
    main()
