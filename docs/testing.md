# Testes reais e evals

O repositório tem quatro níveis de validação:

1. **Testes determinísticos**: `cargo test --workspace` e os testes do desktop. Não usam rede nem tokens.
2. **E2E real opt-in**: valida o caminho crítico do CLI e do daemon com um provider real.
3. **Evals**: mede qualidade, custo e latência em casos de conversa com checks determinísticos.
4. **Sensores de observabilidade**: leem logs e bundles para detectar exceções, vazamentos e desvios de comportamento.

Os níveis 2 e 3 não rodam no CI normal e exigem token real. O nível 4 roda no CI sobre fixtures e bundles; ao ler um run já executado, não chama provider.

## Preparar o binário

```sh
cargo build -p dcx-cli
```

O runner usa `target/debug/dcx` por padrão. Defina `DCX_BIN` para usar outro binário.

## E2E real

O e2e real cobre:

- `dcx chat` com provider real, streaming, persistência de sessão e ausência de escrita no projeto;
- daemon `dcxd`, `/health`, `POST /chat` via SSE, evento `done` e sessão visível em `/sessions`.

Ele ainda não cobre a interface Electron empacotada, o fluxo completo de contrato/execução, terminal, rotinas ou GitHub.

```sh
DCX_E2E=1 \
DCX_E2E_PROVIDER=minimax \
MINIMAX_API_KEY=... \
node tools/e2e/real.mjs
```

Para Claude:

```sh
DCX_E2E=1 \
DCX_E2E_PROVIDER=claude \
ANTHROPIC_API_KEY=... \
node tools/e2e/real.mjs
```

O relatório vai para `target/e2e-real-report.json` e o bundle para `target/e2e-bundle.json`; use `DCX_E2E_REPORT` e `DCX_E2E_BUNDLE` para outros caminhos. Sem `DCX_E2E=1`, o comando apenas imprime `SKIP`.

## Evals

Essa camada é a base de eval engineering: casos versionados, checks determinísticos, `pass@k`, custo e latência. Ela ainda não substitui avaliação humana nem mede qualidade aberta.

A suite fica em `evals/smoke.json` e usa checks determinísticos:

- `contains`, `not_contains`, `equals`, `regex` e `not_regex` no texto da última resposta;
- `file_exists` e `file_not_exists` no workspace temporário do caso.

Cada caso roda em um workspace isolado e pode ter `prompt` ou `turns`, além de `files` para preparar fixtures. O runner registra sessão, modelo, custo, duração e tokens quando o provider reporta. Use `--provider claude` para sobrescrever o provider da suite.

```sh
DCX_EVAL=1 \
DCX_E2E_PROVIDER=minimax \
MINIMAX_API_KEY=... \
node tools/eval/run.mjs --suite evals/smoke.json
```

Para repetir cada caso e calcular `pass@k`:

```sh
DCX_EVAL=1 \
DCX_E2E_PROVIDER=minimax \
MINIMAX_API_KEY=... \
node tools/eval/run.mjs --suite evals/smoke.json --repeat 3
```

Valide a suite sem gastar token:

```sh
node tools/eval/run.mjs --suite evals/smoke.json --dry-run
```

O relatório vai para `target/eval-report.json` e o bundle para `target/eval-bundle.json`; use `--out` e `DCX_EVAL_BUNDLE` para outros caminhos e `--json` para imprimir o JSON completo. O comando falha se algum caso falhar.

Os testes do scorer rodam sem token:

```sh
node --test tools/eval/scorer.test.mjs
```

## Linha de base

Medição mais recente com MiniMax-M3 (`--repeat 3`, 28/10/2026):

| Métrica | Resultado |
|---|---|
| Casos | 15/15 aprovados (pass@k 100%) |
| Latência | p50 3,4 s, p95 12,7 s |
| Custo da suíte | US$ 0,36 (cerca de US$ 0,024 por resposta) |
| Sensores | 6/6 passaram |

Na comparação direta da suite anterior, com 12 casos e 3 repetições, o custo caiu de US$ 0,9569 para US$ 0,2910 (redução de 69,6%) e a entrada caiu de 225.635 para 60.531 tokens (redução de 73,2%). O p95 medido foi de 12,5 s antes e 14,4 s depois. A suite atualizada, com um caso novo de resposta direta, voltou a 12,7 s de p95 e passou em todas as repetições.

Comparação entre providers na mesma suite, com 3 repetições por caso:

| Provider e modelo | Aprovação | Custo por resposta | p95 |
|---|---:|---:|---:|
| MiniMax-M3 | 15/15 | US$ 0,0243 | 12,7 s |
| Claude Opus 5.5 | 15/15 | US$ 0,0208 | 4,5 s |

A comparação passou pelos limites configurados: nenhuma queda de qualidade, custo abaixo de +10% e p95 abaixo de +25%. Os resultados variam entre execuções.

O chat continua respondendo em português, consultando arquivos quando necessário e sem alterar o projeto. Se custo, latência ou algum check piorar numa mudança futura, compare com estes resultados.

Para comparar uma nova medição com a linha de base, rode a eval normalmente e depois:

```sh
node tools/eval/compare.mjs \
  --baseline evals/baselines/chat-smoke-minimax.json \
  --candidate target/eval-report.json
```

A comparação exige os mesmos casos e repetições, não aceita queda de qualidade e reprova quando o custo por resposta sobe mais de 10% ou o p95 sobe mais de 25%. Se faltar o custo informado pelo provider, a comparação falha em vez de considerar a medição aprovada.

### Avaliar qualidade com juiz

Para avaliar respostas abertas, adicione `--judge`. O juiz recebe a solicitação da eval, a resposta gerada, o contexto do caso e uma rubrica versionada; devolve somente notas de 1 a 5 e códigos curtos de problema, sem justificativa ou raciocínio. Uma resposta só passa se cumprir os checks determinísticos e atingir a nota mínima de cada critério. O relatório separa custo e latência do chat e do juiz; o bundle soma ambos para os sensores e tetos.

Como essa opção faz chamadas extras e pode gerar custo, ela fica desligada por padrão. Se o chat usa MiniMax e o juiz Claude, as duas chaves precisam estar disponíveis:

```sh
set -a; source .env; set +a
DCX_EVAL=1 node tools/eval/run.mjs \
  --suite evals/smoke.json \
  --provider minimax \
  --judge \
  --judge-provider claude \
  --repeat 3
```

A rubrica padrão está em `evals/rubrics/chat-quality.json`. `--judge-rubric` permite usar outra; `--judge-provider` aceita `claude` ou `minimax`. O nome do relatório ganha o sufixo `-judged`, para não comparar acidentalmente com a linha de base sem juiz.

Antes de confiar em uma rubrica ou trocar o modelo avaliador, rode a calibração com respostas de resultado conhecido:

```sh
DCX_EVAL=1 node tools/eval/judge-calibration.mjs \
  --provider claude \
  --repeat 3
```

`--dry-run` mostra os casos e provider sem chamar modelos. Os testes do parser, da rubrica e do gate rodam sem token:

```sh
node --test tools/eval/judge.test.mjs tools/eval/judge-calibration.test.mjs tools/eval/run.test.mjs
```

Além dos checks de cada caso, todo caso reprova quando a resposta vem vazia ou quando o CLI informa um erro, para que um caso com checks só negativos não passe quando o modelo não responde.

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

## Runs do dcode

O coletor transforma um run real em um bundle de observabilidade. Ele lê `.dcx/runs/<id>/` e confere artefatos, eventos, veredito, sensores, estado final, workspace e provider.

```sh
node tools/observe/collect.mjs --run .dcx/runs/<id> --out target/run-bundle.json
node tools/observe/run.mjs --bundle target/run-bundle.json
```

Capture stdout e stderr do `dcx run` para que os sensores também os leiam:

```sh
target/debug/dcx run --request "..." > target/run.stdout 2> target/run.stderr
node tools/observe/collect.mjs \
  --run .dcx/runs/<id> \
  --stdout target/run.stdout \
  --stderr target/run.stderr \
  --out target/run-bundle.json
```

Sem `--run`, o coletor usa o run mais recente em `.dcx/runs`. Use `--expect rejected` para um run que deve falhar, `--allow-fallback` para declarar múltiplos providers e `--strict` para sair com erro quando o bundle não estiver ok. Os tetos opcionais são `--max-cost-usd`, `--max-duration-ms` e `--max-model-calls`.

### Medições locais de 2026-10-27 e 2026-10-28

Calibração do juiz Claude Sonnet: 9/9 classificações corretas (3 casos × 3 repetições), US$ 0,0097 e 33,2 s. A primeira passada MiniMax + juiz Claude, com a suite smoke de 5 casos, aprovou 5/5 no chat e 5/5 no juiz; custo US$ 0,1130 do chat + US$ 0,0087 do juiz; sensores 6/6.

Na repetição 3× da integração, o juiz e os checks aprovaram 13/15 tentativas. Duas respostas ao pedido de criar um arquivo foram reprovadas: uma atingiu o limite de ferramentas e não respondeu; outra passou dos limites de concisão definidos pelo produto. O custo de chat ficou incompleto porque uma chamada foi interrompida; o juiz custou US$ 0,0210. Isso detectou uma instabilidade real do MiniMax nesse pedido; a linha de base de qualidade com juiz ainda não deve ser criada.

| Provider | E2E | Custo E2E | Eval | Custo eval | p95 eval | Observabilidade |
|---|---|---|---|---|---|---|
| MiniMax | 9,3 s | US$ 0,1421 | 4/4 | US$ 0,3476 | 18,7 s | 6/6 |
| Claude | 13,6 s | US$ 0,2667 | 4/4 | US$ 0,4408 | 5,4 s | 6/6 |

Os valores variam por execução. O controle negativo reprovou os seis sensores no fixture defeituoso nas duas rodadas. O coletor também foi validado em um `dcx run --request` completo com Codex falso: 7/7 checks do bundle, 6/6 sensores e estado final `awaiting_merge`.

## O que ainda falta

- E2E da interface Electron e do DMG empacotado.
- E2E do fluxo completo `run --request` com contrato, sensores, plano, execução e merge gate.
- Automatizar a coleta de `.dcx/runs/<id>` no e2e e no CI; hoje o coletor é manual.
- Corrigir e repetir a resposta read-only do MiniMax, que falhou em 2/3 tentativas na suite julgada; depois criar uma linha de base versionada para notas abertas.
- Threshold de regressão por suite e histórico versionado de resultados.
- Cobertura de terminal, rotinas, GitHub e recuperação de runs no e2e real.

O workflow manual `.github/workflows/real.yml` executa e2e, evals e observabilidade quando acionado e lê os segredos `MINIMAX_API_KEY` e `ANTHROPIC_API_KEY`.
