# Roadmap do desktop

Roadmap da interface Electron em `desktop/`, separado do roadmap do motor Harness.
O objetivo é aproximar a interface do Claude Desktop mantendo o Harness como backend
de conversa e execução.

## Fases

| Fase | Estado | Entrega |
|---|---|---|
| 1. Sessões confiáveis | Concluída | Criar uma conversa nova não reutiliza a sessão anterior; removido o fluxo de simulação que confundia o estado real. |
| 2. Dados reais de trabalho | Concluída | Alterações mostra PRs reais via `gh`; tarefas em segundo plano acompanham runs do Harness e podem ser canceladas. |
| 3. Troca de workspace | Concluída | Itens locais em Workspaces são selecionáveis; o seletor nativo adiciona qualquer pasta; histórico, configurações e conversas usam o projeto escolhido; workspaces recentes persistem entre aberturas. |
| 4. Ferramentas da interface | Concluída | Terminal real com confirmação por comando, catálogo/visualizador de artefatos JSON, busca local, rotinas com agendamento recorrente de 1 h a 7 dias e retomada de runs após reinício do daemon. |
| 5. Distribuição | Em andamento | Empacotamento macOS reproduzível em DMG, CLI Harness incluído, ícone próprio e daemon local iniciado automaticamente. Assinatura/notarização condicionais estão implementadas; credenciais Apple e canal de atualização automática ainda são pendências externas. |

## Critérios de conclusão

- Uma conversa nova recebe um `session_id` novo e não altera o histórico anterior.
- A lista de tarefas reflete runs reais e permite cancelar uma execução ativa.
- Trocar de workspace recarrega a lista de sessões e não mistura mensagens entre projetos.
- O botão `+` em Workspaces abre o seletor nativo de pastas, e a escolha continua disponível após reiniciar o app.
- O terminal executa no workspace selecionado, pede confirmação antes de cada comando e encerra a sessão se um comando exceder o timeout.
- A busca encontra mensagens apenas no workspace atual e abre a conversa correspondente sem enviar conteúdo para um provedor.
- Rotinas são persistidas por workspace; usar uma rotina prepara uma conversa nova e não envia nada antes de revisão e envio pelo usuário.
- Agendamentos ficam desativados por padrão, exigem confirmação, executam em modo somente leitura e registram status e custo conhecido.
- Runs em andamento são retomados após o reinício do daemon; se o contrato ainda não foi aprovado, o fluxo recomeça e pede aprovação novamente.
- O DMG de desenvolvimento inclui o CLI do Harness; o app inicia o daemon local quando o endpoint padrão está indisponível e não assume controle de endpoints customizados.
- O app usa ícone próprio; sem credenciais Apple o build continua reproduzível com assinatura ad-hoc, e com identidade Developer ID + credenciais o hook notariza e aplica o ticket.
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

Em uma instalação nova, use o botão `+` de **Workspaces** para escolher a pasta do
projeto; o app passa a usar esse workspace e o mantém na lista para a próxima abertura.

Para validar **Alterações**, a máquina precisa ter `gh` autenticado e PRs visíveis.
Para validar **Tarefas em segundo plano**, promova uma conversa para execução e
acompanhe o run no painel; use o controle de cancelar enquanto ele estiver ativo.

Para validar o **Terminal**, abra `>_`, execute `pwd` e confirme o diálogo: a saída
deve apontar para o workspace selecionado. Cada comando exige confirmação; `↻`
reinicia o shell, e comandos que excedem 60 segundos encerram a sessão do shell.

Para validar **Artefatos**, abra a seção e selecione um run e um arquivo JSON para conferir o conteúdo. Para validar **Procurar**, busque uma palavra de uma conversa existente no workspace, confira o trecho e abra o resultado.

Para validar **Rotinas**, crie uma instrução, salve, selecione-a novamente e clique em **Usar na conversa**. O texto deve aparecer no composer de uma conversa nova para revisão antes do envio.

Para validar **Agendamento**, selecione uma frequência e confirme o aviso de custo. A primeira execução ocorre após o intervalo escolhido; a lista exibe a próxima execução e, após disparar, o status e o custo conhecido. O daemon precisa permanecer aberto; ao voltar depois de uma pausa, ele executa uma vez a ocorrência vencida e ignora os ciclos intermediários.

Para gerar a build macOS, rode `cd desktop && pnpm package:mac`. O DMG aparece em `desktop/release/` com o ícone próprio. Sem identidade Developer ID ele sai com assinatura ad-hoc; para distribuir externamente, configure a identidade e as credenciais de notarização descritas no `desktop/README.md`. A atualização automática continua pendente de um canal de releases.
