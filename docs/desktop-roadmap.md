# Roadmap do desktop

Roadmap da interface Electron em `desktop/`, separado do roadmap do motor Harness.
O objetivo é aproximar a interface do Claude Desktop mantendo o Harness como backend
de conversa e execução.

## Fases

| Fase | Estado | Entrega |
|---|---|---|
| 1. Sessões confiáveis | Concluída | Criar uma conversa nova não reutiliza a sessão anterior; removido o fluxo de simulação que confundia o estado real. |
| 2. Dados reais de trabalho | Concluída | Alterações mostra PRs reais via `gh`; tarefas em segundo plano acompanham runs do Harness e podem ser canceladas. |
| 3. Troca de workspace | Concluída | Itens locais em Workspaces são selecionáveis; histórico, configurações e conversas passam a usar o projeto escolhido; seleção persiste entre aberturas. |
| 4. Ferramentas da interface | Em andamento | Terminal executa comandos reais no workspace selecionado com confirmação por comando, timeout e saída limitada. Ainda faltam Artefatos, Rotinas e Procurar. |
| 5. Distribuição | Depois | Empacotar como aplicativo macOS instalável, com ícone, configuração do daemon e atualização/build reproduzível. |

## Critérios de conclusão

- Uma conversa nova recebe um `session_id` novo e não altera o histórico anterior.
- A lista de tarefas reflete runs reais e permite cancelar uma execução ativa.
- Trocar de workspace recarrega a lista de sessões e não mistura mensagens entre projetos.
- O terminal executa no workspace selecionado, pede confirmação antes de cada comando e encerra a sessão se um comando exceder o timeout.
- A interface não apresenta dados simulados como se fossem dados reais.
- `cd desktop && npm run typecheck && npm run build` passa antes de cada entrega.

## Como exercitar manualmente

Inicie o daemon na raiz do repositório e abra o desktop apontando para um workspace:

```sh
cargo run -p harness-cli -- serve --bind 127.0.0.1:8787
cd desktop
HARNESS_WORKSPACE="$PWD/.." npm run desktop
```

Na interface, crie uma conversa, envie uma mensagem e clique em **Criar**. Envie outra
mensagem e confirme que a conversa anterior continua na lista e que a nova aparece
separada. Em **Workspaces**, clique em outro projeto local; confirme que o chip muda,
as conversas carregadas pertencem ao projeto escolhido e que, ao reiniciar a janela,
esse workspace continua selecionado.

Para validar **Alterações**, a máquina precisa ter `gh` autenticado e PRs visíveis.
Para validar **Tarefas em segundo plano**, promova uma conversa para execução e
acompanhe o run no painel; use o controle de cancelar enquanto ele estiver ativo.

Para validar o **Terminal**, abra `>_`, execute `pwd` e confirme o diálogo: a saída
deve apontar para o workspace selecionado. Cada comando exige confirmação; `↻`
reinicia o shell, e comandos que excedem 60 segundos encerram a sessão do shell.
