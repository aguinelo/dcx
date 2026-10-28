# Testes reais e evals

O repositório tem quatro níveis de validação:

1. **Testes determinísticos**: `cargo test --workspace` e os testes do desktop. Não usam rede nem tokens.
2. **E2E real opt-in**: valida o caminho crítico do CLI e do daemon com um provider real.
3. **Evals**: mede qualidade, custo e latência em casos de conversa com checks determinísticos.
4. **Sensores de observabilidade**: leem logs e bundles para detectar exceções, vazamentos e desvios de comportamento.

Os níveis 2 e 3 não rodam no CI normal e exigem token real. O nível 4 roda no CI sobre fixtures e bundles; ao ler um run já executado, não chama provider.

## Preparar o binário

```sh
cargo build -p harness-cli
```

O runner usa `target/debug/harness` por padrão. Defina `HARNESS_BIN` para usar outro binário.

## E2E real

O e2e real cobre:

- `harness chat` com provider real, streaming, persistência de sessão e ausência de escrita no projeto;
- daemon `harness serve`, `/health`, `POST /chat` via SSE, evento `done` e sessão visível em `/sessions`.

Ele ainda não cobre a interface Electron empacotada, o fluxo completo de contrato/execução, terminal, rotinas ou GitHub.

```sh
HARNESS_E2E=1 \
HARNESS_E2E_PROVIDER=minimax \
MINIMAX_API_KEY=... \
node tools/e2e/real.mjs
```

Para Claude:

```sh
HARNESS_E2E=1 \
HARNESS_E2E_PROVIDER=claude \
ANTHROPIC_API_KEY=... \
node tools/e2e/real.mjs
```

O relatório vai para `target/e2e-real-report.json` e o bundle para `target/e2e-bundle.json`; use `HARNESS_E2E_REPORT` e `HARNESS_E2E_BUNDLE` para outros caminhos. Sem `HARNESS_E2E=1`, o comando apenas imprime `SKIP`.

## Evals

Essa camada é a base de eval engineering: casos versionados, checks determinísticos, `pass@k`, custo e latência. Ela ainda não substitui avaliação humana nem mede qualidade aberta.

A suite fica em `evals/smoke.json` e usa checks determinísticos:

- `contains`, `not_contains`, `equals` e `regex` no texto da última resposta;
- `file_exists` e `file_not_exists` no workspace temporário do caso.

Cada caso roda em um workspace isolado e pode ter `prompt` ou `turns`, além de `files` para preparar fixtures. O runner registra sessão, modelo, custo, duração e tokens quando o provider reporta. Use `--provider claude` para sobrescrever o provider da suite.

```sh
HARNESS_EVAL=1 \
HARNESS_E2E_PROVIDER=minimax \
MINIMAX_API_KEY=... \
node tools/eval/run.mjs --suite evals/smoke.json
```

Para repetir cada caso e calcular `pass@k`:

```sh
HARNESS_EVAL=1 \
HARNESS_E2E_PROVIDER=minimax \
MINIMAX_API_KEY=... \
node tools/eval/run.mjs --suite evals/smoke.json --repeat 3
```

Valide a suite sem gastar token:

```sh
node tools/eval/run.mjs --suite evals/smoke.json --dry-run
```

O relatório vai para `target/eval-report.json` e o bundle para `target/eval-bundle.json`; use `--out` e `HARNESS_EVAL_BUNDLE` para outros caminhos e `--json` para imprimir o JSON completo. O comando falha se algum caso falhar.

Os testes do scorer rodam sem token:

```sh
node --test tools/eval/scorer.test.mjs
```

## Sensores de observabilidade

O e2e e os evals geram bundles em `target/e2e-bundle.json` e `target/eval-bundle.json`. Os sensores de `tools/observe/` leem esses bundles e procuram logs, exceções e desvios de comportamento. Eles seguem as premissas do M0: especificação antes da execução, sensores visíveis e ocultos, trava por checksum e controle negativo.

- `visible-no-unhandled-exception`: procura traceback, pânico e exceção não tratada nos logs;
- `visible-behavior-contract`: confere checks, provider, duração e ordem dos eventos;
- `visible-budget`: confere custo, chamadas de modelo e duração;
- `hidden-secret-leak`: procura chaves de API e padrões de segredo;
- `hidden-silent-fallback`: recusa troca de provider sem fallback declarado;
- `hidden-workspace-mutation`: recusa escrita inesperada e caminhos absolutos ou com `..`.

O contrato fica em `tools/observe/contract.json`, a suíte em `tools/observe/suite.json` e a trava em `tools/observe/sensor_lock.json`. Cada critério tem pelo menos um sensor visível, e a suíte sempre inclui sensores ocultos. O runner valida a trava e roda um controle saudável e um defeituoso antes de medir o bundle.

```sh
node tools/observe/run.mjs --bundle target/e2e-bundle.json
node tools/observe/run.mjs --bundle target/eval-bundle.json
```

Use `--visible-only` para o feedback visível sem executar os ocultos, `--out` para outro relatório e `--json` para imprimir o JSON completo. O relatório padrão é `target/observability-report.json`; o comando falha se qualquer sensor falhar.

Depois de editar contrato, suíte ou scripts, renove a trava:

```sh
node tools/observe/lock.mjs
```

Os testes dos sensores rodam sem token:

```sh
node --test tools/observe/run.test.mjs tools/observe/collect.test.mjs
```

## Runs do harness

O coletor transforma um run real em um bundle de observabilidade. Ele lê `.harness/runs/<id>/` e confere artefatos, eventos, veredito, sensores, estado final, workspace e provider.

```sh
node tools/observe/collect.mjs --run .harness/runs/<id> --out target/run-bundle.json
node tools/observe/run.mjs --bundle target/run-bundle.json
```

Capture stdout e stderr do `harness run` para que os sensores também os leiam:

```sh
target/debug/harness run --request "..." > target/run.stdout 2> target/run.stderr
node tools/observe/collect.mjs \
  --run .harness/runs/<id> \
  --stdout target/run.stdout \
  --stderr target/run.stderr \
  --out target/run-bundle.json
```

Sem `--run`, o coletor usa o run mais recente em `.harness/runs`. Use `--expect rejected` para um run que deve falhar, `--allow-fallback` para declarar múltiplos providers e `--strict` para sair com erro quando o bundle não estiver ok. Os tetos opcionais são `--max-cost-usd`, `--max-duration-ms` e `--max-model-calls`.

### Medição local de 2026-10-27

| Provider | E2E | Custo E2E | Eval | Custo eval | p95 eval | Observabilidade |
|---|---|---|---|---|---|---|
| MiniMax | 9,3 s | US$ 0,1421 | 4/4 | US$ 0,3476 | 18,7 s | 6/6 |
| Claude | 13,6 s | US$ 0,2667 | 4/4 | US$ 0,4408 | 5,4 s | 6/6 |

Os valores variam por execução. O controle negativo reprovou os seis sensores no fixture defeituoso nas duas rodadas. O coletor também foi validado em um `harness run --request` completo com Codex falso: 7/7 checks do bundle, 6/6 sensores e estado final `awaiting_merge`.

## O que ainda falta

- E2E da interface Electron e do DMG empacotado.
- E2E do fluxo completo `run --request` com contrato, sensores, plano, execução e merge gate.
- Automatizar a coleta de `.harness/runs/<id>` no e2e e no CI; hoje o coletor é manual.
- Eval com juiz de modelo, rubricas abertas e comparação entre providers/modelos.
- Threshold de regressão por suite e histórico versionado de resultados.
- Cobertura de terminal, rotinas, GitHub e recuperação de runs no e2e real.

O workflow manual `.github/workflows/real.yml` executa e2e, evals e observabilidade quando acionado e lê os segredos `MINIMAX_API_KEY` e `ANTHROPIC_API_KEY`.
