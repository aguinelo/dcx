# Regras do projeto dcode

## Teste primeiro, sempre

Nenhuma mudança de comportamento entra sem antes ter um teste ou eval que a prove.

1. **Antes de alterar qualquer código**, escreva o teste (ou o caso de eval) que descreve o comportamento desejado e rode-o para ver falhar pelo motivo certo. Um teste que já passa antes da mudança não prova nada.
2. **Implemente o mínimo** para fazê-lo passar, rodando o teste a cada passo.
3. **Só está pronto quando** o teste novo passa e toda a suíte continua verde: `cargo fmt --all -- --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` e, no desktop, `npm run typecheck` e `npm test`.
4. **Bug encontrado vira teste de regressão** antes do conserto.

Qual camada usar:

- **Lógica Rust**: teste unitário no módulo ou teste de integração em `crates/dcx-cli/tests/` (o CLI de verdade com provider falso via `DCX_CLAUDE_BIN`).
- **Desktop**: `desktop/tests/`.
- **Comportamento do modelo** (prompt, idioma, uso de ferramentas, recusa, formato da resposta): caso novo em `evals/` antes de mexer no prompt. Rode com provider real e repetição (`--repeat 3`), e compare com a linha de base em `docs/testing.md`. Mudança de prompt sem eval não entra.
- **Logs, exceções e desvios**: sensores em `tools/observe/`. Ao editar um sensor, renove a trava com `node tools/observe/lock.mjs`.

"Satisfatório" quer dizer: o teste novo passa de forma estável (os evals em todas as repetições), nenhum teste existente regrediu e, quando for um eval, latência e custo continuam perto da linha de base. Se algo não puder ser testado automaticamente, diga isso explicitamente em vez de declarar pronto.

## Documentação de uso

O `README.md` e os guias de uso descrevem o que o produto faz e como usar, em linguagem simples, sem entrar no funcionamento interno. Mantenha-os atualizados a cada feature.

## Idioma

Interface, mensagens e documentação em português do Brasil. Identificadores e código em inglês.
