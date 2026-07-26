"""Content-addressed stage cache for the texture pipeline: a stage's version
is the sha256 of the transitive intra-tree import closure of its own source
(derived, not declared, so an edit anywhere in that closure invalidates the
right stages and nowhere else), `stage_key` folds that version with resolved
params and input hashes into one key, and `cached` stores/reads
`target/prop-cache/<stage>/<key>/` atomically. Stdlib-only, no bpy/cv2/numpy,
so it imports and runs under a plain interpreter, like provenance.py.
"""

import ast
import hashlib
import json
import os
import shutil
import tempfile
import time
from collections import namedtuple
from pathlib import Path
from types import ModuleType

from proptex.provenance import stage_record

CACHE_ROOT = Path(__file__).resolve().parents[3] / "target" / "prop-cache"

CacheResult = namedtuple("CacheResult", "dir record hit")


class CacheError(Exception):
    pass


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def sha256_file(path):
    return sha256_bytes(Path(path).read_bytes())


def canonical_json(obj):
    """Structurally-equal objects always produce identical bytes: sorted
    keys, no incidental whitespace, no ASCII-escaping drift."""
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def _find_root(start_dir):
    """The directory `proptex.*` and bare top-level imports resolve against,
    found structurally by locating the proptex package above start_dir --
    so it matches whatever tree entry_module actually lives under, including
    a copy made to probe cache invalidation without touching the original."""
    d = start_dir
    while not (d / "proptex" / "__init__.py").is_file():
        if d.parent == d:
            raise CacheError(f"no proptex/ package found above {start_dir}")
        d = d.parent
    return d


def _resolve_import(root, dotted):
    rel = Path(*dotted.split("."))
    for candidate in (root / rel.with_suffix(".py"), root / rel / "__init__.py"):
        if candidate.is_file():
            return candidate
    return None


def _imported_names(path):
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    names = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            names.extend(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            # A relative import names a file this walk cannot resolve, which
            # would drop it from the closure and leave the version blind to
            # edits in it. The tree uses absolute imports throughout, so
            # refusing costs nothing and cannot go silently stale.
            if node.level:
                raise CacheError(f"{path}: relative import at line {node.lineno}; "
                                 f"the source-set closure resolves absolute imports only")
            if node.module:
                names.append(node.module)
    return names


def source_version(entry_module, extra_files=()):
    """The sha256 over entry_module's transitive intra-tree import closure.
    An import that resolves outside the tree (bpy, cv2, numpy, ...) is
    toolchain identity and rides the params instead. `extra_files` is only for
    a file whose content changes the output across an edge no import expresses
    -- a subprocess launch, which is why prop_pbr.py must be named where the
    estimator stage is keyed. Anything reachable by an import is already in the
    closure, so adding it here would be a transcription that can go stale."""
    entry_file = (Path(entry_module.__file__) if isinstance(entry_module, ModuleType)
                  else Path(entry_module)).resolve()
    root = _find_root(entry_file.parent)

    visited, stack = set(), [entry_file]
    while stack:
        f = stack.pop()
        if f in visited:
            continue
        visited.add(f)
        for name in _imported_names(f):
            resolved = _resolve_import(root, name)
            if resolved is not None and resolved not in visited:
                stack.append(resolved)

    for extra in extra_files:
        p = Path(extra)
        visited.add((p if p.is_absolute() else root / p).resolve())

    pairs = sorted((f.relative_to(root).as_posix(), sha256_file(f)) for f in visited)
    return sha256_bytes(canonical_json(pairs))


def stage_key(stage, version, params, inputs):
    payload = {"stage": stage, "version": version, "params": params, "inputs": inputs}
    return sha256_bytes(canonical_json(payload))


def _entry(stage, source, params, inputs, extra_files, cache_root):
    version = source_version(source, extra_files)
    key = stage_key(stage, version, params, inputs)
    return cache_root / stage / key, version, key


def hits(stage, source, params, inputs, extra_files=(), cache_root=CACHE_ROOT):
    """Whether `cached` would answer from the cache without running
    `produce`. Asked by a stage whose producer needs a resource far more
    expensive than the stage setups entered unconditionally elsewhere -- the
    ComfyUI server -- so that resource is opened only when something misses.
    Takes no unit: the unit is a label on the entry, never part of its key."""
    out_dir, _, _ = _entry(stage, source, params, inputs, extra_files, cache_root)
    return (out_dir / "key.json").is_file()


def outputs_of(unit, *names):
    """The named entries of `unit`'s outputs, as a {key: sha256} dict ready
    to merge into a consumer's `inputs`: a consumer's input key is then
    literally its producer's output key rather than a transcription of one,
    which is what lets the chain be walked producer-to-consumer."""
    by_name = {key.rsplit(":", 1)[1]: key for key in unit.record["outputs"]}
    missing = [name for name in names if name not in by_name]
    if missing:
        raise CacheError(f"{unit.record['stage']}: no output named {missing[0]!r}")
    return {by_name[name]: unit.record["outputs"][by_name[name]] for name in names}


def cached(stage, unit, source, params, inputs, output_names, produce,
           extra_files=(), cache_root=CACHE_ROOT):
    """Content-addressed run: a hit returns the prior record unchanged, so
    elapsed_s always stays the cost of the run that actually produced the
    outputs. A miss runs `produce` into a sibling temp dir and publishes it
    with one atomic rename, so a reader never observes a partially-written
    stage directory."""
    out_dir, version, key = _entry(stage, source, params, inputs,
                                   extra_files, cache_root)
    stage_dir = out_dir.parent
    key_path = out_dir / "key.json"
    if key_path.is_file():
        return CacheResult(out_dir, json.loads(key_path.read_text(encoding="utf-8")), True)

    stage_dir.mkdir(parents=True, exist_ok=True)
    tmp_dir = Path(tempfile.mkdtemp(prefix=f"{key}.", dir=stage_dir))
    t0 = time.time()
    try:
        measurements = produce(tmp_dir) or {}
        missing = [name for name in output_names if not (tmp_dir / name).is_file()]
        if missing:
            raise CacheError(f"{stage}:{unit} did not write declared output {missing[0]!r}")
    except Exception:
        shutil.rmtree(tmp_dir, ignore_errors=True)
        raise
    elapsed_s = round(time.time() - t0, 1)

    # An output key names its own entry, not just its stage, so that a
    # consumer of one of several units of the same stage -- the 41 depth
    # renders one nbv pick consumes -- can merge the producer's key verbatim
    # instead of rebuilding it, and no two of them collide in one dict.
    prefix = stage if unit is None else f"{stage}:{unit}"
    outputs = {f"{prefix}:{name}": sha256_file(tmp_dir / name) for name in output_names}
    record = stage_record(stage, unit, version, params, inputs, key, outputs,
                          elapsed_s, measurements=measurements)
    (tmp_dir / "key.json").write_text(json.dumps(record, indent=2), encoding="utf-8")
    try:
        os.rename(tmp_dir, out_dir)
    except FileExistsError:
        shutil.rmtree(tmp_dir, ignore_errors=True)
        return CacheResult(out_dir, json.loads(key_path.read_text(encoding="utf-8")), True)
    return CacheResult(out_dir, record, False)
