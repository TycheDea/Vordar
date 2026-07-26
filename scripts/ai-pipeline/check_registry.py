"""Validate content/models/surface_classes.json and assets.json against each
other and against content/models/props/ on disk. Exits non-zero on any
failure, printing every failure found (not just the first)."""
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
MODELS_DIR = REPO_ROOT / "content" / "models"
PROPS_DIR = MODELS_DIR / "props"

EXPECTED_CLASSES = {"limestone", "wood", "foliage", "painted_metal", "character_skin"}
CLASS_FIELDS = {"metallic", "roughness", "albedo_source", "detail"}
GENERATED_FIELDS = {"subject", "texture_size", "view_res"}


def main() -> int:
    errors = []

    surface_classes = json.loads((MODELS_DIR / "surface_classes.json").read_text(encoding="utf-8"))
    assets = json.loads((MODELS_DIR / "assets.json").read_text(encoding="utf-8"))

    # 1. Exact class set.
    class_names = set(surface_classes.keys())
    if class_names != EXPECTED_CLASSES:
        errors.append(
            f"surface_classes.json class set mismatch: got {sorted(class_names)}, "
            f"expected {sorted(EXPECTED_CLASSES)}"
        )

    # 2. Each class has exactly the four fields, with correct types/values.
    for name, fields in surface_classes.items():
        keys = set(fields.keys())
        if keys != CLASS_FIELDS:
            errors.append(f"surface_classes.json[{name!r}] has keys {sorted(keys)}, expected {sorted(CLASS_FIELDS)}")
        if fields.get("metallic") != 0.0:
            errors.append(f"surface_classes.json[{name!r}].metallic = {fields.get('metallic')!r}, expected 0.0")
        if fields.get("albedo_source") not in ("direct", "delit"):
            errors.append(
                f"surface_classes.json[{name!r}].albedo_source = {fields.get('albedo_source')!r}, "
                "expected 'direct' or 'delit'"
            )
        if not isinstance(fields.get("detail"), bool):
            errors.append(f"surface_classes.json[{name!r}].detail = {fields.get('detail')!r}, expected a bool")

    # 3. Every assets.json surface_class resolves into surface_classes.json.
    for asset_name, entry in assets.items():
        sc = entry.get("surface_class")
        if sc not in surface_classes:
            errors.append(f"assets.json[{asset_name!r}].surface_class = {sc!r} not found in surface_classes.json")

    # 4. Directories <-> assets.json entries, bijective.
    disk_dirs = {p.name for p in PROPS_DIR.iterdir() if p.is_dir()} if PROPS_DIR.is_dir() else set()
    asset_names = set(assets.keys())
    missing_entries = disk_dirs - asset_names
    missing_dirs = asset_names - disk_dirs
    if missing_entries:
        errors.append(f"directories under content/models/props/ with no assets.json entry: {sorted(missing_entries)}")
    if missing_dirs:
        errors.append(f"assets.json entries with no directory under content/models/props/: {sorted(missing_dirs)}")

    # 5. generated entries carry subject/texture_size/view_res; downloaded entries carry none of them.
    for asset_name, entry in assets.items():
        kind = entry.get("kind")
        present = GENERATED_FIELDS & entry.keys()
        if kind == "generated":
            missing = GENERATED_FIELDS - entry.keys()
            if missing:
                errors.append(f"assets.json[{asset_name!r}] (generated) missing fields: {sorted(missing)}")
        elif kind == "downloaded":
            if present:
                errors.append(f"assets.json[{asset_name!r}] (downloaded) must not carry {sorted(present)}")
        else:
            errors.append(f"assets.json[{asset_name!r}].kind = {kind!r}, expected 'generated' or 'downloaded'")

    if errors:
        print(f"FAIL: {len(errors)} error(s)")
        for e in errors:
            print(f"  - {e}")
        return 1

    print("OK: surface_classes.json and assets.json are consistent")
    return 0


if __name__ == "__main__":
    sys.exit(main())
