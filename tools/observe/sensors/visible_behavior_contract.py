import json
import os
import sys


def load_bundle():
    path = os.environ.get("DCX_BUNDLE")
    if not path:
        raise SystemExit("DCX_BUNDLE não definido")
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def items(bundle):
    return (bundle.get("checks") or []) + (bundle.get("cases") or [])


def main():
    bundle = load_bundle()
    expected = bundle.get("expected") or {}
    errors = []

    if bundle.get("ok") is not True:
        errors.append("o bundle não está marcado como ok")

    expected_provider = expected.get("provider")
    fallback = bundle.get("fallback") or {}
    if expected_provider and bundle.get("provider") != expected_provider:
        if not fallback.get("used"):
            errors.append(
                f"provider do bundle {bundle.get('provider')!r} != {expected_provider!r}"
            )

    for name in expected.get("checks") or []:
        matches = [
            item
            for item in bundle.get("checks") or []
            if item.get("name") == name
        ]
        if len(matches) != 1:
            errors.append(f"check {name!r} aparece {len(matches)} vez(es)")
        elif matches[0].get("ok") is not True:
            errors.append(f"check {name!r} não passou")

    for case_id in expected.get("cases") or []:
        matches = [
            item
            for item in bundle.get("cases") or []
            if item.get("id") == case_id
        ]
        if len(matches) != 1:
            errors.append(f"caso {case_id!r} aparece {len(matches)} vez(es)")
        elif matches[0].get("ok") is not True:
            errors.append(f"caso {case_id!r} não passou")

    max_duration = expected.get("max_duration_ms")
    duration = bundle.get("duration_ms")
    if max_duration is not None:
        if not isinstance(duration, (int, float)):
            errors.append("duração do bundle desconhecida")
        elif duration > max_duration:
            errors.append(f"duração {duration} ms > {max_duration} ms")

    events = bundle.get("events") or []
    sequences = [event.get("sequence") for event in events]
    if any(not isinstance(sequence, int) for sequence in sequences):
        errors.append("evento sem sequence inteiro")
    elif sequences != sorted(sequences) or len(set(sequences)) != len(sequences):
        errors.append(f"sequência de eventos fora de ordem: {sequences}")

    for item in items(bundle):
        label = item.get("name") or item.get("id") or "item"
        if item.get("ok") is not True:
            errors.append(f"{label} não passou")

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(
        f"Contrato de comportamento preservado: {len(items(bundle))} item(ns), provider {bundle.get('provider')}."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
