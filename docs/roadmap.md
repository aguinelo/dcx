# Roadmap por rodadas

Cada rodada fecha com testes, demo reproduzível, documentação e commit. A rodada interna é sempre: hipótese → fixture → sensores → implementação → avaliação → decisão.

| Rodada | Foco | Estado |
|---|---|---|
| R0 | Fundação: schemas, engine, CLI, demo | concluída |
| R1 | Entendimento e contrato | concluída |
| R2 | Sensores executáveis, lock e baseline | concluída |
| R3 | Plano, cópia isolada, verifier e merge gate | concluída |
| R4 | Prova adversarial `job-02`, disputa de sensor, issues saneadas | concluída |
| R5 | Apply atômico, ledger de eventos e limpeza do estado | concluída |
| R6 | Isolamento de ambiente e sandbox opt-in dos sensores | concluída no macOS e Linux |
| R7 | Provider `auto`, CI e documentação operacional | concluída |
| R8 | Passos paralelos, retomada e múltiplos runs | concluída; demo live de plano paralelo pendente |
| R9 | Sessão conversacional read-only e base do app desktop | concluída |
| R10 | Loops longos: limites por run, fila finita, recuperação após reinício, orçamento global e painel de filas | concluída |
| R11 | Qualidade medida: e2e com provider real, evals com `pass@k`, sensores de observabilidade e linha de base | concluída (base); evolução em R14 |
| R12 | Custo por resposta | concluída: redução medida de cerca de 70%, sem queda nos checks da eval |
| R13 | Pendências do motor | concluída: tentativas interrompidas preservam o uso parcial, e a demo paralela roda sem token |
| R14 | Evals completos | em andamento: baseline versionado e bloqueio automático de regressão em qualidade, custo e latência |
| R15 | Distribuição | em andamento; depende de repositório remoto e conta Apple |

## Critérios de saída

- **R4:** o passo visível do `job-02` passa com o atalho e a integração é reprovada pelo sensor oculto; a disputa altera somente o sensor contestado; issues do verifier não vazam ids/checks ocultos.
- **R5:** uma falha no meio do apply restaura os arquivos já copiados; cada run grava `events.jsonl`; `state prune` remove runs antigos e preserva os recentes.
- **R6:** sensores não herdam o ambiente do usuário e podem rodar sob `sandbox-exec` no macOS ou `bwrap` no Linux, com rede e escrita fora do workspace bloqueadas.
- **R7:** `DCX_PROVIDER=auto` tenta Claude e cai para Codex; CI roda fmt, testes, clippy, schemas e as demos; cada run registra chamadas, latência e custo conhecido em `metrics.json`.
- **R8:** passos independentes executam em worktrees separadas; um run interrompido retoma do último passo concluído; dois runs concorrentes não colidem.
- **R9:** `dcx chat` mantém sessão persistente, streama a resposta, retoma histórico e não altera arquivos fora de `.dcx/sessions`; `dcxd` expõe a mesma sessão por SSE local; o modo usa Claude/MiniMax em `plan` com apenas `Read`, `Glob` e `Grep`; uma conversa pode ser promovida para run com aprovação explícita.
- **R10:** cada run respeita teto de chamadas, custo conhecido e duração; uma fila de até 25 tarefas retoma do ponto em que parou, inclusive após o daemon reiniciar; o orçamento global soma todas as tarefas; o app mostra cada fila e permite pausar entre tarefas e retomar.
- **R11:** `tools/e2e` e `tools/eval` rodam com token real e geram bundles que os sensores de `tools/observe` auditam; a linha de base fica em `docs/testing.md`.
- **R12:** o custo fixo por resposta (hoje ~18 mil tokens de entrada, ~US$ 0,07) cai de forma mensurável, com eval antes e depois e sem regressão de qualidade.
- **R13:** uma tentativa interrompida pelo limite salva os tokens já informados e registra custo desconhecido quando o provider não entrega o valor final; `dcx run --demo-parallel` demonstra dois passos concorrentes e a integração, sem token e sem alterar o projeto atual.
- **R14:** respostas abertas são avaliadas por um juiz com rubrica; a suíte reprova quando a qualidade cai abaixo da linha de base; MiniMax e Claude são comparados lado a lado; o fluxo completo `dcx run` e o app empacotado têm e2e.
- **R15:** o repositório tem remoto e CI rodando; o app tem canal de releases e atualização automática; com credenciais Apple, o DMG sai notarizado.

## Próximas rodadas

Ordem planejada, sem datas: cada rodada começa quando a anterior fecha com testes verdes.

1. **R14 — Evals completos.** Juiz, limiar de regressão, comparação de modelos e e2e do run completo.
2. **R15 — Distribuição.** Começa quando o repositório remoto existir; a notarização depende da conta Apple.
