"""Resolves an asset name to its full material/generation contract.

Reads content/models/surface_classes.json (per-surface-class fields) and
content/models/assets.json (per-asset fields), merges them, and refuses on
any unknown name, unknown class, or missing field. No field is ever defaulted
or substituted except `azimuths`, whose [0, 90, 180, 270] default lives here
and nowhere else.
"""

import json
from dataclasses import dataclass
from pathlib import Path
from typing import List, Union

_REPO_ROOT = Path(__file__).resolve().parents[3]
_CLASSES_PATH = _REPO_ROOT / "content" / "models" / "surface_classes.json"
_ASSETS_PATH = _REPO_ROOT / "content" / "models" / "assets.json"

_DEFAULT_AZIMUTHS = [0, 90, 180, 270]
_ASSET_FIELDS = ("kind", "surface_class")
_CLASS_FIELDS = ("metallic", "roughness", "albedo_source", "detail")
_GENERATED_FIELDS = ("subject", "texture_size", "view_res", "height_m", "tri_budget")


class RegistryError(Exception):
    pass


def resolve_class(class_name: str) -> dict:
    """Surface-class fields without an asset instance -- for callers that
    build a fixed body (char_mpfb.py's MPFB parametric character) rather
    than texture a generated asset."""
    classes = json.loads(_CLASSES_PATH.read_text())
    if class_name not in classes:
        raise RegistryError(f"unknown surface class {class_name!r}")
    surface_class = classes[class_name]
    for field_name in _CLASS_FIELDS:
        if field_name not in surface_class:
            raise RegistryError(
                f"surface class {class_name!r} has no {field_name!r} field"
            )
    return {field_name: surface_class[field_name] for field_name in _CLASS_FIELDS}


@dataclass(frozen=True)
class Contract:
    name: str
    kind: str
    surface_class: str
    metallic: float
    roughness: float
    albedo_source: str
    detail: bool


@dataclass(frozen=True)
class GeneratedContract(Contract):
    subject: str
    texture_size: int
    view_res: int
    height_m: float
    tri_budget: int
    azimuths: List[int]


def resolve(name: str) -> Union[Contract, GeneratedContract]:
    assets = json.loads(_ASSETS_PATH.read_text())
    if name not in assets:
        raise RegistryError(f"unknown asset {name!r}")
    asset = assets[name]
    for field_name in _ASSET_FIELDS:
        if field_name not in asset:
            raise RegistryError(f"asset {name!r} has no {field_name!r} field")
    if asset["kind"] not in ("generated", "downloaded"):
        raise RegistryError(f"asset {name!r} has unknown kind {asset['kind']!r}")

    class_name = asset["surface_class"]
    try:
        class_kwargs = resolve_class(class_name)
    except RegistryError as e:
        raise RegistryError(f"asset {name!r}: {e}") from e

    common = dict(name=name, kind=asset["kind"], surface_class=class_name, **class_kwargs)
    if asset["kind"] == "downloaded":
        return Contract(**common)

    for field_name in _GENERATED_FIELDS:
        if field_name not in asset:
            raise RegistryError(f"asset {name!r} has no {field_name!r} field")

    return GeneratedContract(
        **common,
        subject=asset["subject"],
        texture_size=asset["texture_size"],
        view_res=asset["view_res"],
        height_m=asset["height_m"],
        tri_budget=asset["tri_budget"],
        azimuths=list(asset.get("azimuths", _DEFAULT_AZIMUTHS)),
    )
