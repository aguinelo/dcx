#!/usr/bin/env node
import { spawn } from "node:child_process";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";

import {
	changedPaths,
	freePort,
	harnessBinary,
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

async function chatCheck(provider, env, binary) {
	const workspace = await makeWorkspace("harness-e2e-chat-");
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
	const workspace = await makeWorkspace("harness-e2e-daemon-");
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

async function main() {
	if (process.env.HARNESS_E2E !== "1") {
		console.log(
			"SKIP: defina HARNESS_E2E=1 para executar o e2e com provider real.",
		);
		return;
	}

	const { provider, env } = realProviderEnv();
	const binary = await harnessBinary();
	const startedAt = new Date().toISOString();
	const started = Date.now();
	const checks = [];
	checks.push(await chatCheck(provider, env, binary));
	checks.push(await daemonCheck(provider, env, binary));

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
			model_calls: checks.length,
			cost_usd: costs.reduce((sum, value) => sum + value, 0),
			cost_complete: costs.length === checks.length,
		},
		expected: {
			provider,
			checks: ["chat", "daemon"],
			max_duration_ms: 120_000,
			max_cost_usd: 1.0,
			max_model_calls: 4,
			allow_writes: false,
		},
	};
	const reportPath =
		process.env.HARNESS_E2E_REPORT ??
		path.join(repoRoot, "target", "e2e-real-report.json");
	const bundlePath =
		process.env.HARNESS_E2E_BUNDLE ??
		path.join(repoRoot, "target", "e2e-bundle.json");
	await mkdir(path.dirname(reportPath), { recursive: true });
	await mkdir(path.dirname(bundlePath), { recursive: true });
	await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
	await writeFile(bundlePath, `${JSON.stringify(bundle, null, 2)}\n`);
	console.log(JSON.stringify(report, null, 2));
	console.log(`Relatório: ${reportPath}`);
	console.log(`Bundle de observabilidade: ${bundlePath}`);
}

main().catch((error) => {
	console.error(`FALHA: ${error.message}`);
	process.exitCode = 1;
});
