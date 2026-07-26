#!/usr/bin/env python3
"""Promotes a pipeline-built prop glb into content/models/props/<name>/: the
single path from a candidate's output to a shipped asset, replacing the
manual copy that let six props ship a stale material contract.

Every step refuses rather than degrades: an unknown asset/class, a source
glb with no sibling generation_manifest.json, a manifest whose texture
section carries no cache chain record, a glb whose bytes don't match its
own export record, or an export record built under a superseded class
contract all stop the install before anything is written. Provenance is
never fabricated and never repaired here -- a glb this command cannot bind
to the run that produced it, or whose contract has since moved, is refused
by name rather than silently carried forward.

install never changes the bytes it installs: the copy is byte-faithful and
nothing between the copy and the final manifest write mutates the glb, so
final_glb_sha256 -- computed last, from the copied file -- equals both the
source glb's hash and the export stage's own recorded output hash.

Run:
  python scripts/ai-pipeline/install_asset.py <built.glb> --asset NAME [--dry-run]
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
from proptex.registry import RegistryError, resolve  # noqa: E402

REPO_ROOT = SCRIPT_DIR.parent.parent
PROPS_DIR = REPO_ROOT / "content" / "models" / "props"
BAKE_TEXTURES_MJS = REPO_ROOT / "scripts" / "asset-pipeline" / "bake_textures.mjs"

TOLERANCE = 1e-6  # float round-trip through JSON; matches content_lint.rs's own tolerance


class InstallError(Exception):
    pass


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(cmd: list, cwd: Path = None) -> None:
    try:
        proc = subprocess.run([str(c) for c in cmd], cwd=str(cwd) if cwd else None)
    except FileNotFoundError as e:
        raise InstallError(f"command not found: {' '.join(str(c) for c in cmd)} ({e})") from e
    if proc.returncode != 0:
        raise InstallError(f"command failed (exit {proc.returncode}): {' '.join(str(c) for c in cmd)}")


def read_chain_record(name: str, source_glb: Path) -> dict:
    """The source manifest sitting beside the glb being installed, refusing
    if it is absent or its texture section carries no cache chain -- the
    shape prop_texture.py's `provenance.chain()` writes (a `stages` list).
    An older manifest's `texture` section (a flat strategy/views/
    pbr_estimator record) fails this the same way an absent file does."""
    manifest_path = source_glb.parent / "generation_manifest.json"
    if not manifest_path.is_file():
        raise InstallError(
            f"asset '{name}': no generation_manifest.json beside {source_glb} -- "
            "install_asset does not synthesise provenance"
        )
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    texture = manifest.get("texture")
    if not isinstance(texture, dict) or not isinstance(texture.get("stages"), list):
        raise InstallError(
            f"asset '{name}': {manifest_path} carries no chain record "
            "(texture.stages) -- install_asset does not synthesise provenance"
        )
    return manifest


def find_export_record(name: str, stages: list) -> dict:
    exports = [s for s in stages if s.get("stage") == "export"]
    if len(exports) != 1:
        raise InstallError(
            f"asset '{name}': texture.stages has {len(exports)} 'export' record(s), expected exactly 1"
        )
    return exports[0]


def verify_export_record(name: str, source_glb: Path, export_record: dict, contract) -> None:
    """Binds the glb being installed to the cache record that describes it
    -- the guarantee a target/prop-cache/ reverse lookup would give, built
    instead from data the chain record already carries (cache.py writes
    outputs["export:textured.glb"], provenance.py carries params verbatim).
    A hash mismatch means the record describes bytes that exist nowhere; a
    params mismatch means the glb was built under a surface_classes.json
    that has since moved -- the export stage's own cache key already made
    that detectable, so this refuses rather than stamping over it."""
    want_hash = sha256_file(source_glb)
    got_hash = export_record.get("outputs", {}).get("export:textured.glb")
    if got_hash is None:
        raise InstallError(f"asset '{name}': export record has no 'export:textured.glb' output")
    if want_hash != got_hash:
        raise InstallError(
            f"asset '{name}': {source_glb} sha256 {want_hash} does not match its export "
            f"record's output hash {got_hash} -- the glb the record describes exists nowhere"
        )

    params = export_record.get("params", {})
    mismatches = []
    for field in ("metallic", "roughness"):
        want = getattr(contract, field)
        got = params.get(field)
        if got is None or abs(got - want) > TOLERANCE:
            mismatches.append(f"{field} record={got!r} class={want!r}")
    if params.get("detail") != contract.detail:
        mismatches.append(f"detail record={params.get('detail')!r} class={contract.detail!r}")
    if mismatches:
        raise InstallError(
            f"asset '{name}': export record was built under a superseded class contract "
            f"({'; '.join(mismatches)}) -- rebuild required, install does not repair a stale build"
        )


class Ctx:
    pass


def build_steps(ctx):
    def do_copy(c):
        c.dest_dir.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(c.source_glb, c.dest_glb)

    def do_bake(c):
        run(["node", BAKE_TEXTURES_MJS, "gltf", c.dest_glb])

    def do_write_manifest(c):
        bake_manifest_path = c.dest_dir / f"{c.name}.textures" / "manifest.json"
        manifest = dict(c.source_manifest)
        manifest["bake"] = json.loads(bake_manifest_path.read_text(encoding="utf-8"))
        manifest["final_glb_sha256"] = sha256_file(c.dest_glb)
        (c.dest_dir / "generation_manifest.json").write_text(
            json.dumps(manifest, indent=2), encoding="utf-8"
        )

    def do_lint(c):
        run(["cargo", "nextest", "run", "-p", "vordar-game", "--test", "content_lint",
             "prop_material_matches_surface_class"], cwd=REPO_ROOT)

    return [
        (f"copy {ctx.source_glb} -> {ctx.dest_glb} (byte-faithful; provenance already verified)", do_copy),
        (f"bake sidecars: node {BAKE_TEXTURES_MJS} gltf {ctx.dest_glb}", do_bake),
        (f"write {ctx.dest_dir / 'generation_manifest.json'} (final_glb_sha256 computed here, "
         "after the bake above)", do_write_manifest),
        ("run lint clause: cargo nextest run -p vordar-game --test content_lint "
         "prop_material_matches_surface_class", do_lint),
    ]


def install_asset(name: str, source_glb: Path, dry_run: bool) -> None:
    try:
        contract = resolve(name)
    except RegistryError as e:
        raise InstallError(f"asset '{name}': {e}") from e
    print(f"resolve '{name}' -> surface_class '{contract.surface_class}' "
          f"(metallic={contract.metallic}, roughness={contract.roughness}, detail={contract.detail})")

    source_manifest = read_chain_record(name, source_glb)
    stages = source_manifest["texture"]["stages"]
    print(f"read chain record: {len(stages)} stage(s) from "
          f"{source_glb.parent / 'generation_manifest.json'}")

    export_record = find_export_record(name, stages)
    verify_export_record(name, source_glb, export_record, contract)
    print(f"verify export record: {source_glb} sha256 matches its export:textured.glb output, "
          f"params (metallic={contract.metallic}, roughness={contract.roughness}, "
          f"detail={contract.detail}) match class '{contract.surface_class}'")

    ctx = Ctx()
    ctx.name = name
    ctx.contract = contract
    ctx.source_glb = source_glb
    ctx.source_manifest = source_manifest
    ctx.dest_dir = PROPS_DIR / name
    ctx.dest_glb = ctx.dest_dir / f"{name}.glb"

    for i, (label, action) in enumerate(build_steps(ctx), start=1):
        prefix = "[dry-run] " if dry_run else ""
        print(f"{prefix}{i}. {label}")
        if not dry_run:
            action(ctx)

    if dry_run:
        print("[dry-run] no files written")
    else:
        print(f"OK: installed '{name}' -> {ctx.dest_glb}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("source_glb", type=Path, help="The built glb to install")
    parser.add_argument("--asset", required=True, dest="name",
                        help="Registered asset name (content/models/assets.json)")
    parser.add_argument("--dry-run", action="store_true",
                        help="Print the ordered plan without writing or running anything")
    args = parser.parse_args()

    try:
        install_asset(args.name, args.source_glb.resolve(), args.dry_run)
    except InstallError as e:
        print(f"install_asset: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
