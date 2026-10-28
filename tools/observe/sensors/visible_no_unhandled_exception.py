import json
import os
import re
import sys

PATTERNS = [
    ("traceback", re.compile(r"Traceback \(most recent call last\)")),
    ("panic", re.compile(r"\bpanicked at\b")),
    ("panic-thread", re.compile(r"thread '[^']*' panicked")),
    ("unhandled-exception", re.compile(r"Unhandled exception", re.IGNORECASE)),
    ("unhandled-rejection", re.compile(r"UnhandledPromiseRejection")),
    ("fatal", re.compile(r"\bFATAL\b")),
]


def load_bundle():
    path = os.environ.get("DCX_BUNDLE")
    if not path:
        raise SystemExit("DCX_BUNDLE não definido")
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def text_parts(value):
    if isinstance(value, str):
        return [value]
    if isinstance(value, dict):
        return [item for item in value.values() if isinstance(item, str)]
    return []


def logs(bundle):
    parts = []
    for key in ("stdout", "stderr", "logs"):
        parts.extend(text_parts(bundle.get(key)))
    for collection in ("checks", "cases"):
        for item in bundle.get(collection) or []:
            parts.extend(text_parts(item.get("stdout")))
            parts.extend(text_parts(item.get("stderr")))
    return "\n".join(parts)


def main():
    bundle = load_bundle()
    text = logs(bundle)
    hits = []
    for name, pattern in PATTERNS:
        for match in pattern.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            snippet = text[match.start() : match.start() + 160].splitlines()[0]
            hits.append(f"{name} na linha {line}: {snippet}")
    if hits:
        for hit in hits[:10]:
            print(hit, file=sys.stderr)
        print(f"{len(hits)} sinal(is) de exceção não tratada", file=sys.stderr)
        return 1
    print("Nenhum traceback, pânico ou exceção não tratada nos logs.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
