# dcode

O dcode ajuda você a explorar um projeto, transformar uma ideia em uma tarefa clara e acompanhar a implementação com critérios de aceite. Você decide o que será feito e aprova os pontos importantes.

## O que você pode fazer

- Conversar sobre um projeto e entender como ele funciona.
- Pedir uma mudança e acompanhar o trabalho até a revisão.
- Organizar vários pedidos em uma fila finita, acompanhar o orçamento global e retomá-la depois de uma interrupção.
- Escolher o modelo usado e definir limites de tempo, chamadas e custo conhecido.
- Usar o app desktop para conversar e acompanhar o projeto.

## Instalar os comandos

O dcode tem dois comandos:

- `dcx`: conversa, pede mudanças e roda filas de tarefas.
- `dcxd`: o servidor local que o app desktop usa. O app inicia sozinho quando precisa.

Para instalar os dois:

```sh
cargo install --path crates/dcx-cli
```

Depois disso, `dcx chat` funciona em qualquer pasta. Os exemplos abaixo usam `cargo run`, que funciona sem instalar.

## Começar pelo terminal

Abra o terminal na pasta do projeto e inicie uma conversa:

```sh
cargo run -p dcx-cli -- chat
```

O dcode analisa o projeto e conversa sem alterar arquivos. Digite `/help` para ver os comandos disponíveis e `/exit` para sair. Para continuar uma conversa anterior:

```sh
cargo run -p dcx-cli -- chat --session ID_DA_SESSAO
```

## Pedir uma mudança

Descreva o resultado que você quer:

```sh
cargo run -p dcx-cli -- run --request "Adicione uma página de status ao serviço."
```

O dcode esclarece dúvidas e pede sua aprovação dos critérios antes de implementar. Ao final, revise as mudanças no projeto e faça o merge pelo seu fluxo habitual. Para continuar um run interrompido:

```sh
cargo run -p dcx-cli -- resume --run ID_DO_RUN
```

Se o app ou o daemon reiniciar durante um run, o dcode retoma o trabalho já salvo. Se o contrato ainda não estiver aprovado, a conversa recomeça e pede sua aprovação novamente.

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
cargo run -p dcx-cli -- loop start --file tasks.json
```

A fila executa os pedidos na ordem do arquivo e aceita até 25 tarefas. Se uma tarefa falhar ou o processo for interrompido, o terminal mostra um identificador. Use-o para continuar. A fila também retoma após o reinício do daemon:

```sh
cargo run -p dcx-cli -- loop resume --id ID_DA_FILA
```

O dcode preserva as aprovações: uma tarefa pode parar para pedir sua decisão.

Para limitar a fila inteira, defina um orçamento global antes de iniciar:

```env
DCX_LOOP_MAX_MODEL_CALLS=80
DCX_LOOP_MAX_COST_USD=4.00
DCX_LOOP_MAX_DURATION_MINUTES=240
```

Os limites globais somam o uso de todas as tarefas e são combinados com os limites de cada tarefa. A fila pausa quando qualquer um for atingido. Ao retomar com essas variáveis, o orçamento salvo é substituído. Para acompanhar o andamento, o uso e o orçamento:

```sh
cargo run -p dcx-cli -- loop status --id ID_DA_FILA
```

## Escolher um modelo e definir limites

Copie `.env.example` para `.env` na pasta do projeto e preencha as opções desejadas. O arquivo `.env` é ignorado pelo Git.

Você pode escolher `codex`, `claude`, `minimax` ou `auto` em `DCX_PROVIDER`. Para controlar uma execução, configure:

```env
DCX_RUN_MAX_MODEL_CALLS=40
DCX_RUN_MAX_COST_USD=2.00
DCX_RUN_MAX_DURATION_MINUTES=120
```

Os limites por tarefa e os limites globais da fila são aplicados juntos. O custo pode passar do teto pelo valor de uma chamada já iniciada, pois ele só é conhecido quando o modelo responde. Para usar um teto de custo, escolha um modelo que informe o custo; o Codex não fornece esse dado.

Para habilitar pesquisa na web durante a conversa, configure `DCX_CHAT_WEB=1` no `.env`. Sem essa opção, a conversa pesquisa apenas o projeto.

## Usar o app desktop

Para iniciar a versão de desenvolvimento:

```sh
cd desktop
pnpm desktop
```

No desktop, escolha o projeto, converse e acompanhe o trabalho. O acesso a repositórios fora do projeto pede confirmação e pode ser removido depois.

## Desenvolvimento

O dcode é desenvolvido em Rust; o app desktop usa Electron e TypeScript. Para validar as alterações localmente:

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```

Testes com provider real, evals e sensores de observabilidade são opt-in e podem gerar custo; veja `docs/testing.md`.
