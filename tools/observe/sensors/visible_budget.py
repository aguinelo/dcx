import json
import os
import sys


def load_bundle():
    path = os.environ.get("DCX_BUNDLE")
    if not path:
        raise SystemExit("DCX_BUNDLE não definido")
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def main():
    bundle = load_bundle()
    metrics = bundle.get("metrics") or {}
    expected = bundle.get("expected") or {}
    errors = []

    cost = metrics.get("cost_usd")
    max_cost = expected.get("max_cost_usd")
    if max_cost is not None:
        if not number(cost):
            errors.append("custo do bundle desconhecido")
        elif cost > max_cost:
            errors.append(f"custo US$ {cost:.4f} > US$ {max_cost:.4f}")
        if metrics.get("cost_complete") is False:
            errors.append("métricas de custo incompletas")

    calls = metrics.get("model_calls")
    max_calls = expected.get("max_model_calls")
    if max_calls is not None:
        if not isinstance(calls, int):
            errors.append("número de chamadas de modelo desconhecido")
        elif calls > max_calls:
            errors.append(f"{calls} chamadas de modelo > {max_calls}")

    max_duration = expected.get("max_duration_ms")
    duration = bundle.get("duration_ms")
    if max_duration is not None:
        if not number(duration):
            errors.append("duração do bundle desconhecida")
        elif duration > max_duration:
            errors.append(f"duração {duration} ms > {max_duration} ms")

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(
        f"Orçamento preservado: US$ {cost if number(cost) else '?'} em {calls} chamada(s), {duration} ms."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
