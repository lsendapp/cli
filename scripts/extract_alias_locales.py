#!/usr/bin/env python3
"""Extract aliasGenerator data from the official LocalSend i18n files."""

from __future__ import annotations

import json
import sys
from pathlib import Path

KEY = "aliasGenerator(ignoreMissing, ignoreGpt)"


def main() -> int:
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} <upstream-i18n-dir> <output-json>", file=sys.stderr)
        return 1

    i18n_dir = Path(sys.argv[1])
    out_path = Path(sys.argv[2])

    en_path = i18n_dir / "en.json"
    with en_path.open(encoding="utf-8") as handle:
        base = json.load(handle)[KEY]

    locales: dict[str, dict[str, object]] = {}
    for path in sorted(i18n_dir.glob("*.json")):
        if path.name.startswith("_"):
            continue
        with path.open(encoding="utf-8") as handle:
            data = json.load(handle)
        entry = data.get(KEY, {})
        locale_id = path.stem
        locales[locale_id] = {
            "adjectives": entry.get("adjectives") or base["adjectives"],
            "fruits": entry.get("fruits") or base["fruits"],
            "combination": entry.get("combination") or base["combination"],
        }

    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", encoding="utf-8") as handle:
        json.dump(locales, handle, ensure_ascii=False, indent=2)
        handle.write("\n")

    print(f"wrote {len(locales)} locales to {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
