#!/usr/bin/env node
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

import {
	dcxBinary,
	makeWorkspace,
	realProviderEnv,
	repoRoot,
	runProcess,
} from "../lib/real.mjs";
import { scoreContract } from "./contract-quality.mjs";

function parseArgs(argv) {
	const args = {
		suite: "evals/contracts/simple-file.json",
		provider: process.env.DCX_E2E_PROVIDER ?? "minimax",
		repeat: 1,
		dryRun: false,
		out: path.join(repoRoot, "target", "contract-eval-report.json"),
	};
	for (let i = 0; i < argv.length; i += 1) {
		const arg = argv[i];
		if (arg === "--suite") args.suite = argv[++i];
		else if (arg === "--provider") args.provider = argv[++i];
		else if (arg === "--repeat") args.repeat = Number(argv[++i]);
		else if (arg === "--out") args.out = argv[++i];
		else if (arg === "--dry-run") args.dryRun = true;
		else if (arg === "--help" || arg === "-h") {
			console.log(
				"Uso: node tools/eval/contract-run.mjs [--suite <json>] [--provider minimax|claude] [--repeat n] [--dry-run] [--out <json>]",
			);
			process.exit(0);
		} else throw new Error(`argumento desconhecido: ${arg}`);
	}
	if (!Number.isInteger(args.repeat) || args.repeat < 1) {
		throw new Error("--repeat precisa ser um inteiro >= 1");
	}
	return args;
}

async function readJson(file) {
	return JSON.parse(await readFile(file, "utf8"));
}

async function evaluateOnce(testCase, provider, env, binary, attempt) {
	const workspace = await makeWorkspace(`dcx-contract-eval-${attempt}-`);
	const started = Date.now();
	let processResult;
	try {
		processResult = await runProcess(
			binary,
			["run", "--request", testCase.request],
			{
				cwd: workspace,
				env: {
					...env,
					DCX_PROVIDER: provider,
					DCX_RUN_MAX_MODEL_CALLS: String(testCase.max_model_calls),
					DCX_RUN_MAX_COST_USD: String(testCase.max_cost_usd_per_attempt),
				},
			input: "s\n",
			timeoutMs: 8 * 60 * 1000,
			},
		);
		const entries = await readdir(path.join(workspace, ".dcx", "runs"), {
			withFileTypes: true,
		});
		const runs = entries.filter((entry) => entry.isDirectory());
		if (runs.length !== 1) {
			return {
				id: testCase.name,
				attempt,
				ok: false,
				infrastructure_ok: false,
				failures: [`esperava um contrato salvo; encontrei ${runs.length} run(s)`],
				duration_ms: Date.now() - started,
				cost_usd: null,
				cost_complete: false,
				stdout: processResult.stdout.slice(-5000),
				stderr: processResult.stderr.slice(-5000),
			};
		}
		const runDir = path.join(workspace, ".dcx", "runs", runs[0].name);
		const [contract, sensorSuite, metrics, run] = await Promise.all([
			readJson(path.join(runDir, "contract.json")),
			readJson(path.join(runDir, "sensor_suite.json")),
			readJson(path.join(runDir, "metrics.json")),
			readJson(path.join(runDir, "run.json")),
		]);
		const score = scoreContract(contract, sensorSuite, testCase.artifact, testCase);
		const failures = [...score.failures];
		if (metrics.call_count > testCase.max_model_calls) {
			failures.push(
				`limite de ${testCase.max_model_calls} chamadas de entendimento/contrato excedido; recebi ${metrics.call_count}`,
			);
		}
		if (metrics.cost_complete !== true) failures.push("custo do provider incompleto");
		if (metrics.known_cost_usd > testCase.max_cost_usd_per_attempt) {
			failures.push("custo por tentativa excedeu o teto da eval");
		}
		if (!run.states?.includes("awaiting_human")) {
			failures.push("contrato não chegou ao gate de aprovação humana");
		}
		return {
			id: testCase.name,
			attempt,
			ok: failures.length === 0,
			infrastructure_ok:
				metrics.cost_complete === true && run.states?.includes("awaiting_human"),
			failures,
			provider,
			model: metrics.calls?.at(-1)?.model ?? null,
			criteria_count: contract.criteria?.length ?? 0,
			sensor_count: sensorSuite.sensors?.length ?? 0,
			duration_ms: Date.now() - started,
			model_calls: metrics.call_count,
			cost_usd: metrics.known_cost_usd,
			cost_complete: metrics.cost_complete === true,
			contract,
			sensor_suite: sensorSuite,
			stdout: processResult.stdout.slice(-5000),
			stderr: processResult.stderr.slice(-5000),
		};
	} catch (error) {
		return {
			id: testCase.name,
			attempt,
			ok: false,
			infrastructure_ok: false,
			failures: [error.message],
			duration_ms: Date.now() - started,
			cost_usd: null,
			cost_complete: false,
			stdout: processResult?.stdout.slice(-5000) ?? "",
			stderr: processResult?.stderr.slice(-5000) ?? "",
		};
	} finally {
		await rm(workspace, { recursive: true, force: true });
	}
}

export async function runContractEval({
	suite,
	provider,
	repeat,
	env,
	binary,
	onProgress = async () => {},
}) {
	const results = [];
	const startedAt = new Date().toISOString();
	for (let attempt = 1; attempt <= repeat; attempt += 1) {
		const result = await evaluateOnce(suite, provider, env, binary, attempt);
		results.push(result);
		const snapshot = createReport(suite, provider, repeat, results, startedAt);
		await onProgress(snapshot);
		if (!result.infrastructure_ok) break;
	}
	return createReport(suite, provider, repeat, results, startedAt);
}

function createReport(suite, provider, repeat, results, startedAt) {
	const costs = results.map((result) => result.cost_usd);
	const durations = results
		.map((result) => result.duration_ms)
		.sort((a, b) => a - b);
	return {
		ok: results.length === repeat && results.every((result) => result.ok),
		suite: suite.name,
		provider,
		repeat,
		started_at: startedAt,
		summary: {
			passed: results.filter((result) => result.ok).length,
			total: results.length,
			cost_usd: costs.reduce((sum, value) => sum + (value ?? 0), 0),
			cost_complete: costs.every((value) => typeof value === "number") && results.every((result) => result.cost_complete),
			latency_ms: {
				p50: durations[Math.floor((durations.length - 1) * 0.5)] ?? null,
				p95: durations[Math.ceil(durations.length * 0.95) - 1] ?? null,
			},
		},
		results,
	};
}

async function main() {
	const args = parseArgs(process.argv.slice(2));
	const suite = await readJson(path.resolve(args.suite));
	if (suite.schema_version !== 1 || !suite.name || !suite.request) {
		throw new Error("suite de contrato inválida");
	}
	if (args.dryRun) {
		console.log(
			JSON.stringify({ suite: suite.name, provider: args.provider, repeat: args.repeat, dry_run: true }, null, 2),
		);
		return;
	}
	if (process.env.DCX_EVAL !== "1") {
		throw new Error("defina DCX_EVAL=1 para chamar um provider real");
	}
	const { provider, env } = realProviderEnv(args.provider);
	const reportPath = path.resolve(args.out);
	const saveProgress = async (report) => {
		await mkdir(path.dirname(reportPath), { recursive: true });
		await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
	};
	const report = await runContractEval({
		suite,
		provider,
		repeat: args.repeat,
		env,
		binary: await dcxBinary(),
		onProgress: saveProgress,
	});
	await saveProgress(report);
	console.log(JSON.stringify(report, null, 2));
	console.log(`Relatório: ${reportPath}`);
	if (!report.ok) process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
	main().catch((error) => {
		console.error(`FALHA: ${error.message}`);
		process.exitCode = 1;
	});
}
