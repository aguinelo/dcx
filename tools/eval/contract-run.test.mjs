import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import { runContractEval } from "./contract-run.mjs";

test("contract eval records saved artifacts before the call budget stops the run", async () => {
	const root = await mkdtemp(path.join(tmpdir(), "dcx-contract-eval-fake-"));
	const binary = path.join(root, "dcx-fake.mjs");
	const criteria = [{ id: "file-content", description: "resultado.txt contém o texto pedido." }];
	const sensors = [
		{ id: "visible", visibility: "visible", check: "Ler resultado.txt e comparar o conteúdo." },
		{ id: "hidden", visibility: "hidden", check: "Comparar os bytes de resultado.txt." },
	];
	const metrics = {
		call_count: 2,
		known_cost_usd: 0.05,
		cost_complete: true,
		calls: [
			{ provider: "minimax", model: "MiniMax-M3", cost_usd: 0.02 },
			{ provider: "minimax", model: "MiniMax-M3", cost_usd: 0.03 },
		],
	};
	const fake = `#!/usr/bin/env node
import { mkdirSync, writeFileSync } from "node:fs";
const dir = ".dcx/runs/run-fake";
mkdirSync(dir, { recursive: true });
writeFileSync(dir + "/contract.json", JSON.stringify({ objective: "Criar resultado.txt.", criteria: ${JSON.stringify(criteria)}, interface: {} }));
writeFileSync(dir + "/sensor_suite.json", JSON.stringify({ sensors: ${JSON.stringify(sensors)} }));
writeFileSync(dir + "/metrics.json", JSON.stringify(${JSON.stringify(metrics)}));
writeFileSync(dir + "/run.json", JSON.stringify({ states: ["new", "contracting", "awaiting_human", "acceptance_authoring"] }));
console.error("Falha esperada: limite de 2 chamadas de modelo atingido");
process.exit(1);
`;
	try {
		await writeFile(binary, fake, { mode: 0o755 });
		const report = await runContractEval({
			suite: {
				name: "simple-file-contract",
				request: "Crie resultado.txt com DCX_RUN_E2E_OK.",
				artifact: "resultado.txt",
				max_model_calls: 3,
				max_cost_usd_per_attempt: 1,
			},
			provider: "minimax",
			repeat: 1,
			env: process.env,
			binary,
		});
		assert.equal(report.ok, true);
		assert.equal(report.summary.cost_usd, 0.05);
		assert.equal(report.results[0].criteria_count, 1);
	} finally {
		await rm(root, { recursive: true, force: true });
	}
});

test("contract eval saves progress and stops after an attempt with no artifacts", async () => {
	const root = await mkdtemp(path.join(tmpdir(), "dcx-contract-eval-timeout-"));
	const binary = path.join(root, "dcx-fake.mjs");
	const fake = `#!/usr/bin/env node
console.error("provider timeout before contract artifacts");
process.exit(1);
`;
	const progress = [];
	try {
		await writeFile(binary, fake, { mode: 0o755 });
		const report = await runContractEval({
			suite: {
				name: "simple-file-contract",
				request: "Crie resultado.txt com DCX_RUN_E2E_OK.",
				artifact: "resultado.txt",
				max_model_calls: 2,
				max_cost_usd_per_attempt: 1,
			},
			provider: "minimax",
			repeat: 3,
			env: process.env,
			binary,
			onProgress: async (snapshot) => progress.push(snapshot),
		});
		assert.equal(report.ok, false);
		assert.equal(report.results.length, 1);
		assert.equal(progress.length, 1);
		assert.equal(progress[0].summary.total, 1);
	} finally {
		await rm(root, { recursive: true, force: true });
	}
});

test("contract eval scores a contract that was saved before the process was killed", async () => {
	const root = await mkdtemp(path.join(tmpdir(), "dcx-contract-eval-killed-"));
	const binary = path.join(root, "dcx-fake.mjs");
	const criteria = [{ id: "file-content", description: "resultado.txt contém o texto pedido." }];
	const sensors = [
		{ id: "visible", visibility: "visible", check: "Comparar o conteúdo." },
	];
	const metrics = { call_count: 1, known_cost_usd: 0.02, cost_complete: true, calls: [] };
	const fake = `#!/usr/bin/env node
import { mkdirSync, writeFileSync } from "node:fs";
const dir = ".dcx/runs/run-fake";
mkdirSync(dir, { recursive: true });
writeFileSync(dir + "/contract.json", JSON.stringify({ objective: "Criar resultado.txt.", criteria: ${JSON.stringify(criteria)}, interface: {} }));
writeFileSync(dir + "/sensor_suite.json", JSON.stringify({ sensors: ${JSON.stringify(sensors)} }));
writeFileSync(dir + "/metrics.json", JSON.stringify(${JSON.stringify(metrics)}));
writeFileSync(dir + "/run.json", JSON.stringify({ states: ["new", "contracting", "awaiting_human"] }));
await new Promise(r => setTimeout(r, 120_000));
`;
	try {
		await writeFile(binary, fake, { mode: 0o755 });
		const report = await runContractEval({
			suite: {
				name: "simple-file-contract",
				request: "Crie resultado.txt com DCX_RUN_E2E_OK.",
				artifact: "resultado.txt",
				max_model_calls: 2,
				max_cost_usd_per_attempt: 1,
			},
			provider: "minimax",
			repeat: 1,
			env: process.env,
			binary,
			timeoutMs: 500,
		});
		assert.equal(report.results[0].infrastructure_ok, true, "deve reconhecer o contrato salvo mesmo após timeout");
		assert.equal(report.results[0].criteria_count, 1, "deve contar critérios do contrato salvo");
	} finally {
		await rm(root, { recursive: true, force: true });
	}
});

test("contract eval preserves CLI diagnostics when artifacts were not saved", async () => {
	const root = await mkdtemp(path.join(tmpdir(), "dcx-contract-eval-diagnostics-"));
	const binary = path.join(root, "dcx-fake.mjs");
	try {
		await writeFile(binary, `#!/usr/bin/env node\nconsole.log("stdout diagnostic"); console.error("stderr diagnostic"); process.exit(1);\n`, { mode: 0o755 });
		const report = await runContractEval({
			suite: { name: "simple-file-contract", request: "test", artifact: "x", max_model_calls: 2, max_cost_usd_per_attempt: 1 },
			provider: "minimax", repeat: 1, env: process.env, binary,
		});
		assert.match(report.results[0].stdout, /stdout diagnostic/);
		assert.match(report.results[0].stderr, /stderr diagnostic/);
	} finally {
		await rm(root, { recursive: true, force: true });
	}
});
