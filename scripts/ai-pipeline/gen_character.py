#!/usr/bin/env python3
"""Character generation chain assembly (Phase A4.8): concept -> geometry ->
cleanup -> texture -> rig+clips -> preprocess+bake -> review renders ->
chained manifest. One invocation = one candidate under <out>/cand_<seed>/.
Plain system Python -- this script only subprocess-orchestrates the
per-stage tools, each of which runs under its own venv/interpreter
(Hi3DGen venv, Blender, node, cargo). gen_prop.py's skeleton (A3.9),
extended with the two character-only stages: rig transplant (A4.4) and
multi-clip review renders (A4.5).

Every stage is skipped if its output already exists, so re-running the same
command resumes rather than restarts. Any stage's non-zero exit aborts the
whole chain with that stage named -- no silent fallbacks. The one stage
whose failure carries extra context is rig: a rig-quality-gate failure is a
recorded candidate outcome (A4.4), so the abort message includes the gate's
stats alongside the exit code, not just the stage name.

The character texture strategy is fixed by the A4.6 ruling (multiview
ControlNet-depth) -- not exposed as a CLI flag, unlike gen_prop.py's
--texture-strategy (props still default to projection). MR takes
prop_texture's non-metal defaults, which are the character contract.

Run:
  python scripts/ai-pipeline/gen_character.py "<subject prompt>" --out <dir> --seed N [--skip-concept <image.png>] [--height M]

ComfyUI server lifecycle is the CALLER's job for the concept stage only
(gen_prop.py's convention): it must already be running before this script
starts. Geometry (Hi3DGen) must never run while it's up (11.4-11.5 GiB
peak, A3.4/A4.3-measured). The multiview texture stage starts/stops its own
server strictly after geometry, so the VRAM rule holds. Rig, preprocess,
bake, and review-render stages are CPU/offscreen-GPU only -- no VRAM
sequencing concern.
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent

COMFY_RUN = SCRIPT_DIR / "comfy_run.py"
CHAR_CONCEPT_WORKFLOW = SCRIPT_DIR / "workflows" / "char_concept.json"
PROP_HI3DGEN = SCRIPT_DIR / "prop_hi3dgen.py"
PROP_CLEANUP = SCRIPT_DIR / "prop_cleanup.py"
PROP_TEXTURE = SCRIPT_DIR / "prop_texture.py"
CHAR_RIG = SCRIPT_DIR / "char_rig.py"
PREPROCESS_PROP_MJS = SCRIPT_DIR / "preprocess_prop.mjs"
BAKE_TEXTURES_MJS = REPO_ROOT / "scripts" / "asset-pipeline" / "bake_textures.mjs"

HI3DGEN_PYTHON = Path(r"C:\tools\Hi3DGen\venv\Scripts\python.exe")
HI3DGEN_REPO = Path(r"C:\tools\Hi3DGen\Hi3DGen")
BLENDER = Path(r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe")

MIXAMO_DIR = REPO_ROOT / "content" / "source" / "characters" / "mixamo"
CHARACTER_FBX = MIXAMO_DIR / "Character.fbx"
CLIPS_DIR = MIXAMO_DIR / "clips"

# Must match scripts/asset-pipeline/mixamo_rig.py's TARGET_HEIGHT (the
# VRoid body's authored height); duplicated here because this script runs
# under plain system Python and mixamo_rig.py imports bpy.
TARGET_HEIGHT = 1.744
TRI_BUDGET = 30000
# VQ-B2's character file cap (docs/visual-quality.md), vs the 8 MB prop
# default preprocess_prop.mjs otherwise applies.
MAX_BYTES = 16 * 1024 * 1024

TURNTABLE_ANGLES = 8
TURNTABLE_SIZE = "512x512"
REVIEW_CLIPS = ["walk", "idle", "attack_slash"]


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(cmd: list, cwd: Path = None) -> None:
    """Stream a subprocess to the console; abort the chain on non-zero exit."""
    proc = subprocess.run([str(c) for c in cmd], cwd=str(cwd) if cwd else None)
    if proc.returncode != 0:
        sys.exit(f"gen_character: command failed (exit {proc.returncode}): {' '.join(str(c) for c in cmd)}")


def run_capture(cmd: list, cwd: Path = None) -> str:
    """Like run(), but captures stdout (for stages that print a JSON stats
    line rather than writing their own manifest file) and echoes both
    streams so the run stays visible."""
    proc = subprocess.run([str(c) for c in cmd], cwd=str(cwd) if cwd else None,
                          capture_output=True, text=True, encoding="utf-8", errors="replace")
    sys.stdout.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        sys.exit(f"gen_character: command failed (exit {proc.returncode}): {' '.join(str(c) for c in cmd)}")
    return proc.stdout


def last_json_line(text: str) -> dict:
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("{"):
            return json.loads(line)
    sys.exit("gen_character: expected a JSON stats line in stage output, found none")


def read_or_note(meta_path: Path) -> dict:
    """Stage provenance sidecar, or an honest placeholder if the stage's
    output pre-existed this script's involvement (e.g. seeded from another
    tool's run) and so was never captured."""
    if meta_path.exists():
        return json.loads(meta_path.read_text(encoding="utf-8"))
    return {"note": f"no stats captured ({meta_path.name} not found; output pre-existed this gen_character.py run)"}


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
        # A3.7 convention (char_concept.json ships the same {subject}
        # placeholder): str-replace and submit a candidate-scoped copy
        # (comfy_run.py has no prompt override CLI).
        workflow = json.loads(CHAR_CONCEPT_WORKFLOW.read_text(encoding="utf-8").replace("{subject}", subject))
        for node in workflow.values():
            inputs = node.get("inputs", {})
            for key in inputs:
                if key in ("seed", "noise_seed"):
                    inputs[key] = seed
        workflow_copy = cand_dir / "concept_workflow.json"
        workflow_copy.write_text(json.dumps(workflow, indent=2), encoding="utf-8")

        concept_raw = cand_dir / "concept_raw"
        run([sys.executable, COMFY_RUN, workflow_copy, "--out", concept_raw])
        pngs = sorted(concept_raw.glob("*.png"))
        if len(pngs) != 1:
            sys.exit(f"gen_character: concept stage produced {len(pngs)} PNG(s) in {concept_raw}, expected exactly 1")
        shutil.copyfile(pngs[0], concept_png)
        meta = {
            "mode": "generated",
            "subject": subject,
            "workflow_copy": workflow_copy.name,
            "comfy_manifest": json.loads((concept_raw / "manifest.json").read_text(encoding="utf-8")),
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
        # Use absolute paths since subprocess runs with cwd=HI3DGEN_REPO
        run([HI3DGEN_PYTHON, PROP_HI3DGEN, concept_png.resolve(), "--out", cand_dir.resolve(), "--seed", seed], cwd=HI3DGEN_REPO)
        # prop_hi3dgen.py writes <out>/generation_manifest.json -- move it
        # aside immediately so that filename stays reserved for this script's
        # own chained manifest (the final stage); otherwise the aggregate
        # step would overwrite this stage's provenance and re-runs couldn't
        # recover it.
        (cand_dir / "generation_manifest.json").replace(hi3dgen_manifest_path)
        print(f"geometry: generated -> {raw_glb}")
    meta = read_or_note(hi3dgen_manifest_path)
    meta["raw_glb_sha256"] = sha256_file(raw_glb)
    meta["concept_rgba_sha256"] = sha256_file(concept_rgba)
    return meta


def stage_cleanup(cand_dir: Path, height: float) -> dict:
    clean_glb = cand_dir / "clean.glb"
    hires_glb = cand_dir / "clean_hires.glb"
    meta_path = cand_dir / "cleanup_stats.json"
    if clean_glb.exists() and hires_glb.exists():
        print(f"cleanup: skip (exists) -> {clean_glb}")
    else:
        raw_glb = cand_dir / "raw.glb"
        out = run_capture([BLENDER, "--background", "--python", PROP_CLEANUP, "--",
                           raw_glb, clean_glb, "--height", height, "--tri-budget", TRI_BUDGET])
        meta_path.write_text(json.dumps(last_json_line(out), indent=2), encoding="utf-8")
        print(f"cleanup: generated -> {clean_glb}")
    meta = read_or_note(meta_path)
    meta["clean_glb_sha256"] = sha256_file(clean_glb)
    meta["clean_hires_glb_sha256"] = sha256_file(hires_glb)
    return meta


def stage_texture(cand_dir: Path, subject: str, seed: int) -> dict:
    textured_glb = cand_dir / "textured.glb"
    meta_path = cand_dir / "texture_stats.json"
    if textured_glb.exists():
        print(f"texture: skip (exists) -> {textured_glb}")
    else:
        clean_glb = cand_dir / "clean.glb"
        hires_glb = cand_dir / "clean_hires.glb"
        concept_rgba = cand_dir / "concept_rgba.png"
        # Strategy is the A4.6 ruling, not CLI-selectable here: multiview,
        # because projection mirrors the face onto the hood's back. MR takes
        # prop_texture's non-metal defaults, which are the character contract.
        cmd = [BLENDER, "--background", "--python", PROP_TEXTURE, "--",
               clean_glb, hires_glb, concept_rgba, textured_glb,
               "--strategy", "multiview", "--subject", subject, "--seed", seed]
        out = run_capture(cmd)
        meta_path.write_text(json.dumps(last_json_line(out), indent=2), encoding="utf-8")
        print(f"texture: generated -> {textured_glb}")
    meta = read_or_note(meta_path)
    meta["textured_glb_sha256"] = sha256_file(textured_glb)
    return meta


def stage_rig(cand_dir: Path, height: float) -> dict:
    textured_glb = cand_dir / "textured.glb"
    rigged_glb = cand_dir / "rigged.glb"
    meta_path = cand_dir / "rig_stats.json"
    if rigged_glb.exists():
        print(f"rig: skip (exists) -> {rigged_glb}")
    else:
        cmd = [BLENDER, "--background", "--python", CHAR_RIG, "--",
               textured_glb, CHARACTER_FBX, CLIPS_DIR, rigged_glb, "--height", height]
        proc = subprocess.run([str(c) for c in cmd], capture_output=True, text=True,
                              encoding="utf-8", errors="replace")
        sys.stdout.write(proc.stdout)
        sys.stderr.write(proc.stderr)
        stats = None
        for line in proc.stdout.splitlines():
            line = line.strip()
            if line.startswith("{"):
                stats = json.loads(line)
        if stats is not None:
            meta_path.write_text(json.dumps(stats, indent=2), encoding="utf-8")
        if proc.returncode != 0:
            # A rig-gate failure is a recorded candidate outcome (A4.4), not
            # a bug to patch around -- carry its stats in the abort reason
            # instead of just naming the stage.
            reason = f"rig-quality gate stats: {json.dumps(stats)}" if stats is not None \
                else "no stats line captured"
            sys.exit(f"gen_character: stage 'rig' failed (exit {proc.returncode}) -- {reason}")
        print(f"rig: generated -> {rigged_glb}")
    meta = read_or_note(meta_path)
    meta["rigged_glb_sha256"] = sha256_file(rigged_glb)
    return meta


def stage_preprocess_bake(cand_dir: Path) -> dict:
    rigged_glb = cand_dir / "rigged.glb"
    final_glb = cand_dir / "final.glb"
    bake_manifest = cand_dir / "final.textures" / "manifest.json"
    meta_path = cand_dir / "preprocess_stats.json"

    if final_glb.exists():
        print(f"preprocess: skip (exists) -> {final_glb}")
    else:
        preprocess_stats = {"rigged_glb_bytes": rigged_glb.stat().st_size}
        run(["node", PREPROCESS_PROP_MJS, rigged_glb, final_glb, "--max-bytes", MAX_BYTES])
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


def stage_review(cand_dir: Path) -> dict:
    final_glb = cand_dir / "final.glb"
    renders = {}

    static_dir = cand_dir / "turntable_static"
    if (static_dir / "contact_sheet.png").exists():
        print(f"review(static): skip (exists) -> {static_dir}")
    else:
        # Use absolute paths since subprocess runs with cwd=REPO_ROOT
        run(["cargo", "run", "-p", "engine-renderer", "--release", "--features", "offscreen", "--bin", "turntable",
             "--", final_glb.resolve(), "--out", static_dir.resolve(), "--angles", str(TURNTABLE_ANGLES), "--size", TURNTABLE_SIZE],
            cwd=REPO_ROOT)
        print(f"review(static): generated -> {static_dir}")
    renders["static"] = {"angles": TURNTABLE_ANGLES, "size": TURNTABLE_SIZE,
                         "contact_sheet": str((static_dir / "contact_sheet.png").relative_to(cand_dir))}

    for clip in REVIEW_CLIPS:
        clip_dir = cand_dir / f"turntable_{clip}"
        if (clip_dir / "contact_sheet.png").exists():
            print(f"review({clip}): skip (exists) -> {clip_dir}")
        else:
            # Use absolute paths since subprocess runs with cwd=REPO_ROOT
            run(["cargo", "run", "-p", "engine-renderer", "--release", "--features", "offscreen", "--bin", "turntable",
                 "--", final_glb.resolve(), "--out", clip_dir.resolve(), "--clip", clip, "--size", TURNTABLE_SIZE],
                cwd=REPO_ROOT)
            print(f"review({clip}): generated -> {clip_dir}")
        renders[clip] = {"size": TURNTABLE_SIZE,
                         "contact_sheet": str((clip_dir / "contact_sheet.png").relative_to(cand_dir))}

    return renders


def main():
    # Line-buffer stdout: without this, CPython fully buffers stdout when it
    # isn't a tty, so this script's own stage-progress prints all land at
    # process exit instead of interleaving live with each subprocess's
    # inherited-fd output -- misleading during the multi-minute real runs
    # this same chain drives in A4.9.
    sys.stdout.reconfigure(line_buffering=True)

    parser = argparse.ArgumentParser(description="Generate one character candidate through the full A4 chain.")
    parser.add_argument("subject", help="Character description substituted into char_concept.json's {subject}")
    parser.add_argument("--out", type=Path, required=True, help="Batch directory; the candidate lands in <out>/cand_<seed>/")
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--skip-concept", type=Path, default=None, metavar="IMAGE",
                        help="Bypass concept generation with a provided image (re-roll geometry without re-rolling the concept)")
    parser.add_argument("--height", type=float, default=TARGET_HEIGHT,
                        help="Target character height in metres, applied to both the cleanup and rig stages")
    args = parser.parse_args()

    cand_dir = args.out / f"cand_{args.seed}"
    cand_dir.mkdir(parents=True, exist_ok=True)

    concept = stage_concept(cand_dir, args.subject, args.seed, args.skip_concept)
    geometry = stage_geometry(cand_dir, args.seed)
    cleanup = stage_cleanup(cand_dir, args.height)
    texture = stage_texture(cand_dir, args.subject, args.seed)
    rig = stage_rig(cand_dir, args.height)
    preprocess_bake = stage_preprocess_bake(cand_dir)  # final.glb, needed by the review stage below
    review = stage_review(cand_dir)

    manifest = {
        "subject": args.subject,
        "seed": args.seed,
        "height": args.height,
        "candidate_dir": str(cand_dir),
        "concept": concept,
        "geometry": geometry,
        "cleanup": cleanup,
        "texture": texture,
        "rig": rig,
        **preprocess_bake,
        "review": review,
    }
    manifest_path = cand_dir / "generation_manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"OK: wrote {manifest_path}")


if __name__ == "__main__":
    main()
