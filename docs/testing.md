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
- daemon `dcxd`, `/health`, `POST /chat` via SSE, evento `done` e sessão visível em `/sessions`;
- `dcx run --request` num projeto temporário: contrato, sensores, plano, execução, arquivo final e merge gate. A aprovação automática existe apenas nesse workspace descartável; o script confere o resultado e rejeita alterações extras;
- coleta do run e sensores de observabilidade, mesmo quando o run falha ou é interrompido.

Ele ainda não cobre a interface Electron empacotada, terminal, rotinas ou GitHub.

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

### Avaliar contratos de tarefas simples

Esta eval mede se uma solicitação pequena vira um contrato proporcional. Ela aprova o contrato apenas dentro de um projeto temporário e para antes da implementação, com o limite de quatro chamadas. O relatório registra critérios, sensores, custo e tempo; nenhuma alteração chega ao projeto real.

```sh
set -a; source .env; set +a
DCX_EVAL=1 node tools/eval/contract-run.mjs --provider minimax --repeat 3
```

O teste sem custo valida a regra e o runner:

```sh
node --test tools/eval/contract-quality.test.mjs tools/eval/contract-run.test.mjs
```

Linha de base com MiniMax-M3.1-Flash (`--repeat 1`, 28/10/2026):

| Métrica | Resultado |
|---|---|
| Critérios gerados | 5 (limite da eval: 3) |
| Sensores gerados | 7 (limite da eval: 5) |
| Metadados internos no contrato | sim (PID e hash do lock) |
| Chamadas de modelo (entendimento + contrato) | 2 |
| Latência | ~8 min (timeout na etapa de sensores) |

O modelo super-especifica contratos para tarefas simples e vaza detalhes internos do dcode (como caminho temporário e conteúdo do lock) nos critérios e sensores. Antes de ajustar o prompt do contratante, repita a medição 3 vezes para confirmar estabilidade; depois compare com os resultados pós-ajuste.

## Linha de base

Medição da suite de chat com MiniMax-M3 (`--repeat 3`, 28/10/2026):

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

Os tempos de E2E na tabela acima são da versão anterior, que cobria apenas chat e daemon. A medição real de `dcx run` começou na tentativa abaixo e ainda não tem uma execução completa aprovada.

Na primeira tentativa de `dcx run --request` real com MiniMax, três chamadas custaram US$ 0,7665 e levaram cerca de 215 s. O contrato gerado para criar um único arquivo trouxe nove sensores, incluindo verificações ligadas ao PID do teste e ao conteúdo interno do lock. Interrompi a tentativa antes da autoria dos sensores terminar; ela **não** passou pelo fluxo completo. O runner foi ajustado para preservar relatório e bundle em runs incompletos. Não repeti a chamada para evitar custo sem corrigir antes esse desvio.

## Distribuição macOS

O script `desktop/tools/package-macos.mjs` gera um DMG em `desktop/release/`:

```sh
cd desktop && node tools/package-macos.mjs
```

**Sem assinatura Apple Developer:** o macOS bloqueia o app na primeira abertura. Para abrir, clique com o botão direito no app → **Abrir** e confirme. Não afeta usuários que compilam e instalam localmente.

**Para distribuição pública (App Store ou download direto):** é necessário um Apple Developer ID (US$ 99/ano em developer.apple.com). Antes de rodar o script, defina as variáveis de ambiente:

```sh
# Opção A: perfil de keychain com a identidade já cadastrada
export APPLE_KEYCHAIN_PROFILE=<nome-do-perfil>

# Opção B: credenciais diretas
export APPLE_ID=<seu-apple-id>
export APPLE_APP_SPECIFIC_PASSWORD=<senha-de-app-gerada-em-appleid.apple.com>
export APPLE_TEAM_ID=<team-id-do-developer-account>
```

O script `desktop/tools/notarize.mjs` já lê essas variáveis e executa a notarização via `notarytool` após a compilação. Sem elas, a notarização é ignorada e o DMG é gerado sem assinatura (adequado para uso interno).

## O que ainda falta

- E2E da interface Electron e do DMG empacotado.
- Repetir o e2e completo `run --request` depois de corrigir a geração de contrato/sensores para pedidos simples; a primeira tentativa MiniMax foi interrompida e não deve ser tratada como aprovada.
- Automatizar a coleta de `.dcx/runs/<id>` no CI.
- Corrigir e repetir a resposta read-only do MiniMax, que falhou em 2/3 tentativas na suite julgada; depois criar uma linha de base versionada para notas abertas.
- Threshold de regressão por suite e histórico versionado de resultados.
- Cobertura de terminal, rotinas, GitHub e recuperação de runs no e2e real.

O workflow manual `.github/workflows/real.yml` executa e2e, evals e observabilidade quando acionado e lê os segredos `MINIMAX_API_KEY` e `ANTHROPIC_API_KEY`.
