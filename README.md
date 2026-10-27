# Harness

O Harness ajuda você a explorar um projeto, transformar uma ideia em uma tarefa clara e acompanhar a implementação com critérios de aceite. Você decide o que será feito e aprova os pontos importantes.

## O que você pode fazer

- Conversar sobre um projeto e entender como ele funciona.
- Pedir uma mudança e acompanhar o trabalho até a revisão.
- Organizar vários pedidos em uma fila finita e retomá-la depois de uma interrupção.
- Escolher o modelo usado e definir limites de tempo, chamadas e custo conhecido.
- Usar o app desktop para conversar e acompanhar o projeto.

## Começar pelo terminal

Abra o terminal na pasta do projeto e inicie uma conversa:

```sh
cargo run -p harness-cli -- chat
```

O Harness analisa o projeto e conversa sem alterar arquivos. Digite `/help` para ver os comandos disponíveis e `/exit` para sair. Para continuar uma conversa anterior:

```sh
cargo run -p harness-cli -- chat --session ID_DA_SESSAO
```

## Pedir uma mudança

Descreva o resultado que você quer:

```sh
cargo run -p harness-cli -- run --request "Adicione uma página de status ao serviço."
```

O Harness esclarece dúvidas e pede sua aprovação dos critérios antes de implementar. Ao final, revise as mudanças no projeto e faça o merge pelo seu fluxo habitual. Para continuar um run interrompido:

```sh
cargo run -p harness-cli -- resume --run ID_DO_RUN
```

## Rodar uma fila de tarefas

Crie um arquivo `tasks.json` na pasta do projeto. Cada tarefa deve ter um identificador e um pedido:

```json
{
  "tasks": [
    { "id": "status", "request": "Adicione uma página de status ao serviço." },
    { "id": "erros", "request": "Deixe as mensagens de erro mais claras." }
  ]
}
```

Inicie a fila:

```sh
cargo run -p harness-cli -- loop start --file tasks.json
```

A fila executa os pedidos na ordem do arquivo e aceita até 25 tarefas. Se uma tarefa falhar ou o processo for interrompido, o terminal mostra um identificador. Use-o para continuar:

```sh
cargo run -p harness-cli -- loop resume --id ID_DA_FILA
```

O Harness preserva as aprovações: uma tarefa pode parar para pedir sua decisão. Os limites de custo e duração são aplicados separadamente a cada tarefa.

## Escolher um modelo e definir limites

Copie `.env.example` para `.env` na pasta do projeto e preencha as opções desejadas. O arquivo `.env` é ignorado pelo Git.

Você pode escolher `codex`, `claude`, `minimax` ou `auto` em `HARNESS_PROVIDER`. Para controlar uma execução, configure:

```env
HARNESS_RUN_MAX_MODEL_CALLS=40
HARNESS_RUN_MAX_COST_USD=2.00
HARNESS_RUN_MAX_DURATION_MINUTES=120
```

Os limites valem para cada tarefa. O custo pode passar do teto pelo valor de uma chamada já iniciada, pois ele só é conhecido quando o modelo responde. Para usar um teto de custo, escolha um modelo que informe o custo; o Codex não fornece esse dado.

Para habilitar pesquisa na web durante a conversa, configure `HARNESS_CHAT_WEB=1` no `.env`. Sem essa opção, a conversa pesquisa apenas o projeto.

## Usar o app desktop

Para iniciar a versão de desenvolvimento:

```sh
cd desktop
pnpm desktop
```

No desktop, escolha o projeto, converse e acompanhe o trabalho. O acesso a repositórios fora do projeto pede confirmação e pode ser removido depois.

## Desenvolvimento

O Harness é desenvolvido em Rust; o app desktop usa Electron e TypeScript. Para validar as alterações localmente:

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```
