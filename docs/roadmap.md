# Roadmap por rodadas

Cada rodada fecha com testes, demo reproduzível, documentação e commit. A rodada interna é sempre: hipótese → fixture → sensores → implementação → avaliação → decisão.

| Rodada | Foco | Estado |
|---|---|---|
| R0 | Fundação: schemas, engine, CLI, demo | concluída |
| R1 | Entendimento e contrato | concluída |
| R2 | Sensores executáveis, lock e baseline | concluída |
| R3 | Plano, cópia isolada, verifier e merge gate | concluída |
| R4 | Prova adversarial `job-02`, disputa de sensor, issues saneadas | concluída; disputa interativa pendente |
| R5 | Apply atômico, ledger de eventos e limpeza do estado | concluída |
| R6 | Isolamento de ambiente e sandbox opt-in dos sensores | concluída no macOS e Linux; validação runtime Linux pendente |
| R7 | Provider `auto`, CI e documentação operacional | concluída; custo por run pendente |
| R8 | Passos paralelos, retomada e múltiplos runs | concluída; demo live de plano paralelo pendente |

## Critérios de saída

- **R4:** o passo visível do `job-02` passa com o atalho e a integração é reprovada pelo sensor oculto; a disputa altera somente o sensor contestado; issues do verifier não vazam ids/checks ocultos.
- **R5:** uma falha no meio do apply restaura os arquivos já copiados; cada run grava `events.jsonl`; `state prune` remove runs antigos e preserva os recentes.
- **R6:** sensores não herdam o ambiente do usuário e podem rodar sob `sandbox-exec` no macOS ou `bwrap` no Linux, com rede e escrita fora do workspace bloqueadas.
- **R7:** `HARNESS_PROVIDER=auto` tenta Claude e cai para Codex; CI roda fmt, testes, clippy, schemas e as demos.
- **R8:** passos independentes executam em worktrees separadas; um run interrompido retoma do último passo concluído; dois runs concorrentes não colidem.

## Próxima rodada

R8 tem lock de projeto, `harness resume --run <id>` e execução por grupos topológicos. Passos independentes rodam em cópias isoladas por passo (`runs/<id>/steps/<step_id>`), com merge no workspace base e detecção de conflito no mesmo grupo; a retomada pula passos com `step.completed` aprovado, exceto a integração. A demo live de um plano com passos paralelos ainda não foi feita.
