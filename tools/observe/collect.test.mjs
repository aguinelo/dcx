import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"..",
	"..",
);
const collect = path.join(repoRoot, "tools", "observe", "collect.mjs");
const observe = path.join(repoRoot, "tools", "observe", "run.mjs");

function runNode(args) {
	return spawnSync(process.execPath, args, {
		cwd: repoRoot,
		encoding: "utf8",
	});
}

async function writeJson(file, value) {
	await writeFile(file, `${JSON.stringify(value, null, 2)}\n`);
}

async function makeRun(overrides = {}) {
	const directory = await mkdtemp(path.join(tmpdir(), "observe-run-"));
	const approved = overrides.approved ?? true;
	const providers = overrides.providers ?? ["claude"];
	const calls = providers.map((provider, index) => ({
		role: index === 0 ? "planner" : "executor",
		provider,
		model: "test-model",
		duration_ms: 100,
		cost_usd: 0.01,
		input_tokens: 10,
		output_tokens: 5,
	}));
	await writeJson(path.join(directory, "run.json"), {
		schema_version: 1,
		states: ["new", "understanding", approved ? "awaiting_merge" : "failed"],
	});
	await writeJson(path.join(directory, "metrics.json"), {
		schema_version: 1,
		calls,
		started_at_unix_ms: Date.now(),
		call_count: calls.length,
		total_duration_ms: 1200,
		known_cost_usd: calls.length * 0.01,
		cost_complete: true,
	});
	await writeJson(path.join(directory, "verdict.json"), {
		schema_version: 1,
		approved,
		criteria: [
			{
				criterion_id: "criterion-1",
				passed: approved,
				evidence: [approved ? "ok" : "falhou"],
			},
		],
		issues: approved
			? []
			: [
					{
						criterion_id: "criterion-1",
						description: "implementação incompleta",
						severity: "blocker",
					},
				],
		notes: [],
	});
	await writeJson(path.join(directory, "delivery_report.json"), {
		schema_version: 1,
		run_id: path.basename(directory),
		summary: approved ? "Implementação aprovada." : "Execução reprovada.",
		criteria_passed: approved ? 1 : 0,
		criteria_total: 1,
		sensor_results: [],
		states: [],
	});
	await writeJson(path.join(directory, "sensor_results.json"), [
		{
			sensor_id: "visible-criterion-1",
			criterion_id: "criterion-1",
			passed: approved,
			evidence: approved ? "ok" : "implementação incompleta",
		},
	]);
	await writeJson(path.join(directory, "step_reports.json"), [
		{
			schema_version: 1,
			step_id: "integration",
			attempts: 1,
			summary: approved ? "ok" : "falhou",
			files_changed: ["implemented.txt"],
			decisions: [],
			disputes: [],
		},
	]);
	const events = [
		{
			schema_version: 1,
			run_id: path.basename(directory),
			sequence: 1,
			event_type: "contract.approved",
			payload: {},
		},
		{
			schema_version: 1,
			run_id: path.basename(directory),
			sequence: 2,
			event_type: "step.completed",
			payload: { step_id: "integration", files: ["implemented.txt"] },
		},
		{
			schema_version: 1,
			run_id: path.basename(directory),
			sequence: 3,
			event_type: approved ? "changes.applied" : "run.rejected",
			payload: {},
		},
	];
	await writeFile(
		path.join(directory, "events.jsonl"),
		`${events.map((event) => JSON.stringify(event)).join("\n")}\n`,
	);
	return directory;
}

async function collectBundle(directory, extra = []) {
	const bundle = path.join(directory, "bundle.json");
	const result = runNode([
		collect,
		"--run",
		directory,
		"--out",
		bundle,
		...extra,
	]);
	assert.equal(result.status, 0, result.stderr);
	return { bundle, data: JSON.parse(await readFile(bundle, "utf8")) };
}

test("run aprovado vira bundle e passa nos seis sensores", async () => {
	const directory = await makeRun();
	try {
		const { bundle, data } = await collectBundle(directory);
		assert.equal(data.ok, true);
		assert.equal(data.provider, "claude");
		assert.equal(data.checks.length, 7);
		const report = path.join(directory, "report.json");
		const result = runNode([observe, "--bundle", bundle, "--out", report]);
		assert.equal(result.status, 0, result.stderr);
		assert.match(result.stdout, /6\/6/);
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
});

test("run sem veredito produz bundle reprovado", async () => {
	const directory = await makeRun();
	try {
		await rm(path.join(directory, "verdict.json"));
		const { bundle, data } = await collectBundle(directory);
		assert.equal(data.ok, false);
		assert.equal(
			data.checks.find((check) => check.name === "artifacts").ok,
			false,
		);
		assert.equal(
			data.checks.find((check) => check.name === "verdict").ok,
			false,
		);
		const report = path.join(directory, "report.json");
		const result = runNode([observe, "--bundle", bundle, "--out", report]);
		assert.equal(result.status, 1);
		const strict = runNode([
			collect,
			"--run",
			directory,
			"--out",
			path.join(directory, "strict.json"),
			"--strict",
		]);
		assert.equal(strict.status, 1);
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
});

test("múltiplos providers exigem fallback declarado", async () => {
	const directory = await makeRun({ providers: ["claude", "minimax"] });
	try {
		const rejected = await collectBundle(directory);
		assert.equal(rejected.data.ok, false);
		assert.equal(rejected.data.provider, "mixed");
		assert.equal(
			rejected.data.checks.find((check) => check.name === "provider").ok,
			false,
		);
		const report = path.join(directory, "report.json");
		const failed = runNode([
			observe,
			"--bundle",
			rejected.bundle,
			"--out",
			report,
		]);
		assert.equal(failed.status, 1);
		assert.match(failed.stdout, /sem fallback declarado/);

		const allowed = await collectBundle(directory, ["--allow-fallback"]);
		assert.equal(allowed.data.ok, true);
		assert.equal(allowed.data.fallback.used, true);
		const passed = runNode([
			observe,
			"--bundle",
			allowed.bundle,
			"--out",
			report,
		]);
		assert.equal(passed.status, 0, passed.stderr);
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
});

test("run reprovado é observado com --expect rejected", async () => {
	const directory = await makeRun({ approved: false });
	try {
		const { bundle, data } = await collectBundle(directory, [
			"--expect",
			"rejected",
		]);
		assert.equal(data.ok, true);
		const report = path.join(directory, "report.json");
		const result = runNode([observe, "--bundle", bundle, "--out", report]);
		assert.equal(result.status, 0, result.stderr);
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
});
