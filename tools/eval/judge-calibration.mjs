#!/usr/bin/env node
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { repoRoot } from "../lib/real.mjs";
import { judgeResponse } from "./judge.mjs";

function parseArgs(argv) {
	const args = {
		file: "evals/judge-calibration.json",
		provider: process.env.DCX_E2E_PROVIDER || "minimax",
		repeat: 1,
		dryRun: false,
		out: path.join(repoRoot, "target", "judge-calibration-report.json"),
	};
	for (let index = 0; index < argv.length; index += 1) {
		const arg = argv[index];
		if (arg === "--suite") args.file = argv[++index];
		else if (arg === "--provider") args.provider = argv[++index];
		else if (arg === "--repeat") args.repeat = Number(argv[++index]);
		else if (arg === "--out") args.out = argv[++index];
		else if (arg === "--dry-run") args.dryRun = true;
		else throw new Error(`argumento desconhecido: ${arg}`);
	}
	if (!Number.isInteger(args.repeat) || args.repeat < 1 || args.repeat > 10) {
		throw new Error("--repeat precisa ser um inteiro entre 1 e 10");
	}
	if (!new Set(["claude", "minimax"]).has(args.provider)) {
		throw new Error("--provider precisa ser claude ou minimax");
	}
	return args;
}

function validateSuite(suite) {
	if (suite.schema_version !== 1 || !suite.name) {
		throw new Error("suite de calibração inválida: schema_version ou name ausente");
	}
	if (!Array.isArray(suite.cases) || suite.cases.length === 0) {
		throw new Error("suite de calibração precisa de casos");
	}
	for (const item of suite.cases) {
		if (!item.id || typeof item.prompt !== "string" || typeof item.response !== "string") {
			throw new Error("cada caso precisa de id, prompt e response");
		}
		if (typeof item.expected_pass !== "boolean") {
			throw new Error(`caso ${item.id} precisa de expected_pass booleano`);
		}
	}
}

export function scoreCalibration(results) {
	const mismatches = results.filter((item) => item.passed !== item.expected_pass);
	return {
		total: results.length,
		passed: results.length - mismatches.length,
		failed: mismatches.length,
		pass_rate: results.length ? (results.length - mismatches.length) / results.length : 0,
		cost_usd: results.reduce((sum, item) => sum + (item.cost_usd ?? 0), 0),
		cost_complete: results.every((item) => typeof item.cost_usd === "number"),
		latency_ms: results.map((item) => item.duration_ms),
	};
}

async function main() {
	const args = parseArgs(process.argv.slice(2));
	const suite = JSON.parse(await readFile(args.file, "utf8"));
	validateSuite(suite);
	if (args.dryRun) {
		console.log(JSON.stringify({ suite: suite.name, provider: args.provider, repeat: args.repeat, cases: suite.cases.map(({ id, expected_pass }) => ({ id, expected_pass })) }, null, 2));
		return;
	}
	if (process.env.DCX_EVAL !== "1") {
		throw new Error("defina DCX_EVAL=1 para executar a calibração com provider real");
	}

	const startedAt = new Date().toISOString();
	const started = Date.now();
	const results = [];
	for (let repeat = 0; repeat < args.repeat; repeat += 1) {
		for (const item of suite.cases) {
			try {
				const judged = await judgeResponse({ testCase: item, rubric: suite.rubric, provider: args.provider });
				results.push({
					id: item.id,
					repeat: repeat + 1,
					expected_pass: item.expected_pass,
					passed: judged.passed,
					match: judged.passed === item.expected_pass,
					scores: judged.scores,
					issue_codes: judged.issue_codes,
					cost_usd: judged.cost_usd,
					model: judged.model,
					duration_ms: judged.duration_ms,
					usage: judged.usage,
				});
			} catch (error) {
				results.push({
					id: item.id,
					repeat: repeat + 1,
					expected_pass: item.expected_pass,
					passed: false,
					match: false,
					error: error.message,
					cost_usd: null,
					duration_ms: null,
				});
			}
		}
	}
	const report = {
		suite: suite.name,
		provider: args.provider,
		model: results.find((item) => item.model)?.model ?? null,
		started_at: startedAt,
		duration_ms: Date.now() - started,
		repeat: args.repeat,
		results,
		summary: scoreCalibration(results),
	};
	await mkdir(path.dirname(args.out), { recursive: true });
	await writeFile(args.out, `${JSON.stringify(report, null, 2)}\n`);
	console.log(`Calibração: ${report.summary.passed}/${report.summary.total} acertos`);
	console.log(`Custo: US$ ${report.summary.cost_usd.toFixed(4)}${report.summary.cost_complete ? "" : " (incompleto)"}`);
	console.log(`Relatório: ${args.out}`);
	for (const item of results.filter((result) => !result.match)) {
		console.log(`Falha ${item.id} (repetição ${item.repeat}): ${item.error ?? JSON.stringify(item.scores)}`);
	}
	if (report.summary.failed > 0 || !report.summary.cost_complete) process.exitCode = 1;
}

if (
	process.argv[1] &&
	pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
	main().catch((error) => {
		console.error(error.message);
		process.exitCode = 1;
	});
}
