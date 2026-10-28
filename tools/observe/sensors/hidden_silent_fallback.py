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
    expected_provider = expected.get("provider")
    fallback = bundle.get("fallback") or {}
    errors = []

    if expected_provider:
        actual = bundle.get("provider")
        if actual and actual != expected_provider:
            if not fallback.get("used"):
                errors.append(
                    f"provider mudou de {expected_provider!r} para {actual!r} sem fallback declarado"
                )
            elif not fallback.get("from") or not fallback.get("reason"):
                errors.append("fallback declarado sem from/reason")

        for item in items(bundle):
            label = item.get("name") or item.get("id") or "item"
            actual = item.get("provider")
            if actual and actual != expected_provider:
                if not item.get("fallback") and not fallback.get("used"):
                    errors.append(
                        f"{label} usou {actual!r} em vez de {expected_provider!r} sem fallback"
                    )

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(f"Provider esperado preservado: {expected_provider or 'não declarado'}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
