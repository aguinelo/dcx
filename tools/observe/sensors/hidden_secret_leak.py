import json
import os
import re
import sys

SECRET_ENV = ("MINIMAX_API_KEY", "DCX_MINIMAX_API_KEY", "ANTHROPIC_API_KEY")
PATTERNS = [
    ("anthropic", re.compile(r"sk-ant-[A-Za-z0-9_-]{8,}")),
    ("openai", re.compile(r"sk-[A-Za-z0-9]{16,}")),
    (
        "api-key",
        re.compile(
            r"api[_-]?key['\"]?\s*[:=]\s*['\"]?[A-Za-z0-9_-]{8,}", re.IGNORECASE
        ),
    ),
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


def redact(value):
    if len(value) <= 8:
        return "***"
    return f"{value[:4]}...{value[-2:]}"


def main():
    bundle = load_bundle()
    text = logs(bundle)
    hits = []
    for name in SECRET_ENV:
        secret = os.environ.get(name)
        if secret and len(secret) >= 8 and secret in text:
            hits.append(f"valor de {name} apareceu nos logs ({redact(secret)})")
    for name, pattern in PATTERNS:
        for match in pattern.finditer(text):
            hits.append(f"padrão {name} nos logs ({redact(match.group(0))})")
    if hits:
        for hit in hits[:10]:
            print(hit, file=sys.stderr)
        print(f"{len(hits)} vazamento(s) de segredo", file=sys.stderr)
        return 1
    print("Nenhuma chave de API ou padrão de segredo nos logs.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
