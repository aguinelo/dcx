import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { access, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import { runCheck, validateRunEvidence } from "./real.mjs";

test("importing the e2e helpers does not start the real-provider runner", () => {
	const imported = spawnSync(
		process.execPath,
		["--input-type=module", "-e", 'await import("./tools/e2e/real.mjs")'],
		{ cwd: process.cwd(), encoding: "utf8", env: { ...process.env, DCX_E2E: "0" } },
	);
	assert.equal(imported.status, 0);
	assert.equal(imported.stdout, "");
	assert.equal(imported.stderr, "");
});

test("full-run evidence requires an approved terminal state and expected file", () => {
	const result = validateRunEvidence({
		exitCode: 0,
		workspaceFile: "DCX_RUN_E2E_OK\n",
		expectedContent: "DCX_RUN_E2E_OK\n",
		run: { states: ["understanding", "awaiting_merge"] },
		verdict: { approved: true },
		metrics: { call_count: 5, cost_complete: true, known_cost_usd: 0.08 },
		files: ["resultado.txt"],
		expectedFile: "resultado.txt",
	});

	assert.equal(result.ok, true);
	assert.deepEqual(result.failures, []);
});

test("full-run evidence rejects a failed run even when it produced the file", () => {
	const result = validateRunEvidence({
		exitCode: 1,
		workspaceFile: "DCX_RUN_E2E_OK\n",
		expectedContent: "DCX_RUN_E2E_OK\n",
		run: { states: ["understanding", "failed"] },
		verdict: { approved: false },
		metrics: { call_count: 5, cost_complete: true, known_cost_usd: 0.08 },
		files: ["resultado.txt"],
		expectedFile: "resultado.txt",
	});

	assert.equal(result.ok, false);
	assert.ok(result.failures.includes("run não terminou em estado aprovado"));
});

test("full-run evidence fails closed when the provider omits cost", () => {
	const result = validateRunEvidence({
		exitCode: 0,
		workspaceFile: "DCX_RUN_E2E_OK\n",
		expectedContent: "DCX_RUN_E2E_OK\n",
		run: { states: ["awaiting_merge"] },
		verdict: { approved: true },
		metrics: { call_count: 5, cost_complete: false, known_cost_usd: 0.08 },
		files: ["resultado.txt"],
		expectedFile: "resultado.txt",
	});

	assert.equal(result.ok, false);
	assert.ok(result.failures.includes("custo do provider incompleto"));
});

test("full-run evidence checks exact file bytes, including the final newline", () => {
	const result = validateRunEvidence({
		exitCode: 0,
		workspaceFile: "DCX_RUN_E2E_OK",
		expectedContent: "DCX_RUN_E2E_OK",
		run: { states: ["awaiting_merge"] },
		verdict: { approved: true },
		metrics: { call_count: 5, cost_complete: true, known_cost_usd: 0.08 },
		files: ["resultado.txt"],
		expectedFile: "resultado.txt",
	});

	assert.equal(result.ok, true);
});

test("an interrupted full run is represented as a reportable failed check", () => {
	const result = validateRunEvidence({
		exitCode: 143,
		workspaceFile: null,
		run: { states: ["understanding", "contracting"] },
		verdict: null,
		metrics: { call_count: 3, cost_complete: true, known_cost_usd: 0.7665058 },
		files: [],
		expectedFile: "resultado.txt",
	});

	assert.equal(result.ok, false);
	assert.ok(result.failures.length >= 3);
});

test("a failed real-run check still writes its observability bundle", async () => {
	const fakeRoot = await mkdtemp(path.join(tmpdir(), "dcx-e2e-fake-bin-"));
	const fakeBinary = path.join(fakeRoot, "dcx-fake.mjs");
	const payload = JSON.stringify({
		run: { states: ["new", "failed"] },
		verdict: { approved: false, issues: [] },
		metrics: {
			call_count: 1,
			known_cost_usd: 0.01,
			cost_complete: true,
			total_duration_ms: 100,
			calls: [{ provider: "minimax", model: "test" }],
		},
	});
	const fake = `#!/usr/bin/env node
import { mkdirSync, writeFileSync } from "node:fs";
const root = ".dcx/runs/fake-run";
mkdirSync(root, { recursive: true });
const payload = ${JSON.stringify(payload)};
const data = JSON.parse(payload);
writeFileSync(root + "/run.json", JSON.stringify(data.run));
writeFileSync(root + "/verdict.json", JSON.stringify(data.verdict));
writeFileSync(root + "/metrics.json", JSON.stringify(data.metrics));
writeFileSync(root + "/step_reports.json", "[]");
writeFileSync(root + "/sensor_results.json", "[]");
writeFileSync(root + "/delivery_report.json", JSON.stringify({ summary: "fake failure" }));
writeFileSync(root + "/events.jsonl", JSON.stringify({ sequence: 1, event_type: "run.failed", payload: {} }) + "\\n");
console.error("Erro: falha simulada do run");
process.exit(1);
`;
	try {
		await writeFile(fakeBinary, fake, { mode: 0o755 });
		const probe = spawnSync(fakeBinary, [], { cwd: fakeRoot, encoding: "utf8" });
		assert.equal(probe.status, 1, probe.stderr);
		await access(path.join(fakeRoot, ".dcx", "runs", "fake-run", "run.json"));
		const check = await runCheck("minimax", process.env, fakeBinary);
		assert.equal(check.ok, false);
		assert.equal(check.run_id, "fake-run");
		assert.equal(check.observability_ok, false);
		const bundle = JSON.parse(await readFile(path.resolve("target/run-bundle.json"), "utf8"));
		assert.equal(bundle.ok, false);
	} finally {
		await rm(fakeRoot, { recursive: true, force: true });
	}
});

test("a complete provider run passes evidence collection and observation", async () => {
	const fakeRoot = await mkdtemp(path.join(tmpdir(), "dcx-e2e-success-"));
	const fakeBinary = path.join(fakeRoot, "dcx-fake.mjs");
	const calls = [
		{
			role: "planner",
			provider: "minimax",
			model: "MiniMax-M3",
			duration_ms: 100,
			cost_usd: 0.01,
			input_tokens: 10,
			output_tokens: 5,
		},
	];
	const fake = `#!/usr/bin/env node
import { mkdirSync, writeFileSync } from "node:fs";
const root = ".dcx/runs/fake-approved";
mkdirSync(root, { recursive: true });
writeFileSync("resultado.txt", "DCX_RUN_E2E_OK");
writeFileSync(root + "/run.json", JSON.stringify({ states: ["new", "awaiting_merge"] }));
writeFileSync(root + "/verdict.json", JSON.stringify({ approved: true, issues: [] }));
writeFileSync(root + "/metrics.json", JSON.stringify({ call_count: 1, known_cost_usd: 0.01, cost_complete: true, total_duration_ms: 100, calls: ${JSON.stringify(calls)} }));
writeFileSync(root + "/step_reports.json", JSON.stringify([{ files_changed: ["resultado.txt"] }]));
writeFileSync(root + "/sensor_results.json", JSON.stringify([{ sensor_id: "visible-result", criterion_id: "result", passed: true, evidence: "ok" }]));
writeFileSync(root + "/delivery_report.json", JSON.stringify({ summary: "resultado aprovado" }));
const events = [
  { sequence: 1, event_type: "contract.approved", payload: {} },
  { sequence: 2, event_type: "step.completed", payload: { files: ["resultado.txt"] } },
  { sequence: 3, event_type: "changes.applied", payload: {} },
];
writeFileSync(root + "/events.jsonl", events.map((event) => JSON.stringify(event)).join("\\n") + "\\n");
`;
	try {
		await writeFile(fakeBinary, fake, { mode: 0o755 });
		const check = await runCheck("minimax", process.env, fakeBinary);
		assert.equal(check.ok, true, check.failures.join("; "));
		assert.equal(check.observability_ok, true);
		assert.equal(check.final_state, "awaiting_merge");
	} finally {
		await rm(fakeRoot, { recursive: true, force: true });
	}
});

test("a provider that fails before creating artifacts returns a failed check", async () => {
	const fakeRoot = await mkdtemp(path.join(tmpdir(), "dcx-e2e-no-run-"));
	const fakeBinary = path.join(fakeRoot, "dcx-fake.mjs");
	try {
		await writeFile(
			fakeBinary,
			'#!/usr/bin/env node\nconsole.error("Erro: provider indisponível");\nprocess.exit(1);\n',
			{ mode: 0o755 },
		);
		const check = await runCheck("minimax", process.env, fakeBinary);
		assert.equal(check.ok, false);
		assert.ok(check.failures.includes("esperava um run, encontrei 0"));
	} finally {
		await rm(fakeRoot, { recursive: true, force: true });
	}
});

test("a partial run with missing metrics still returns a failed check", async () => {
	const fakeRoot = await mkdtemp(path.join(tmpdir(), "dcx-e2e-partial-run-"));
	const fakeBinary = path.join(fakeRoot, "dcx-fake.mjs");
	try {
		await writeFile(
			fakeBinary,
			'#!/usr/bin/env node\nimport { mkdirSync, writeFileSync } from "node:fs";\nmkdirSync(".dcx/runs/partial", { recursive: true });\nwriteFileSync(".dcx/runs/partial/run.json", JSON.stringify({ states: ["contracting"] }));\nprocess.exit(1);\n',
			{ mode: 0o755 },
		);
		const check = await runCheck("minimax", process.env, fakeBinary);
		assert.equal(check.ok, false);
		assert.equal(check.run_id, "partial");
		assert.equal(check.cost_complete, false);
	} finally {
		await rm(fakeRoot, { recursive: true, force: true });
	}
});
