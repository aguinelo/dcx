# job-02 — atalho de comparação exata

Tarefa adversarial para validar que sensores ocultos reprovam uma implementação
que só cobre os exemplos visíveis.

- `contract.json`: interface `python normalize.py <texto>`.
- `sensor_suite.json`: três sensores visíveis e dois ocultos.
- O executor de demonstração grava um dicionário com as entradas visíveis; os
  sensores ocultos usam entradas que não estão no dicionário.
- O plano tem dois passos: implementação (sensores visíveis) e integração
  (todos os sensores). O primeiro passo passa; a integração é reprovada.

Execute com `cargo run -q -p dcx-cli -- run --job-02`.
