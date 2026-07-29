#!/usr/bin/env python3
"""Plain-assert test for mv_sheet.py's split_thirds_array, run under plain
system Python:
  python scripts/ai-pipeline/test_mv_sheet.py

An analytic invariant (three known-color blocks recovered at their exact
boundaries), not a calibrated band.
"""
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mv_sheet


def test_split_thirds_recovers_exact_panels():
    """A 30x9 image built from three flat-color 30x3 blocks must split back
    into exactly those three blocks, unchanged."""
    img = np.zeros((30, 9, 3), dtype=np.uint8)
    img[:, 0:3] = (10, 20, 30)
    img[:, 3:6] = (40, 50, 60)
    img[:, 6:9] = (70, 80, 90)

    front, side, back = mv_sheet.split_thirds_array(img)

    assert front.shape == (30, 3, 3), f"expected front shape (30, 3, 3), got {front.shape}"
    assert side.shape == (30, 3, 3), f"expected side shape (30, 3, 3), got {side.shape}"
    assert back.shape == (30, 3, 3), f"expected back shape (30, 3, 3), got {back.shape}"
    assert np.all(front == (10, 20, 30)), "front panel did not match its source block"
    assert np.all(side == (40, 50, 60)), "side panel did not match its source block"
    assert np.all(back == (70, 80, 90)), "back panel did not match its source block"
    print("test_split_thirds_recovers_exact_panels passed")


def test_split_thirds_rejects_non_divisible_width():
    """A width that doesn't divide evenly by 3 must raise, not silently crop."""
    img = np.zeros((10, 10, 3), dtype=np.uint8)
    try:
        mv_sheet.split_thirds_array(img)
    except ValueError as e:
        print(f"test_split_thirds_rejects_non_divisible_width passed ({e})")
        return
    raise AssertionError("expected ValueError for width=10 (not divisible by 3)")


if __name__ == "__main__":
    test_split_thirds_recovers_exact_panels()
    test_split_thirds_rejects_non_divisible_width()
    print("all tests passed")
