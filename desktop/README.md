# Adaptador de desktop do Harness — código-fonte

Interface de três colunas em Electron, com o núcleo de agente que ela aciona.
Este pacote é o **código-fonte** da interface, mais o grafo de módulos que ela
importa — o mínimo para compilar e rodar, e não o repositório inteiro.

## O que está aqui

| Caminho | O quê |
|---|---|
| `src/desktop/` | a interface: `main.ts` (processo principal), `preload.cjs` (ponte), `renderer/` (HTML, CSS, fontes, renderer TS) |
| `src/agent/` | o ciclo do agente e o permission gate |
| `src/provider/` | adaptadores de provider e os tipos de requisição/resposta |
| `src/tools/` | registro de ferramentas e implementações (bash, fs, search, shell) |
| `src/fs/` | sandbox de sistema de arquivos |
| `src/testing/` | provider roteirizado, para rodar a interface sem credencial |
| `src/version.ts` | versão do pacote |
| `tests/` | os testes dos módulos acima |
| `core/` | só o pacote `internal/protocol` em Go, mais `tygo.yaml` — existe para **regerar** `src/desktop/protocol/generated.ts` |
| `tsconfig*.json`, `biome.json`, `tools/copy-assets.mjs` | configuração de build |

### O que **não** está aqui, e por quê

- **`core/` completo** (o núcleo em Go do produto, ~47 mil linhas) — este
  pacote é a interface. O núcleo é outro pacote.
- **`src/harness/`** — cliente dos registros de construção do Harness. Não é
  importado pela interface.
- **`src/cli/`, `src/tui/`** — adaptadores de terminal. Também não são
  importados pela interface.
- **`.harness/`** — o contrato e os registros de construção do projeto.
- **`examples/`, `dist/`, `node_modules/`** — gerados ou reproduzíveis.

## Como rodar

```bash
pnpm install
pnpm desktop
```

`pnpm desktop` compila e abre a janela. O `pnpm install` precisa baixar o
Electron (~250 MB); é o passo mais lento.

### Usar o Harness local

Inicie o daemon Rust na raiz do projeto e informe o workspace que a janela deve
usar:

```bash
cargo run -p harness-cli -- serve --bind 127.0.0.1:8787
HARNESS_WORKSPACE=/caminho/do/projeto pnpm desktop
```

O desktop usa `HARNESS_URL` (padrão `http://127.0.0.1:8787`) e conversa com o
Harness por `/chat`. A sessão fica no daemon e o renderer não recebe acesso ao
filesystem. `REPL_DESKTOP_LOCAL=1` mantém o agente local da base apenas para
testes/demo, sem ser o caminho de produção.

Para rodar sem credencial de modelo, o provider roteirizado está ligado por
variável de ambiente:

```bash
REPL_SCRIPTED=1 npx electron dist/desktop/main.js
```

## Como compilar

```bash
pnpm build
```

Compila `src/**/*.ts` para `dist/` e copia os assets do renderer (HTML, CSS e
as três fontes web em `.woff2`) para `dist/desktop/renderer/`.

## O gate

```bash
pnpm gate
```

Lint, typecheck, testes e build do TypeScript, mais `go test ./...` do pacote
de protocolo.

## `src/desktop/protocol/generated.ts`

Os tipos do protocolo **não são escritos à mão**: são gerados dos tipos Go em
`core/internal/protocol` pelo `tygo`. O arquivo já vem gerado neste pacote.

Para regerar (precisa de Go instalado e de rede na primeira vez):

```bash
pnpm gen:protocol
```

Para conferir se o arquivo commitado está velho (precisa de git):

```bash
pnpm check:protocol
```

O `core/go.mod` deste pacote é mínimo de propósito: o pacote de protocolo usa
só a biblioteca padrão, então ele compila e resolve sem dependência externa.

## Superfície da interface

- `contextIsolation: true`, `nodeIntegration: false`.
- O preload expõe apenas `run`, `cancel`, `context` e `onEvent`.
- O renderer **não** guarda estado: consome snapshots e eventos.
- `src/desktop/renderer/mock.ts` e `simulation.ts` são o fallback explícito de
  modo demo — dados separados do renderer, para troca por dados reais.
- Tipografia: tokens reais do `claude.ai` (`--cds-*`), com fontes substitutas
  OFL vendorizadas em `src/desktop/renderer/fonts/`.

## Licença

Código do autor (MIT). As fontes em `src/desktop/renderer/fonts/` são
substitutas OFL vendorizadas — `Geist` (parente real do `anthropic-sans`),
`Source Serif 4` e `JetBrains Mono`.
