#!/usr/bin/env python3
"""Prop generation chain assembly (Phase A3.9): concept -> geometry ->
cleanup -> texture -> preprocess+bake -> turntable -> chained manifest. One
--seed = one candidate under <out>/cand_<seed>/, and --seed repeats. Plain
system Python -- this script only subprocess-orchestrates the per-stage
tools, each of which runs under its own venv/interpreter (Hi3DGen venv,
Blender, node, cargo).

Every stage is skipped per candidate if its output already exists, so
re-running the same command resumes rather than restarts. Any stage's
non-zero exit aborts the whole chain with that stage named -- no silent
fallbacks.

Run:
  python scripts/ai-pipeline/gen_prop.py --asset <name> --seed N [--seed M ...] --out <dir> [--skip-concept <image.png>]

Every ComfyUI stage owns its server lifecycle (comfy_run.server()): the
concept stage and the multiview texture strategy (inside prop_texture.py)
each start a headless server and stop it before returning, so the chain
runs unattended and ComfyUI is never up while the geometry stage runs
(Hi3DGen peaked 10.6-12.3 GiB of the 12 GiB card across the shipped props,
the top of that spread already over it). An external ComfyUI server is
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
sys.path.insert(0, str(SCRIPT_DIR))
from proptex.registry import resolve  # noqa: E402

REPO_ROOT = SCRIPT_DIR.parent.parent

PROP_CONCEPT_WORKFLOW = SCRIPT_DIR / "workflows" / "prop_concept.json"
PROP_HI3DGEN = SCRIPT_DIR / "prop_hi3dgen.py"
PROP_CLEANUP = SCRIPT_DIR / "prop_cleanup.py"
PROP_TEXTURE = SCRIPT_DIR / "prop_texture.py"
PREPROCESS_PROP_MJS = SCRIPT_DIR / "preprocess_prop.mjs"
BAKE_TEXTURES_MJS = REPO_ROOT / "scripts" / "asset-pipeline" / "bake_textures.mjs"

HI3DGEN_PYTHON = Path(r"C:\tools\Hi3DGen\venv\Scripts\python.exe")
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


def geometry_done(cand_dir: Path) -> bool:
    return all((cand_dir / name).exists()
               for name in ("raw.glb", "concept_rgba.png", "normal.png", "hi3dgen_manifest.json"))


def stage_geometry(out: Path, cand_dirs: dict, concept_sha256: dict) -> dict:
    """One prop_hi3dgen.py process per group of pending candidates sharing a
    concept image: the model load and the normal prediction are the bulk of a
    candidate's cost and neither depends on the seed, but the normal map does
    depend on the image, so candidates drawn from different concepts cannot
    share a process."""
    pending = [seed for seed, cand_dir in cand_dirs.items() if not geometry_done(cand_dir)]
    for seed, cand_dir in cand_dirs.items():
        if seed not in pending:
            print(f"geometry: skip (exists) -> {cand_dir / 'raw.glb'}")

    groups = {}
    for seed in pending:
        groups.setdefault(concept_sha256[seed], []).append(seed)
    for group in groups.values():
        cmd = [HI3DGEN_PYTHON, PROP_HI3DGEN, cand_dirs[group[0]] / "concept.png", "--out", out]
        for seed in group:
            cmd += ["--seed", seed]
        run(cmd)
        for seed in group:
            print(f"geometry: generated -> {cand_dirs[seed] / 'raw.glb'}")

    metas = {}
    for seed, cand_dir in cand_dirs.items():
        meta = read_or_note(cand_dir / "hi3dgen_manifest.json")
        meta["raw_glb_sha256"] = sha256_file(cand_dir / "raw.glb")
        meta["concept_rgba_sha256"] = sha256_file(cand_dir / "concept_rgba.png")
        metas[seed] = meta
    return metas


def stage_cleanup(cand_dir: Path, asset: str, height_m: float, tri_budget: int,
                  symmetrize: bool, symmetrize_keep: str) -> dict:
    clean_glb = cand_dir / "clean.glb"
    hires_glb = cand_dir / "clean_hires.glb"
    meta_path = cand_dir / "cleanup_stats.json"
    if clean_glb.exists() and hires_glb.exists():
        print(f"cleanup: skip (exists) -> {clean_glb}")
    else:
        raw_glb = cand_dir / "raw.glb"
        cmd = [BLENDER, "--background", "--python", PROP_CLEANUP, "--", raw_glb, clean_glb,
               "--height", height_m, "--asset", asset, "--tri-budget", tri_budget]
        if symmetrize:
            cmd += ["--symmetrize", f"--symmetrize-keep={symmetrize_keep}"]
        out = run_capture(cmd)
        meta_path.write_text(json.dumps(last_json_line(out), indent=2), encoding="utf-8")
        print(f"cleanup: generated -> {clean_glb}")
    meta = read_or_note(meta_path)
    meta["clean_glb_sha256"] = sha256_file(clean_glb)
    meta["clean_hires_glb_sha256"] = sha256_file(hires_glb)
    return meta


def stage_texture(cand_dir: Path, asset: str, seed: int) -> dict:
    textured_glb = cand_dir / "textured.glb"
    meta_path = cand_dir / "texture_stats.json"
    if textured_glb.exists():
        print(f"texture: skip (exists) -> {textured_glb}")
    else:
        clean_glb = cand_dir / "clean.glb"
        hires_glb = cand_dir / "clean_hires.glb"
        cmd = [BLENDER, "--background", "--python", PROP_TEXTURE, "--",
               clean_glb, hires_glb, textured_glb,
               "--asset", asset, "--seed", seed]
        out = run_capture(cmd)
        meta_path.write_text(json.dumps(last_json_line(out), indent=2), encoding="utf-8")
        print(f"texture: generated -> {textured_glb}")
    meta = read_or_note(meta_path)
    meta["textured_glb_sha256"] = sha256_file(textured_glb)
    return meta


def stage_preprocess_bake(cand_dir: Path, texture_size: int, max_bytes: int = None) -> dict:
    textured_glb = cand_dir / "textured.glb"
    final_glb = cand_dir / "final.glb"
    bake_manifest = cand_dir / "final.textures" / "manifest.json"
    meta_path = cand_dir / "preprocess_stats.json"

    if final_glb.exists():
        print(f"preprocess: skip (exists) -> {final_glb}")
    else:
        preprocess_stats = {"textured_glb_bytes": textured_glb.stat().st_size}
        cmd = ["node", PREPROCESS_PROP_MJS, textured_glb, final_glb, "--max-dim", texture_size]
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

    parser = argparse.ArgumentParser(description="Generate one prop candidate per seed through the full A3 chain.")
    parser.add_argument("--asset", required=True,
                        help="Registered asset name (content/models/assets.json); resolves "
                             "the subject prompt and material contract (proptex.registry)")
    parser.add_argument("--out", type=Path, required=True, help="Batch directory; each candidate lands in <out>/cand_<seed>/")
    parser.add_argument("--seed", type=int, action="append", dest="seeds", required=True, metavar="N",
                        help="Repeatable: one candidate per seed; candidates sharing a concept image "
                             "run their geometry stage in a single Hi3DGen process")
    parser.add_argument("--skip-concept", type=Path, default=None, metavar="IMAGE",
                        help="Bypass concept generation with a provided image (re-roll geometry without re-rolling the concept)")
    parser.add_argument("--symmetrize", action="store_true",
                        help="Mirror one half of the cleaned mesh across its best-fit vertical plane")
    parser.add_argument("--symmetrize-keep", choices=["+x", "-x"], default="+x",
                        help="Half to mirror in prop_cleanup.py's plane-aligned frame")
    parser.add_argument("--max-bytes", type=int, default=None,
                        help="preprocess_prop.mjs's final.glb size assert (default 32 MB, "
                             "the prop cap)")
    parser.add_argument("--through", choices=STAGES, default="turntable",
                        help="Stop after this stage (batch triage: --through cleanup "
                             "sweeps geometry seeds without paying for texturing)")
    args = parser.parse_args()

    # Resolve once here: stage_turntable runs its subprocess under
    # cwd=REPO_ROOT, so a relative --out would otherwise resolve against
    # the wrong directory.
    args.out = args.out.resolve()
    contract = resolve(args.asset)

    if len(set(args.seeds)) != len(args.seeds):
        parser.error(f"repeated --seed value in {args.seeds}: two candidates would share one cand_<seed>/ directory")
    cand_dirs = {seed: args.out / f"cand_{seed}" for seed in args.seeds}
    for cand_dir in cand_dirs.values():
        cand_dir.mkdir(parents=True, exist_ok=True)

    stop = STAGES.index(args.through)
    manifests = {}
    for seed in args.seeds:
        manifests[seed] = {
            "subject": contract.subject,
            "seed": seed,
            "candidate_dir": str(cand_dirs[seed]),
            "concept": stage_concept(cand_dirs[seed], contract.subject, seed, args.skip_concept),
        }
    if stop >= STAGES.index("geometry"):
        concept_sha256 = {seed: manifests[seed]["concept"]["concept_png_sha256"] for seed in args.seeds}
        for seed, meta in stage_geometry(args.out, cand_dirs, concept_sha256).items():
            manifests[seed]["geometry"] = meta

    for seed in args.seeds:
        cand_dir = cand_dirs[seed]
        manifest = manifests[seed]
        if stop >= STAGES.index("cleanup"):
            manifest["cleanup"] = stage_cleanup(cand_dir, args.asset, contract.height_m, contract.tri_budget,
                                                args.symmetrize, args.symmetrize_keep)
        if stop >= STAGES.index("texture"):
            manifest["texture"] = stage_texture(cand_dir, args.asset, seed)
        if stop >= STAGES.index("preprocess"):
            # final.glb, needed by the turntable stage below
            manifest.update(stage_preprocess_bake(cand_dir, contract.texture_size, args.max_bytes))
        if stop >= STAGES.index("turntable"):
            manifest["turntable"] = stage_turntable(cand_dir)
        manifest_path = cand_dir / "generation_manifest.json"
        manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        print(f"OK: wrote {manifest_path}")


if __name__ == "__main__":
    main()
