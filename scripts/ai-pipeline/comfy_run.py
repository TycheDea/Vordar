#!/usr/bin/env python3
"""Submit a ComfyUI API-format workflow, wait for it to finish, and pull its
outputs plus a provenance manifest down to a local directory.

run_workflow() (and the CLI) target an already-running server; pipeline
stages that own the server wrap their calls in the server() contextmanager.

Stdlib-only so it runs under either the ComfyUI-bundled embedded Python or a
plain system Python.
"""
import argparse
import contextlib
import json
import random
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

COMFY_URL = "http://127.0.0.1:8188"
COMFY_PYTHON = Path(r"C:\tools\ComfyUI\python_embeded\python.exe")
COMFY_MAIN = Path(r"C:\tools\ComfyUI\ComfyUI\main.py")
COMFY_INPUT_DIR = Path(r"C:\tools\ComfyUI\ComfyUI\input")
DEFAULT_WAIT_TIMEOUT = 300.0
SEED_KEY = re.compile(r"^(seed|noise_seed)$")
MODEL_INPUT_KEY = re.compile(r"_name\d*$", re.IGNORECASE)
MODELS_SHA256 = Path(__file__).resolve().parent / "models.sha256"


def http_json(method, path, payload=None):
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    headers = {"Content-Type": "application/json"} if data is not None else {}
    req = urllib.request.Request(f"{COMFY_URL}{path}", data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        try:
            return json.loads(body)
        except json.JSONDecodeError:
            raise SystemExit(f"{method} {path} failed: HTTP {e.code}: {body}")


def reachable():
    try:
        urllib.request.urlopen(f"{COMFY_URL}/system_stats", timeout=2)
        return True
    except Exception:
        return False


@contextlib.contextmanager
def server():
    """A headless ComfyUI server owned for the duration of the block. An
    external server is refused, not reused: the caller's VRAM sequencing
    (geometry must never run while ComfyUI is up) only holds for a server
    this process controls."""
    if reachable():
        raise SystemExit("a ComfyUI server is already running -- this stage owns the "
                         "server lifecycle (VRAM sequencing); stop the external one first")
    proc = subprocess.Popen(
        [str(COMFY_PYTHON), "-s", str(COMFY_MAIN), "--listen", "127.0.0.1", "--port", "8188"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 120
        while not reachable():
            if proc.poll() is not None:
                raise SystemExit(f"ComfyUI exited during startup (code {proc.returncode})")
            if time.monotonic() >= deadline:
                raise SystemExit("ComfyUI not ready after 120 s")
            time.sleep(1)
        yield
    finally:
        proc.kill()
        proc.wait()


def resolve_seeds(workflow):
    """Replace negative seed/noise_seed sentinels with a concrete random value
    (in place, so the submitted prompt and the manifest agree), and return
    every node's final seed value for the manifest."""
    resolved = {}
    for node_id, node in workflow.items():
        inputs = node.get("inputs", {})
        for key, value in inputs.items():
            if SEED_KEY.match(key) and isinstance(value, int):
                if value < 0:
                    value = random.randint(0, 2**32 - 1)
                    inputs[key] = value
                resolved[node_id] = value
    return resolved


def extract_prompts(workflow):
    prompts = {}
    for node_id, node in workflow.items():
        if "CLIPTextEncode" in node.get("class_type", ""):
            text = node.get("inputs", {}).get("text")
            if isinstance(text, str):
                prompts[node_id] = text
    return prompts


def load_model_hashes(sha256_path):
    hashes = {}
    try:
        lines = sha256_path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return hashes
    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            digest, filename = line.split(None, 1)
        except ValueError:
            continue
        hashes[filename.lstrip("*").strip()] = digest
    return hashes


def extract_models(workflow):
    hashes = load_model_hashes(MODELS_SHA256)
    models = []
    for node_id, node in workflow.items():
        class_type = node.get("class_type", "")
        if "Loader" not in class_type:
            continue
        for key, value in node.get("inputs", {}).items():
            if isinstance(value, str) and MODEL_INPUT_KEY.search(key):
                models.append({
                    "node_id": node_id,
                    "class_type": class_type,
                    "input": key,
                    "filename": value,
                    "sha256": hashes.get(value),
                })
    return models


def submit(workflow):
    result = http_json("POST", "/prompt", {"prompt": workflow})
    if "error" in result:
        raise SystemExit(
            f"ComfyUI rejected the workflow: {json.dumps(result['error'])}\n"
            f"node_errors: {json.dumps(result.get('node_errors', {}))}"
        )
    return result["prompt_id"]


def wait_for_completion(prompt_id, timeout):
    deadline = time.monotonic() + timeout
    while True:
        entry = http_json("GET", f"/history/{prompt_id}").get(prompt_id)
        if entry is not None:
            status = entry.get("status") or {}
            if status.get("status_str") == "error":
                raise SystemExit(f"ComfyUI run failed: {json.dumps(status.get('messages', []))}")
            if status.get("completed"):
                return entry
        if time.monotonic() >= deadline:
            raise SystemExit(f"Timed out after {timeout}s waiting for prompt {prompt_id}")
        time.sleep(1)


def download_outputs(entry, out_dir):
    saved = []
    for node_id, node_output in entry.get("outputs", {}).items():
        for output_kind, items in node_output.items():
            if not isinstance(items, list):
                continue
            for item in items:
                if not isinstance(item, dict) or "filename" not in item:
                    continue
                filename = item["filename"]
                subfolder = item.get("subfolder", "")
                file_type = item.get("type", "output")
                query = urllib.parse.urlencode({"filename": filename, "subfolder": subfolder, "type": file_type})
                dest_dir = out_dir / subfolder if subfolder else out_dir
                dest_dir.mkdir(parents=True, exist_ok=True)
                dest_path = dest_dir / filename
                with urllib.request.urlopen(f"{COMFY_URL}/view?{query}") as resp:
                    dest_path.write_bytes(resp.read())
                saved.append({
                    "node_id": node_id,
                    "output_kind": output_kind,
                    "filename": filename,
                    "subfolder": subfolder,
                    "type": file_type,
                    "saved_as": str(dest_path),
                })
    return saved


def run_workflow(workflow, out_dir, wait_timeout=DEFAULT_WAIT_TIMEOUT):
    """Submit an API-format workflow dict, wait, download outputs into
    out_dir, write out_dir/manifest.json, and return the manifest."""
    seeds = resolve_seeds(workflow)
    prompts = extract_prompts(workflow)
    models = extract_models(workflow)

    try:
        prompt_id = submit(workflow)
        entry = wait_for_completion(prompt_id, wait_timeout)
    except urllib.error.URLError as e:
        raise SystemExit(f"Could not reach ComfyUI at {COMFY_URL}: {e}")

    out_dir.mkdir(parents=True, exist_ok=True)
    outputs = download_outputs(entry, out_dir)

    manifest = {
        "workflow": workflow,
        "prompt_id": prompt_id,
        "prompts": prompts,
        "seed": seeds,
        "models": models,
        "outputs": outputs,
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


def main():
    parser = argparse.ArgumentParser(description="Run a ComfyUI workflow and collect its outputs + provenance manifest.")
    parser.add_argument("workflow", type=Path, help="Path to an API-format ComfyUI workflow JSON")
    parser.add_argument("--out", type=Path, required=True, help="Directory to write outputs and manifest.json into")
    parser.add_argument("--wait-timeout", type=float, default=DEFAULT_WAIT_TIMEOUT, help="Seconds to wait for completion")
    args = parser.parse_args()

    workflow = json.loads(args.workflow.read_text(encoding="utf-8"))
    manifest = run_workflow(workflow, args.out, args.wait_timeout)

    print(f"prompt_id={manifest['prompt_id']}")
    print(f"outputs: {len(manifest['outputs'])} file(s) -> {args.out}")


if __name__ == "__main__":
    main()
