import { mkdir, readFile, readdir, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"..",
	"..",
);

const ARTIFACTS = [
	"events.jsonl",
	"run.json",
	"metrics.json",
	"verdict.json",
	"delivery_report.json",
	"sensor_results.json",
	"step_reports.json",
];

function usage() {
	return [
		"Uso: node tools/observe/collect.mjs [opções]",
		"",
		"  --run <dir>              diretório do run (padrão: o mais recente em .dcx/runs)",
		"  --out <arquivo>          bundle de saída (padrão: target/run-bundle.json)",
		"  --expect approved|rejected  resultado esperado (padrão: approved)",
		"  --provider <nome>        provider esperado",
		"  --allow-fallback         aceita múltiplos providers nas métricas",
		"  --max-cost-usd <valor>   teto de custo",
		"  --max-duration-ms <ms>   teto de duração (padrão: 900000)",
		"  --max-model-calls <n>    teto de chamadas de modelo",
		"  --stdout <arquivo>       stdout capturado do run",
		"  --stderr <arquivo>       stderr capturado do run",
		"  --strict                 sai com erro quando o bundle não está ok",
		"  --json                   imprime o bundle completo",
	].join("\n");
}

function parseArgs(argv) {
	const args = {
		run: null,
		out: path.join(repoRoot, "target", "run-bundle.json"),
		expect: "approved",
		provider: null,
		allowFallback: false,
		maxCostUsd: null,
		maxDurationMs: 900_000,
		maxModelCalls: null,
		stdoutPath: null,
		stderrPath: null,
		strict: false,
		json: false,
	};
	for (let index = 0; index < argv.length; index += 1) {
		const arg = argv[index];
		if (arg === "--run") args.run = argv[++index];
		else if (arg === "--out") args.out = argv[++index];
		else if (arg === "--expect") args.expect = argv[++index];
		else if (arg === "--provider") args.provider = argv[++index];
		else if (arg === "--allow-fallback") args.allowFallback = true;
		else if (arg === "--max-cost-usd") args.maxCostUsd = Number(argv[++index]);
		else if (arg === "--max-duration-ms")
			args.maxDurationMs = Number(argv[++index]);
		else if (arg === "--max-model-calls")
			args.maxModelCalls = Number(argv[++index]);
		else if (arg === "--stdout") args.stdoutPath = argv[++index];
		else if (arg === "--stderr") args.stderrPath = argv[++index];
		else if (arg === "--strict") args.strict = true;
		else if (arg === "--json") args.json = true;
		else if (arg === "--help" || arg === "-h") {
			console.log(usage());
			process.exit(0);
		} else {
			throw new Error(`argumento desconhecido: ${arg}`);
		}
	}
	if (args.expect !== "approved" && args.expect !== "rejected") {
		throw new Error("--expect deve ser approved ou rejected");
	}
	for (const [name, value] of [
		["--max-cost-usd", args.maxCostUsd],
		["--max-duration-ms", args.maxDurationMs],
		["--max-model-calls", args.maxModelCalls],
	]) {
		if (value !== null && (!Number.isFinite(value) || value < 0)) {
			throw new Error(`${name} deve ser um número não negativo`);
		}
	}
	return args;
}

async function readText(file) {
	try {
		return await readFile(file, "utf8");
	} catch {
		return null;
	}
}

async function readJson(file) {
	const text = await readText(file);
	if (text === null) return null;
	try {
		return JSON.parse(text);
	} catch {
		return null;
	}
}

async function latestRunDir(workspace) {
	const runsRoot = path.join(workspace, ".dcx", "runs");
	const entries = await readdir(runsRoot, { withFileTypes: true });
	const directories = entries.filter((entry) => entry.isDirectory());
	if (directories.length === 0) {
		throw new Error(`nenhum run em ${runsRoot}`);
	}
	const ranked = await Promise.all(
		directories.map(async (entry) => {
			const absolute = path.join(runsRoot, entry.name);
			const info = await stat(absolute);
			return { absolute, mtime: info.mtimeMs };
		}),
	);
	ranked.sort((left, right) => right.mtime - left.mtime);
	return ranked[0].absolute;
}

function parseEvents(text) {
	const events = [];
	const errors = [];
	if (!text) return { events, errors: ["events.jsonl ausente"] };
	for (const [index, line] of text.split("\n").entries()) {
		if (!line.trim()) continue;
		try {
			const event = JSON.parse(line);
			if (!Number.isInteger(event.sequence)) {
				errors.push(`evento na linha ${index + 1} sem sequence inteiro`);
			}
			events.push(event);
		} catch (error) {
			errors.push(`evento inválido na linha ${index + 1}: ${error.message}`);
		}
	}
	if (events.length === 0) errors.push("events.jsonl vazio");
	return { events, errors };
}

function providerList(metrics) {
	const providers = (metrics?.calls ?? [])
		.map((call) => call.provider)
		.filter((value) => typeof value === "string" && value.length > 0);
	return [...new Set(providers)].sort();
}

function safeRelative(file) {
	if (typeof file !== "string" || file.length === 0) return false;
	if (file.startsWith("/") || file.startsWith("\\")) return false;
	return !file.replaceAll("\\", "/").split("/").includes("..");
}

function changedFiles(stepReports, events) {
	const files = new Set();
	for (const report of stepReports ?? []) {
		for (const file of report.files_changed ?? []) files.add(file);
	}
	for (const event of events) {
		if (event.event_type !== "step.completed") continue;
		for (const file of event.payload?.files ?? []) files.add(file);
	}
	return [...files].sort();
}

function logsFrom(delivery, verdict, sensorResults, events, stdout, stderr) {
	const parts = [];
	if (stdout) parts.push(stdout);
	if (stderr) parts.push(stderr);
	if (delivery?.summary) parts.push(delivery.summary);
	for (const issue of verdict?.issues ?? []) {
		parts.push(`${issue.criterion_id}: ${issue.description}`);
	}
	for (const result of sensorResults ?? []) {
		parts.push(`${result.sensor_id}: ${result.evidence}`);
	}
	for (const event of events) {
		if (!/error|fail|panic|exception/i.test(event.event_type)) continue;
		parts.push(`${event.event_type} ${JSON.stringify(event.payload ?? {})}`);
	}
	return parts.filter(Boolean).join("\n");
}

function terminalState(states, expect) {
	if (!Array.isArray(states) || states.length === 0) return false;
	const last = states.at(-1);
	if (expect === "rejected") return last === "failed" || last === "cancelled";
	return last === "awaiting_merge" || last === "done";
}

export async function collectRun(options = {}) {
	const workspace = options.workspace ?? process.cwd();
	const runDir = options.runDir ?? (await latestRunDir(workspace));
	const expect = options.expect ?? "approved";
	const expectApproved = expect === "approved";

	const texts = {};
	for (const name of ARTIFACTS) {
		texts[name] = await readText(path.join(runDir, name));
	}
	const run = await readJson(path.join(runDir, "run.json"));
	const metrics = await readJson(path.join(runDir, "metrics.json"));
	const verdict = await readJson(path.join(runDir, "verdict.json"));
	const delivery = await readJson(path.join(runDir, "delivery_report.json"));
	const sensorResults = await readJson(
		path.join(runDir, "sensor_results.json"),
	);
	const stepReports = await readJson(path.join(runDir, "step_reports.json"));
	const { events, errors: eventErrors } = parseEvents(texts["events.jsonl"]);

	const stdout =
		options.stdoutPath === null || options.stdoutPath === undefined
			? null
			: await readText(options.stdoutPath);
	const stderr =
		options.stderrPath === null || options.stderrPath === undefined
			? null
			: await readText(options.stderrPath);

	const missing = ARTIFACTS.filter((name) => texts[name] === null);
	const providers = providerList(metrics);
	const expectedProvider = options.provider ?? providers[0] ?? "unknown";
	const mixed = providers.length > 1;
	const mixedWithoutDeclaration = mixed && options.allowFallback !== true;
	const provider = mixed ? "mixed" : expectedProvider;
	const fallback =
		mixed && options.allowFallback
			? {
					used: true,
					from: providers[0] ?? expectedProvider,
					reason: "declarado com --allow-fallback",
				}
			: null;

	const changed = changedFiles(stepReports, events);
	const unsafe = changed.filter((file) => !safeRelative(file));
	const checks = [
		{
			name: "artifacts",
			ok: missing.length === 0,
			detail:
				missing.length === 0
					? "todos os artefatos obrigatórios existem"
					: `faltando: ${missing.join(", ")}`,
		},
		{
			name: "events",
			ok: eventErrors.length === 0,
			detail:
				eventErrors.length === 0
					? `${events.length} evento(s) em ordem`
					: eventErrors.join("; "),
		},
		{
			name: "verdict",
			ok: verdict?.approved === expectApproved,
			detail: `aprovado=${verdict?.approved}; esperado=${expectApproved}`,
		},
		{
			name: "sensors",
			ok: Array.isArray(sensorResults),
			detail: Array.isArray(sensorResults)
				? `${sensorResults.filter((result) => result.passed).length}/${sensorResults.length} sensor(es) passaram`
				: "sensor_results.json ausente ou inválido",
		},
		{
			name: "states",
			ok: terminalState(run?.states, expect),
			detail: `estado final=${run?.states?.at(-1) ?? "desconhecido"}`,
		},
		{
			name: "workspace",
			ok: unsafe.length === 0,
			files_changed: changed,
			detail:
				unsafe.length === 0
					? `${changed.length} arquivo(s) alterado(s)`
					: `caminhos inseguros: ${unsafe.join(", ")}`,
		},
		{
			name: "provider",
			ok: !mixedWithoutDeclaration,
			detail: mixedWithoutDeclaration
				? `múltiplos providers sem --allow-fallback: ${providers.join(", ")}`
				: `provider=${provider}`,
		},
	];

	const ok = checks.every((check) => check.ok);
	const duration = metrics?.total_duration_ms;
	const startedAt = metrics?.started_at_unix_ms
		? new Date(metrics.started_at_unix_ms).toISOString()
		: null;
	const bundle = {
		schema_version: 1,
		name: `run-${path.basename(runDir)}`,
		kind: "run",
		run_id: path.basename(runDir),
		provider,
		started_at: startedAt,
		duration_ms: typeof duration === "number" ? duration : null,
		ok,
		checks,
		metrics: {
			model_calls: metrics?.call_count ?? null,
			cost_usd: metrics?.known_cost_usd ?? null,
			cost_complete: metrics?.cost_complete ?? false,
			input_tokens: (metrics?.calls ?? []).reduce(
				(sum, call) => sum + (call.input_tokens ?? 0),
				0,
			),
			output_tokens: (metrics?.calls ?? []).reduce(
				(sum, call) => sum + (call.output_tokens ?? 0),
				0,
			),
		},
		events,
		logs: logsFrom(delivery, verdict, sensorResults, events, stdout, stderr),
		expected: {
			provider: expectedProvider,
			checks: checks.map((check) => check.name),
			max_duration_ms: options.maxDurationMs ?? 900_000,
			max_cost_usd: options.maxCostUsd ?? null,
			max_model_calls: options.maxModelCalls ?? null,
			allow_writes: true,
		},
	};
	if (fallback) bundle.fallback = fallback;
	return { bundle, runDir };
}

async function main() {
	const args = parseArgs(process.argv.slice(2));
	const { bundle, runDir } = await collectRun({
		workspace: process.cwd(),
		runDir: args.run ? path.resolve(args.run) : undefined,
		expect: args.expect,
		provider: args.provider,
		allowFallback: args.allowFallback,
		maxCostUsd: args.maxCostUsd,
		maxDurationMs: args.maxDurationMs,
		maxModelCalls: args.maxModelCalls,
		stdoutPath: args.stdoutPath,
		stderrPath: args.stderrPath,
	});
	await mkdir(path.dirname(args.out), { recursive: true });
	await writeFile(args.out, `${JSON.stringify(bundle, null, 2)}\n`);
	if (args.json) console.log(JSON.stringify(bundle, null, 2));
	else {
		console.log(`Run: ${runDir}`);
		console.log(
			`Bundle: ${args.out}; ok=${bundle.ok}; provider=${bundle.provider}; ${bundle.checks.filter((check) => check.ok).length}/${bundle.checks.length} check(s)`,
		);
		for (const check of bundle.checks) {
			if (!check.ok) console.log(`- ${check.name}: ${check.detail}`);
		}
	}
	if (args.strict && !bundle.ok) process.exitCode = 1;
}

main().catch((error) => {
	console.error(`FALHA: ${error.message}`);
	process.exitCode = 1;
});
