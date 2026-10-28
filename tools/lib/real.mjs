import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { access, mkdtemp, readFile, readdir } from "node:fs/promises";
import net from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const repoRoot = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"..",
	"..",
);

export async function dcxBinary() {
	if (process.env.DCX_BIN) return process.env.DCX_BIN;
	const local = path.join(repoRoot, "target", "debug", "dcx");
	try {
		await access(local);
		return local;
	} catch {
		return "dcx";
	}
}

export function realProviderEnv(provider) {
	const normalized = (
		provider ??
		process.env.DCX_E2E_PROVIDER ??
		"minimax"
	).toLowerCase();
	if (normalized !== "claude" && normalized !== "minimax") {
		throw new Error(`provider inválido: ${normalized}; use claude ou minimax`);
	}
	const env = {
		...process.env,
		DCX_CHAT_PROVIDER: normalized,
		DCX_CHAT_WEB: "0",
	};
	if (
		normalized === "minimax" &&
		!env.MINIMAX_API_KEY &&
		!env.DCX_MINIMAX_API_KEY
	) {
		throw new Error(
			"defina MINIMAX_API_KEY (ou DCX_MINIMAX_API_KEY) para o provider minimax",
		);
	}
	if (normalized === "claude" && !env.ANTHROPIC_API_KEY) {
		throw new Error("defina ANTHROPIC_API_KEY para o provider claude");
	}
	return { provider: normalized, env };
}

export function runProcess(command, args, options = {}) {
	const { cwd, env = process.env, input, timeoutMs = 180_000 } = options;

	return new Promise((resolve, reject) => {
		const child = spawn(command, args, {
			cwd,
			env,
			detached: process.platform !== "win32",
			stdio: [input === undefined ? "ignore" : "pipe", "pipe", "pipe"],
		});
		let stdout = "";
		let stderr = "";
		let timedOut = false;
		const timer = setTimeout(() => {
			timedOut = true;
			signalProcessTree(child, "SIGTERM");
			setTimeout(() => signalProcessTree(child, "SIGKILL"), 5_000).unref();
		}, timeoutMs);

		child.stdin?.on("error", () => {});
		child.stdout.setEncoding("utf8");
		child.stderr.setEncoding("utf8");
		child.stdout.on("data", (chunk) => {
			stdout += chunk;
		});
		child.stderr.on("data", (chunk) => {
			stderr += chunk;
		});
		child.on("error", (error) => {
			clearTimeout(timer);
			reject(error);
		});
		child.on("close", (code) => {
			clearTimeout(timer);
			if (timedOut) {
				reject(
					new Error(
						`${command} excedeu ${timeoutMs} ms\nstdout:\n${stdout}\nstderr:\n${stderr}`,
					),
				);
				return;
			}
			resolve({ code, stdout, stderr });
		});
		if (input !== undefined) child.stdin.end(input);
	});
}

function signalProcessTree(child, signal) {
	if (!child.pid || child.exitCode !== null) return;
	if (process.platform !== "win32") {
		try {
			process.kill(-child.pid, signal);
			return;
		} catch (error) {
			if (error.code !== "ESRCH") throw error;
		}
	}
	child.kill(signal);
}

export async function makeWorkspace(prefix) {
	return mkdtemp(path.join(tmpdir(), prefix));
}

export async function latestSession(workspace) {
	const sessionsRoot = path.join(workspace, ".dcx", "sessions");
	const entries = await readdir(sessionsRoot, { withFileTypes: true });
	const sessions = entries.filter((entry) => entry.isDirectory());
	if (sessions.length !== 1) {
		throw new Error(
			`esperava 1 sessão em ${sessionsRoot}, encontrei ${sessions.length}`,
		);
	}
	const id = sessions[0].name;
	const session = JSON.parse(
		await readFile(path.join(sessionsRoot, id, "session.json"), "utf8"),
	);
	return { id, session };
}

export async function freePort() {
	return new Promise((resolve, reject) => {
		const server = net.createServer();
		server.unref();
		server.on("error", reject);
		server.listen(0, "127.0.0.1", () => {
			const address = server.address();
			const port = typeof address === "object" && address ? address.port : 0;
			server.close(() => resolve(port));
		});
	});
}

export async function waitForHttp(url, timeoutMs = 15_000) {
	const deadline = Date.now() + timeoutMs;
	let lastError;
	while (Date.now() < deadline) {
		try {
			const response = await fetch(url);
			if (response.ok) return;
			lastError = new Error(`HTTP ${response.status}`);
		} catch (error) {
			lastError = error;
		}
		await new Promise((resolve) => setTimeout(resolve, 200));
	}
	throw new Error(
		`servidor não respondeu em ${timeoutMs} ms: ${lastError?.message ?? "sem erro"}`,
	);
}

export async function stopProcess(child, timeoutMs = 3_000) {
	if (!child || child.exitCode !== null) return;
	child.kill("SIGTERM");
	await Promise.race([
		new Promise((resolve) => child.once("close", resolve)),
		new Promise((resolve) => setTimeout(resolve, timeoutMs)),
	]);
	if (child.exitCode === null) child.kill("SIGKILL");
}

export async function snapshotProject(root) {
	const files = new Map();

	async function walk(directory, prefix) {
		const entries = await readdir(directory, { withFileTypes: true });
		for (const entry of entries) {
			if (entry.name === ".dcx") continue;
			const absolute = path.join(directory, entry.name);
			const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
			if (entry.isDirectory()) {
				await walk(absolute, relative);
			} else if (entry.isFile()) {
				const contents = await readFile(absolute);
				files.set(
					relative,
					createHash("sha256").update(contents).digest("hex"),
				);
			}
		}
	}

	await walk(root, "");
	return Object.fromEntries([...files.entries()].sort());
}

export function changedPaths(before, after) {
	const paths = new Set([...Object.keys(before), ...Object.keys(after)]);
	return [...paths].filter((file) => before[file] !== after[file]).sort();
}
