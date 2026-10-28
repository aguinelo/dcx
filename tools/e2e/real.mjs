#!/usr/bin/env node
import { spawn } from "node:child_process";
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

import {
	changedPaths,
	freePort,
	dcxBinary,
	latestSession,
	makeWorkspace,
	realProviderEnv,
	repoRoot,
	runProcess,
	snapshotProject,
	stopProcess,
	waitForHttp,
} from "../lib/real.mjs";

function assert(condition, message) {
	if (!condition) throw new Error(message);
}

export function validateRunEvidence({
	exitCode,
	workspaceFile,
	expectedContent = "DCX_RUN_E2E_OK\n",
	run,
	verdict,
	metrics,
	files,
	expectedFile,
}) {
	const failures = [];
	if (exitCode !== 0) failures.push(`dcx run terminou com código ${exitCode}`);
	if (workspaceFile !== expectedContent) {
		failures.push("arquivo aplicado ao workspace tem conteúdo inesperado");
	}
	if (!files?.includes(expectedFile)) {
		failures.push(`relatório não registra a alteração ${expectedFile}`);
	}
	if (verdict?.approved !== true) failures.push("veredito não aprovado");
	if (!["awaiting_merge", "done"].includes(run?.states?.at(-1))) {
		failures.push("run não terminou em estado aprovado");
	}
	if (!Number.isInteger(metrics?.call_count) || metrics.call_count < 1) {
		failures.push("métricas não registram chamadas do modelo");
	}
	if (metrics?.cost_complete !== true || typeof metrics?.known_cost_usd !== "number") {
		failures.push("custo do provider incompleto");
	}
	return { ok: failures.length === 0, failures };
}

async function chatCheck(provider, env, binary) {
	const workspace = await makeWorkspace("dcx-e2e-chat-");
	try {
		await writeFile(path.join(workspace, "README.md"), "original\n");
		const before = await snapshotProject(workspace);
		const result = await runProcess(binary, ["chat"], {
			cwd: workspace,
			env,
			input: "Responda apenas com a palavra OK.\n/exit\n",
		});
		assert(
			result.code === 0,
			`chat terminou com ${result.code}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`,
		);

		const { id, session } = await latestSession(workspace);
		assert(
			session.provider === provider,
			`provider da sessão: ${session.provider}`,
		);
		const assistant = [...session.messages]
			.reverse()
			.find((message) => message.role === "assistant");
		assert(
			assistant?.content?.trim(),
			"a sessão não contém resposta do assistente",
		);
		const after = await snapshotProject(workspace);
		const filesChanged = changedPaths(before, after);
		assert(
			(await readFile(path.join(workspace, "README.md"), "utf8")) ===
				"original\n",
			"o chat alterou um arquivo do projeto",
		);
		assert(
			filesChanged.length === 0,
			`o chat alterou arquivos do projeto: ${filesChanged.join(", ")}`,
		);

		return {
			name: "chat",
			ok: true,
			session_id: id,
			provider,
			model: session.model,
			reply_chars: assistant.content.length,
			cost_usd: assistant.cost_usd ?? null,
			duration_ms: assistant.duration_ms ?? null,
			workspace_unchanged: true,
			files_changed: filesChanged,
			stdout: result.stdout.slice(-8000),
			stderr: result.stderr.slice(-8000),
		};
	} finally {
		await rm(workspace, { recursive: true, force: true });
	}
}

async function daemonCheck(provider, env, binary) {
	const workspace = await makeWorkspace("dcx-e2e-daemon-");
	const before = await snapshotProject(workspace);
	const port = await freePort();
	const server = spawn(binary, ["serve", "--bind", `127.0.0.1:${port}`], {
		cwd: workspace,
		env,
		stdio: ["ignore", "pipe", "pipe"],
	});
	let serverStdout = "";
	let serverStderr = "";
	server.on("error", (error) => {
		serverStderr += `\n${error.message}`;
	});
	server.stdout?.setEncoding("utf8");
	server.stderr?.setEncoding("utf8");
	server.stdout?.on("data", (chunk) => {
		serverStdout += chunk;
	});
	server.stderr?.on("data", (chunk) => {
		serverStderr += chunk;
	});

	try {
		await waitForHttp(`http://127.0.0.1:${port}/health`);
		const response = await fetch(`http://127.0.0.1:${port}/chat`, {
			method: "POST",
			headers: { "content-type": "application/json" },
			body: JSON.stringify({
				workspace,
				message: "Responda apenas com a palavra OK.",
			}),
		});
		const body = await response.text();
		assert(response.ok, `POST /chat respondeu HTTP ${response.status}`);
		assert(!body.includes("event: error"), `SSE retornou erro:\n${body}`);
		assert(body.includes("event: done"), `SSE não terminou com done:\n${body}`);

		const sessionEvent = body.match(/event: session\ndata: (\{[^\n]*\})\n\n/);
		assert(sessionEvent, "SSE não enviou o evento session");
		const sessionId = JSON.parse(sessionEvent[1]).session_id;
		assert(sessionId, "evento session sem session_id");

		const listed = await fetch(
			`http://127.0.0.1:${port}/sessions?workspace=${encodeURIComponent(workspace)}`,
		);
		assert(listed.ok, `GET /sessions respondeu HTTP ${listed.status}`);
		const sessions = await listed.json();
		assert(
			sessions.sessions?.some((session) => session.session_id === sessionId),
			"sessão criada pelo daemon não aparece em /sessions",
		);
		const { session } = await latestSession(workspace);
		const assistant = [...session.messages]
			.reverse()
			.find((message) => message.role === "assistant");
		const after = await snapshotProject(workspace);
		const filesChanged = changedPaths(before, after);
		assert(
			filesChanged.length === 0,
			`o daemon alterou arquivos do projeto: ${filesChanged.join(", ")}`,
		);

		return {
			name: "daemon",
			ok: true,
			port,
			session_id: sessionId,
			provider,
			model: session.model,
			reply_chars: assistant?.content?.length ?? 0,
			cost_usd: assistant?.cost_usd ?? null,
			duration_ms: assistant?.duration_ms ?? null,
			sse_bytes: body.length,
			workspace_unchanged: true,
			files_changed: filesChanged,
			stdout: serverStdout.slice(-8000),
			stderr: serverStderr.slice(-8000),
		};
	} catch (error) {
		throw new Error(
			`${error.message}\nservidor stdout:\n${serverStdout}\nservidor stderr:\n${serverStderr}`,
		);
	} finally {
		await stopProcess(server);
		await rm(workspace, { recursive: true, force: true });
	}
}

export async function runCheck(provider, env, binary) {
	const workspace = await makeWorkspace("dcx-e2e-run-");
	const expectedFile = "resultado.txt";
	const before = await snapshotProject(workspace);
	const started = Date.now();
	try {
		let result;
		try {
			result = await runProcess(binary, [
				"run",
				"--request",
				"Crie somente o arquivo resultado.txt com o texto exato DCX_RUN_E2E_OK. Não altere outros arquivos.",
			], {
				cwd: workspace,
				env: {
					...env,
					DCX_PROVIDER: provider,
					DCX_RUN_MAX_COST_USD: "1",
					DCX_RUN_MAX_DURATION_MINUTES: "20",
				},
				input: "s\n",
				timeoutMs: 20 * 60 * 1000,
			});
		} catch (error) {
			result = { code: 124, stdout: "", stderr: error.message };
		}
		const durationMs = Date.now() - started;
		const runsRoot = path.join(workspace, ".dcx", "runs");
		const runEntries = await readdir(runsRoot, { withFileTypes: true }).catch(
			(error) => {
				if (error.code === "ENOENT") return [];
				throw error;
			},
		);
		const runs = runEntries.filter((entry) => entry.isDirectory());
		if (runs.length !== 1) {
			return {
				name: "run",
				ok: false,
				provider,
				duration_ms: durationMs,
				cost_usd: null,
				cost_complete: false,
				failures: [`esperava um run, encontrei ${runs.length}`],
				stdout: result.stdout.slice(-8000),
				stderr: result.stderr.slice(-8000),
			};
		}
		const runDir = path.join(runsRoot, runs[0].name);
		const readJson = async (name) => {
			try {
				return JSON.parse(await readFile(path.join(runDir, name), "utf8"));
			} catch {
				return null;
			}
		};
		const [run, verdict, metrics, stepReports] = await Promise.all([
			readJson("run.json"),
			readJson("verdict.json"),
			readJson("metrics.json"),
			readJson("step_reports.json"),
		]);
		const workspaceFile = await readFile(
			path.join(workspace, expectedFile),
			"utf8",
		).catch(() => null);
		const after = await snapshotProject(workspace);
		const filesChanged = changedPaths(before, after);
		const recordedFiles = (stepReports ?? []).flatMap(
			(step) => step.files_changed ?? [],
		);
		const evidence = validateRunEvidence({
			exitCode: result.code,
			workspaceFile,
			expectedContent: "DCX_RUN_E2E_OK",
			run,
			verdict,
			metrics,
			files: recordedFiles,
			expectedFile,
		});
		if (filesChanged.length !== 1 || filesChanged[0] !== expectedFile) {
			evidence.ok = false;
			evidence.failures.push(
				`arquivos alterados: ${filesChanged.join(", ") || "nenhum"}`,
			);
		}
		const cost = metrics?.known_cost_usd ?? null;
		if (typeof cost === "number" && cost > 1) {
			evidence.ok = false;
			evidence.failures.push(`custo excedeu US$ 1,00: ${cost}`);
		}
		const check = {
			name: "run",
			ok: evidence.ok,
			provider,
			model: metrics?.calls?.at(-1)?.model ?? null,
			run_id: runs[0].name,
			final_state: run?.states?.at(-1) ?? null,
			files_changed: filesChanged,
			model_calls: metrics?.call_count ?? null,
			cost_usd: cost,
			cost_complete: metrics?.cost_complete === true,
			duration_ms: durationMs,
			failures: evidence.failures,
			stdout: result.stdout.slice(-8000),
			stderr: result.stderr.slice(-8000),
		};
		const stdoutPath = path.join(repoRoot, "target", "e2e-run.stdout");
		const stderrPath = path.join(repoRoot, "target", "e2e-run.stderr");
		const bundlePath = path.join(repoRoot, "target", "run-bundle.json");
		await mkdir(path.dirname(stdoutPath), { recursive: true });
		await Promise.all([
			writeFile(stdoutPath, result.stdout),
			writeFile(stderrPath, result.stderr),
		]);
		const collect = await runProcess(
			process.execPath,
			[
				path.join(repoRoot, "tools", "observe", "collect.mjs"),
				"--run",
				runDir,
				"--provider",
				provider,
				"--max-cost-usd",
				"1",
				"--max-duration-ms",
				String(20 * 60 * 1000),
				"--max-model-calls",
				String(metrics?.call_count ?? 32),
				"--stdout",
				stdoutPath,
				"--stderr",
				stderrPath,
				"--out",
				bundlePath,
			],
			{ cwd: workspace },
		);
		if (collect.code !== 0) {
			check.ok = false;
			check.failures.push(`coletor falhou: ${collect.stdout} ${collect.stderr}`);
		}
		const observe = await runProcess(
			process.execPath,
			[
				path.join(repoRoot, "tools", "observe", "run.mjs"),
				"--bundle",
				bundlePath,
				"--out",
				path.join(repoRoot, "target", "observability-run.json"),
			],
			{ cwd: workspace },
		);
		check.observability_ok = observe.code === 0;
		check.observability_output = observe.stdout.trim();
		if (observe.code !== 0) {
			check.ok = false;
			check.failures.push(`sensores reprovaram: ${observe.stdout} ${observe.stderr}`);
		}
		return check;
	} finally {
		await rm(workspace, { recursive: true, force: true });
	}
}

async function main() {
	if (process.env.DCX_E2E !== "1") {
		console.log(
			"SKIP: defina DCX_E2E=1 para executar o e2e com provider real.",
		);
		return;
	}

	const { provider, env } = realProviderEnv();
	const binary = await dcxBinary();
	const startedAt = new Date().toISOString();
	const started = Date.now();
	const checks = [];
	checks.push(await chatCheck(provider, env, binary));
	checks.push(await daemonCheck(provider, env, binary));
	checks.push(await runCheck(provider, env, binary));

	const durationMs = Date.now() - started;
	const costs = checks
		.map((check) => check.cost_usd)
		.filter((value) => typeof value === "number");
	const report = {
		ok: checks.every((check) => check.ok),
		provider,
		started_at: startedAt,
		duration_ms: durationMs,
		checks,
	};
	const bundle = {
		schema_version: 1,
		name: `e2e-${provider}`,
		kind: "e2e",
		provider,
		started_at: startedAt,
		duration_ms: durationMs,
		ok: report.ok,
		checks,
		metrics: {
			model_calls: checks.reduce(
				(sum, check) => sum + (check.model_calls ?? 1),
				0,
			),
			cost_usd: costs.reduce((sum, value) => sum + value, 0),
			cost_complete:
				costs.length === checks.length &&
				checks.every((check) => check.cost_complete !== false),
		},
		expected: {
			provider,
			checks: ["chat", "daemon", "run"],
			max_duration_ms: 25 * 60 * 1000,
			max_cost_usd: 2.0,
			max_model_calls: 32,
			allow_writes: true,
		},
	};
	const reportPath =
		process.env.DCX_E2E_REPORT ??
		path.join(repoRoot, "target", "e2e-real-report.json");
	const bundlePath =
		process.env.DCX_E2E_BUNDLE ??
		path.join(repoRoot, "target", "e2e-bundle.json");
	await mkdir(path.dirname(reportPath), { recursive: true });
	await mkdir(path.dirname(bundlePath), { recursive: true });
	await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
	await writeFile(bundlePath, `${JSON.stringify(bundle, null, 2)}\n`);
	console.log(JSON.stringify(report, null, 2));
	console.log(`Relatório: ${reportPath}`);
	console.log(`Bundle de observabilidade: ${bundlePath}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
	main().catch((error) => {
		console.error(`FALHA: ${error.message}`);
		process.exitCode = 1;
	});
}
