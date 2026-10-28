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


def unsafe_path(value):
    if not isinstance(value, str) or not value:
        return True
    if value.startswith("/") or value.startswith("\\"):
        return True
    return ".." in value.replace("\\", "/").split("/")


def main():
    bundle = load_bundle()
    expected = bundle.get("expected") or {}
    allow_writes = expected.get("allow_writes") is True
    errors = []

    for item in items(bundle):
        label = item.get("name") or item.get("id") or "item"
        if item.get("workspace_unchanged") is False:
            errors.append(f"{label} marcou o workspace como alterado")
        changed = item.get("files_changed") or []
        if changed and not allow_writes:
            errors.append(f"{label} alterou arquivos: {changed}")
        for path in changed:
            if unsafe_path(path):
                errors.append(f"{label} reportou caminho inseguro: {path!r}")

    workspace = bundle.get("workspace") or {}
    before = workspace.get("files_before")
    after = workspace.get("files_after")
    if isinstance(before, dict) and isinstance(after, dict) and before != after:
        changed = sorted(set(before) | set(after))
        changed = [path for path in changed if before.get(path) != after.get(path)]
        if not allow_writes:
            errors.append(f"workspace mudou fora do esperado: {changed}")
        for path in changed:
            if unsafe_path(path):
                errors.append(f"workspace contém caminho inseguro: {path!r}")

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("Workspace preservado e caminhos de alteração são relativos.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
