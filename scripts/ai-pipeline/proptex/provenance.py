"""Per-stage provenance records for the texture pipeline (prop_texture.py):
what a stage was asked for, what it consumed and produced, and how long the
producing run took. Stdlib-only, no bpy/cv2, so it imports and runs under a
plain interpreter.

`outputs` (name -> sha256) and `measurements` (name -> number) are kept
separate because they are consumed differently: outputs are cached and
compared as digests, measurements are recomputed from cached outputs every
run and never themselves cached.
"""


def stage_record(stage, unit, version, params, inputs, key, outputs, elapsed_s,
                 measurements=None):
    """One stage's record: what it was asked for (version + params), what it
    consumed (inputs, name -> sha256), what it produced (outputs, name ->
    sha256), any measurements taken of that output (name -> number), and how
    long the producing run took."""
    return {
        "stage": stage,
        "unit": unit,
        "version": version,
        "params": params,
        "inputs": inputs,
        "key": key,
        "outputs": outputs,
        "measurements": measurements or {},
        "elapsed_s": elapsed_s,
    }


def chain(units):
    """The ordered stage records of a run's cache units, each tagged with
    whether this invocation answered from the cache, plus elapsed_s_total.
    `hit` belongs to the invocation and never to the entry, so it is stamped
    here and not in the record the entry stores; elapsed_s is the producing
    run's time carried out of the cache unchanged, so the total prices a
    cold chain however many stages hit."""
    return {
        "stages": [{**unit.record, "hit": unit.hit} for unit in units],
        "elapsed_s_total": round(sum(unit.record["elapsed_s"] for unit in units), 1),
    }
