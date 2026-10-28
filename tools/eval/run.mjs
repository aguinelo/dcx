#!/usr/bin/env node
import { access, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";

import {
	changedPaths,
	dcxBinary,
	latestSession,
	makeWorkspace,
	realProviderEnv,
	repoRoot,
	runProcess,
	snapshotProject,
} from "../lib/real.mjs";
import { scoreCase, summarize } from "./scorer.mjs";
import {
	judgeResponse,
	summarizeJudgments,
	validateRubric,
} from "./judge.mjs";
import { pathToFileURL } from "node:url";

export function combineEvalScore(deterministic, judgment = null) {
	return deterministic && (judgment === null || judgment.passed === true);
}

export function describeFailure(result) {
	const parts = [];
	if (result.error) parts.push(result.error);
	if (Array.isArray(result.failures) && result.failures.length > 0) {
		parts.push(JSON.stringify(result.failures));
	}
	if (result.judge?.passed === false) {
		const issues = result.judge.issue_codes?.join(", ");
		parts.push(`juiz: ${issues || JSON.stringify(result.judge.scores ?? {})}`);
	}
	return parts.join(" · ") || "falha sem detalhes";
}

export function buildEvalJudgeContext(testCase, deterministic = { passed: true, failures: [] }) {
	const parts = [];
	if (testCase.judge_context) parts.push(testCase.judge_context);
	if (Array.isArray(testCase.checks) && testCase.checks.length > 0) {
		if (deterministic.passed) {
			parts.push(
				`Os checks determinísticos abaixo passaram e devem ser considerados evidência verificada: ${JSON.stringify(testCase.checks)}.`,
			);
		} else {
			parts.push(
				`Os checks determinísticos abaixo não passaram todos. Considere estas falhas registradas: ${JSON.stringify(deterministic.failures)}.`,
			);
		}
	}
	return parts.join("\n");
}

function parseArgs(argv) {
	const args = {
		suite: "evals/smoke.json",
		provider: undefined,
		repeat: 1,
		json: false,
		dryRun: false,
		out: undefined,
		judge: false,
		judgeProvider: process.env.DCX_JUDGE_PROVIDER || undefined,
		judgeRubric: "evals/rubrics/chat-quality.json",
	};
	for (let index = 0; index < argv.length; index += 1) {
		const arg = argv[index];
		if (arg === "--suite") args.suite = argv[++index];
		else if (arg === "--provider") args.provider = argv[++index];
		else if (arg === "--repeat") args.repeat = Number(argv[++index]);
		else if (arg === "--out") args.out = argv[++index];
		else if (arg === "--judge") args.judge = true;
		else if (arg === "--judge-provider") args.judgeProvider = argv[++index];
		else if (arg === "--judge-rubric") args.judgeRubric = argv[++index];
		else if (arg === "--json") args.json = true;
		else if (arg === "--dry-run") args.dryRun = true;
		else throw new Error(`argumento desconhecido: ${arg}`);
	}
	if (!Number.isInteger(args.repeat) || args.repeat < 1) {
		throw new Error("--repeat precisa ser um inteiro >= 1");
	}
	return args;
}

function safeRelative(value) {
	if (!value || path.isAbsolute(value) || value.split(/[\\/]/).includes("..")) {
		throw new Error(`caminho inválido no suite: ${value}`);
	}
	return value;
}

async function loadSuite(file) {
	const suite = JSON.parse(await readFile(file, "utf8"));
	if (suite.schema_version !== 1)
		throw new Error("suite precisa de schema_version 1");
	if (!suite.name) throw new Error("suite precisa de name");
	if (!Array.isArray(suite.cases) || suite.cases.length === 0) {
		throw new Error("suite precisa de cases");
	}
	for (const testCase of suite.cases) {
		if (!testCase.id) throw new Error("caso sem id");
		if (!testCase.prompt && !Array.isArray(testCase.turns)) {
			throw new Error(`caso ${testCase.id} precisa de prompt ou turns`);
		}
		if (!Array.isArray(testCase.checks) || testCase.checks.length === 0) {
			throw new Error(`caso ${testCase.id} precisa de checks`);
		}
		for (const name of Object.keys(testCase.files ?? {})) safeRelative(name);
		for (const spec of testCase.checks) {
			if (spec.type === "file_exists" || spec.type === "file_not_exists") {
				safeRelative(spec.path);
			}
		}
	}
	return suite;
}

async function fileState(workspace, checks) {
	const files = new Map();
	for (const spec of checks) {
		if (spec.type !== "file_exists" && spec.type !== "file_not_exists")
			continue;
		try {
			await access(path.join(workspace, spec.path));
			files.set(spec.path, true);
		} catch {
			files.set(spec.path, false);
		}
	}
	return files;
}

async function executeCase(testCase, env, binary, judge) {
	const workspace = await makeWorkspace(`dcx-eval-${testCase.id}-`);
	const started = Date.now();
	let before = null;
	let after = null;
	try {
		for (const [name, content] of Object.entries(testCase.files ?? {})) {
			const target = path.join(workspace, name);
			await mkdir(path.dirname(target), { recursive: true });
			await writeFile(target, content);
		}
		before = await snapshotProject(workspace);

		const turns = testCase.turns ?? [testCase.prompt];
		const input = `${turns.join("\n")}\n/exit\n`;
		const result = await runProcess(binary, ["chat"], {
			cwd: workspace,
			env,
			input,
			timeoutMs: testCase.timeout_ms ?? 180_000,
		});
		after = await snapshotProject(workspace);
		const filesChanged = changedPaths(before, after);
		if (result.code !== 0) {
			return {
				id: testCase.id,
				ok: false,
				error: `dcx chat terminou com ${result.code}`,
				stdout: result.stdout.slice(-8000),
				stderr: result.stderr.slice(-8000),
				workspace_unchanged: filesChanged.length === 0,
				files_changed: filesChanged,
				duration_ms: Date.now() - started,
				cost_usd: null,
			};
		}

		const { id: sessionId, session } = await latestSession(workspace);
		const assistant = [...session.messages]
			.reverse()
			.find((message) => message.role === "assistant");
		const text = assistant?.content ?? "";
		const files = await fileState(workspace, testCase.checks);
		const score = scoreCase(
			{ text, files, stderr: result.stderr },
			testCase.checks,
		);
		let judgment = null;
		if (judge) {
			try {
				judgment = await judgeResponse({
					testCase: {
						prompt: turns.join("\n"),
						context: buildEvalJudgeContext(testCase, {
							passed: score.ok,
							failures: score.failures,
						}),
						response: text,
					},
					rubric: judge.rubric,
					provider: judge.provider,
				});
			} catch (error) {
				judgment = { passed: false, scores: {}, cost_usd: null, error: error.message };
			}
		}

		return {
			id: testCase.id,
			ok: combineEvalScore(score.ok, judgment),
			deterministic_ok: score.ok,
			failures: score.failures,
			judge: judgment,
			session_id: sessionId,
			provider: session.provider,
			model: session.model,
			reply_preview: text.slice(0, 240),
			reply_chars: text.length,
			cost_usd: assistant?.cost_usd ?? null,
			duration_ms: assistant?.duration_ms ?? Date.now() - started,
			workspace_unchanged: filesChanged.length === 0,
			files_changed: filesChanged,
			stdout: result.stdout.slice(-8000),
			stderr: result.stderr.slice(-8000),
			usage: {
				input_tokens: assistant?.input_tokens ?? null,
				output_tokens: assistant?.output_tokens ?? null,
				cache_creation_input_tokens:
					assistant?.cache_creation_input_tokens ?? null,
				cache_read_input_tokens: assistant?.cache_read_input_tokens ?? null,
			},
		};
	} catch (error) {
		const filesChanged = before && after ? changedPaths(before, after) : [];
		return {
			id: testCase.id,
			ok: false,
			error: error.message,
			workspace_unchanged: filesChanged.length === 0,
			files_changed: filesChanged,
			duration_ms: Date.now() - started,
			cost_usd: null,
		};
	} finally {
		await rm(workspace, { recursive: true, force: true });
	}
}

function printSummary(report, reportPath, bundlePath) {
	const { summary } = report;
	const percent = (summary.pass_rate * 100).toFixed(1);
	console.log(`Suite: ${report.suite}`);
	console.log(
		`Provider: ${report.provider}${report.model ? ` (${report.model})` : ""}`,
	);
	console.log(
		`Casos: ${summary.passed}/${summary.total} passaram (${percent}%), pass@k ${(
			summary.pass_at_k * 100
		).toFixed(1)}%`,
	);
	console.log(
		`Custo: US$ ${summary.cost_usd.toFixed(4)}${summary.cost_complete ? "" : " (incompleto)"}`,
	);
	if (report.judge) {
		const judged = report.judge.summary;
		console.log(
			`Juiz (${report.judge.provider}): ${judged.passed}/${judged.total} aprovados; custo US$ ${judged.cost_usd.toFixed(4)}${judged.cost_complete ? "" : " (incompleto)"}`,
		);
		console.log(`Notas médias: ${JSON.stringify(judged.average_scores)}`);
	}
	console.log(
		`Latência: p50 ${summary.latency_ms.p50 ?? "-"} ms, p95 ${summary.latency_ms.p95 ?? "-"} ms`,
	);
	if (summary.tokens.complete) {
		console.log(
			`Tokens: ${summary.tokens.input} entrada, ${summary.tokens.output} saída`,
		);
	}
	const failed = report.cases.filter((result) => !result.ok);
	if (failed.length > 0) {
		console.log("Falhas:");
		for (const result of failed) {
			console.log(`- ${result.id}: ${describeFailure(result)}`);
		}
	}
	console.log(`Relatório: ${reportPath}`);
	console.log(`Bundle de observabilidade: ${bundlePath}`);
}

async function main() {
	const args = parseArgs(process.argv.slice(2));
	const suite = await loadSuite(args.suite);
	const requestedProvider =
		args.provider ?? suite.provider ?? process.env.DCX_E2E_PROVIDER ?? "minimax";
	let judge = null;
	if (args.judge) {
		const rubricPath = path.resolve(repoRoot, args.judgeRubric);
		const rubric = JSON.parse(await readFile(rubricPath, "utf8"));
		validateRubric(rubric);
		judge = {
			rubric,
			provider: args.judgeProvider ?? requestedProvider,
		};
	}

	if (args.dryRun) {
		console.log(
			JSON.stringify(
				{
						suite: `${suite.name}${judge ? "-judged" : ""}`,
						provider: requestedProvider,
						judge: judge
							? { provider: judge.provider, rubric: judge.rubric.name ?? args.judgeRubric }
							: null,
					cases: suite.cases.map((testCase) => ({
						id: testCase.id,
						checks: testCase.checks,
						judge_context: testCase.judge_context ?? null,
					})),
				},
				null,
				2,
			),
		);
		return;
	}

	if (process.env.DCX_EVAL !== "1") {
		throw new Error(
			"defina DCX_EVAL=1 para executar evals com provider real",
		);
	}
	const { provider, env } = realProviderEnv(args.provider ?? suite.provider);
	const binary = await dcxBinary();
	const startedAt = new Date().toISOString();
	const started = Date.now();
	const results = [];
	for (let attempt = 0; attempt < args.repeat; attempt += 1) {
		for (const testCase of suite.cases) {
			results.push(await executeCase(testCase, env, binary, judge));
		}
	}

	const durationMs = Date.now() - started;
	const report = {
		suite: `${suite.name}${judge ? "-judged" : ""}`,
		provider,
		model: results.find((result) => result.model)?.model ?? null,
		started_at: startedAt,
		duration_ms: durationMs,
		repeat: args.repeat,
		cases: results,
		summary: summarize(results),
		judge: judge
			? {
				provider: judge.provider,
				rubric: judge.rubric.name ?? args.judgeRubric,
				summary: summarizeJudgments(
					results.map((item) => item.judge ?? { passed: false, cost_usd: null }),
				),
			}
			: null,
	};
	const bundle = {
		schema_version: 1,
		name: `eval-${suite.name}${judge ? "-judged" : ""}-${provider}`,
		kind: "eval",
		provider,
		started_at: startedAt,
		duration_ms: durationMs,
		ok: report.summary.failed === 0,
		cases: results,
		metrics: {
			model_calls: results.length * (judge ? 2 : 1),
			cost_usd: report.summary.cost_usd + (report.judge?.summary.cost_usd ?? 0),
			cost_complete: report.summary.cost_complete && (!judge || report.judge.summary.cost_complete),
		},
		expected: {
			provider,
			cases: suite.cases.map((testCase) => testCase.id),
			repeat: args.repeat,
			max_duration_ms: 180_000 * args.repeat,
			max_cost_usd: 2.0,
			max_model_calls: results.length * (judge ? 2 : 1),
			allow_writes: false,
		},
	};
	const reportPath =
		args.out ?? path.join(repoRoot, "target", "eval-report.json");
	const bundlePath =
		process.env.DCX_EVAL_BUNDLE ??
		path.join(repoRoot, "target", "eval-bundle.json");
	await mkdir(path.dirname(reportPath), { recursive: true });
	await mkdir(path.dirname(bundlePath), { recursive: true });
	await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
	await writeFile(bundlePath, `${JSON.stringify(bundle, null, 2)}\n`);

	if (args.json) console.log(JSON.stringify(report, null, 2));
	else printSummary(report, reportPath, bundlePath);
	if (report.summary.failed > 0) process.exitCode = 1;
	if (report.judge && (report.judge.summary.failed > 0 || !report.judge.summary.cost_complete)) {
		process.exitCode = 1;
	}
}

if (
	process.argv[1] &&
	pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
	main().catch((error) => {
		console.error(`FALHA: ${error.message}`);
		process.exitCode = 1;
	});
}
