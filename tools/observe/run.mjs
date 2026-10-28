#!/usr/bin/env node
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";

import { repoRoot } from "../lib/real.mjs";
import { sensorFilename, sha256 } from "./lock.mjs";

function parseArgs(argv) {
	const args = {
		bundle: path.join(repoRoot, "target", "e2e-bundle.json"),
		suite: path.join(repoRoot, "tools", "observe", "suite.json"),
		contract: path.join(repoRoot, "tools", "observe", "contract.json"),
		lock: path.join(repoRoot, "tools", "observe", "sensor_lock.json"),
		out: path.join(repoRoot, "target", "observability-report.json"),
		visibleOnly: false,
		json: false,
		skipLock: false,
		skipControls: false,
	};
	for (let index = 0; index < argv.length; index += 1) {
		const arg = argv[index];
		if (arg === "--bundle") args.bundle = argv[++index];
		else if (arg === "--suite") args.suite = argv[++index];
		else if (arg === "--contract") args.contract = argv[++index];
		else if (arg === "--lock") args.lock = argv[++index];
		else if (arg === "--out") args.out = argv[++index];
		else if (arg === "--visible-only") args.visibleOnly = true;
		else if (arg === "--json") args.json = true;
		else if (arg === "--skip-lock") args.skipLock = true;
		else if (arg === "--skip-controls") args.skipControls = true;
		else throw new Error(`argumento desconhecido: ${arg}`);
	}
	return args;
}

async function readJson(file) {
	return JSON.parse(await readFile(file, "utf8"));
}

function validateCoverage(suite, contract) {
	const criteria = new Map(
		(contract.criteria ?? []).map((criterion) => [criterion.id, criterion]),
	);
	if (criteria.size === 0) throw new Error("contrato sem critérios");
	const visible = new Map([...criteria.keys()].map((id) => [id, 0]));
	let hidden = 0;
	for (const sensor of suite.sensors ?? []) {
		if (!criteria.has(sensor.criterion_id)) {
			throw new Error(
				`sensor ${sensor.id} aponta para critério desconhecido ${sensor.criterion_id}`,
			);
		}
		if (sensor.visibility === "visible") {
			visible.set(sensor.criterion_id, visible.get(sensor.criterion_id) + 1);
		} else if (sensor.visibility === "hidden") {
			hidden += 1;
		} else {
			throw new Error(`visibilidade inválida em ${sensor.id}`);
		}
	}
	for (const [id, count] of visible) {
		if (count === 0) throw new Error(`critério ${id} sem sensor visível`);
	}
	if (hidden === 0) throw new Error("suite sem sensor oculto");
}

async function verifyLock(suitePath, lockPath, contractPath) {
	const suiteBytes = await readFile(suitePath);
	const contractBytes = await readFile(contractPath);
	const suite = JSON.parse(suiteBytes.toString("utf8"));
	const contract = JSON.parse(contractBytes.toString("utf8"));
	const lock = await readJson(lockPath);
	if (lock.schema_version !== 1)
		throw new Error("sensor_lock precisa de schema_version 1");
	if (lock.suite_sha256 !== sha256(suiteBytes))
		throw new Error(
			"suite de observabilidade mudou; rode tools/observe/lock.mjs",
		);
	if (lock.contract_sha256 !== sha256(contractBytes))
		throw new Error(
			"contrato de observabilidade mudou; rode tools/observe/lock.mjs",
		);
	validateCoverage(suite, contract);
	const entries = new Map(
		(lock.sensors ?? []).map((entry) => [entry.sensor_id, entry]),
	);
	for (const sensor of suite.sensors ?? []) {
		const entry = entries.get(sensor.id);
		if (!entry) throw new Error(`sensor sem trava: ${sensor.id}`);
		const filename = sensorFilename(sensor.id);
		if (entry.filename !== filename)
			throw new Error(`arquivo travado divergente para ${sensor.id}`);
		const script = await readFile(
			path.join(path.dirname(suitePath), "sensors", filename),
		);
		if (entry.sha256 !== sha256(script))
			throw new Error(`sensor ${sensor.id} não corresponde à trava`);
	}
	return suite;
}

function runSensor(sensor, script, bundlePath, timeoutMs = 30_000) {
	const python = process.env.PYTHON ?? "python3";
	return new Promise((resolve) => {
		const started = Date.now();
		const child = spawn(python, [script], {
			cwd: repoRoot,
			env: {
				...process.env,
				HARNESS_BUNDLE: bundlePath,
				HARNESS_SENSOR_ID: sensor.id,
				HARNESS_VISIBILITY: sensor.visibility,
			},
			stdio: ["ignore", "pipe", "pipe"],
		});
		let stdout = "";
		let stderr = "";
		let timedOut = false;
		const timer = setTimeout(() => {
			timedOut = true;
			child.kill("SIGTERM");
			setTimeout(() => child.kill("SIGKILL"), 2_000).unref();
		}, timeoutMs);
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
			resolve({
				sensor_id: sensor.id,
				criterion_id: sensor.criterion_id,
				visibility: sensor.visibility,
				passed: false,
				evidence: error.message,
				duration_ms: Date.now() - started,
			});
		});
		child.on("close", (code) => {
			clearTimeout(timer);
			const evidence = (stdout || stderr).trim().slice(0, 2000);
			resolve({
				sensor_id: sensor.id,
				criterion_id: sensor.criterion_id,
				visibility: sensor.visibility,
				passed: !timedOut && code === 0,
				evidence: timedOut ? `timeout após ${timeoutMs} ms` : evidence,
				duration_ms: Date.now() - started,
			});
		});
	});
}

async function runSuite(suite, suitePath, bundlePath, { visibleOnly }) {
	const results = [];
	for (const sensor of suite.sensors ?? []) {
		if (visibleOnly && sensor.visibility === "hidden") continue;
		const script = path.join(
			path.dirname(suitePath),
			"sensors",
			sensorFilename(sensor.id),
		);
		results.push(await runSensor(sensor, script, bundlePath));
	}
	return results;
}

function failedIds(results) {
	return results
		.filter((result) => !result.passed)
		.map((result) => result.sensor_id);
}

async function negativeControl(suite, suitePath, { visibleOnly }) {
	const healthy = path.join(
		repoRoot,
		"tools",
		"observe",
		"fixtures",
		"healthy.json",
	);
	const faulty = path.join(
		repoRoot,
		"tools",
		"observe",
		"fixtures",
		"faulty.json",
	);
	const healthyResults = await runSuite(suite, suitePath, healthy, {
		visibleOnly,
	});
	if (healthyResults.some((result) => !result.passed)) {
		throw new Error(
			`controle saudável falhou: ${failedIds(healthyResults).join(", ")}`,
		);
	}
	const faultyResults = await runSuite(suite, suitePath, faulty, {
		visibleOnly,
	});
	const failed = failedIds(faultyResults);
	if (failed.length === 0)
		throw new Error("controle defeituoso passou em todos os sensores");
	return {
		healthy: true,
		faulty: true,
		faulty_failed: failed,
	};
}

function summarize(results) {
	const visible = results.filter((result) => result.visibility === "visible");
	const hidden = results.filter((result) => result.visibility === "hidden");
	return {
		total: results.length,
		passed: results.filter((result) => result.passed).length,
		failed: results.filter((result) => !result.passed).length,
		visible: {
			total: visible.length,
			passed: visible.filter((result) => result.passed).length,
		},
		hidden: {
			total: hidden.length,
			passed: hidden.filter((result) => result.passed).length,
		},
	};
}

function printSummary(report, reportPath, controls) {
	const { summary } = report;
	console.log(`Bundle: ${report.bundle_name} (${report.bundle_kind})`);
	console.log(
		`Provider: ${report.provider ?? "não declarado"}; sensores: ${summary.passed}/${summary.total} passaram`,
	);
	console.log(
		`Visíveis: ${summary.visible.passed}/${summary.visible.total}; ocultos: ${summary.hidden.passed}/${summary.hidden.total}`,
	);
	if (controls) {
		console.log(
			`Controle negativo: saudável ok; defeituoso reprovado por ${controls.faulty_failed.join(", ")}`,
		);
	}
	const failed = report.sensors.filter((sensor) => !sensor.passed);
	for (const sensor of failed) {
		console.log(`- ${sensor.sensor_id}: ${sensor.evidence}`);
	}
	console.log(`Relatório: ${reportPath}`);
}

async function main() {
	const args = parseArgs(process.argv.slice(2));
	const suite = args.skipLock
		? await readJson(args.suite)
		: await verifyLock(args.suite, args.lock, args.contract);
	const bundle = await readJson(args.bundle);
	if (bundle.schema_version !== 1)
		throw new Error("bundle precisa de schema_version 1");
	if (!bundle.name) throw new Error("bundle precisa de name");

	const controls = args.skipControls
		? null
		: await negativeControl(suite, args.suite, {
				visibleOnly: args.visibleOnly,
			});
	const startedAt = new Date().toISOString();
	const started = Date.now();
	const results = await runSuite(suite, args.suite, args.bundle, {
		visibleOnly: args.visibleOnly,
	});
	const report = {
		schema_version: 1,
		suite: suite.name,
		contract: path.relative(repoRoot, args.contract),
		bundle: args.bundle,
		bundle_name: bundle.name,
		bundle_kind: bundle.kind ?? "unknown",
		provider: bundle.provider ?? null,
		started_at: startedAt,
		duration_ms: Date.now() - started,
		controls,
		sensors: results,
		summary: summarize(results),
	};
	await mkdir(path.dirname(args.out), { recursive: true });
	await writeFile(args.out, `${JSON.stringify(report, null, 2)}\n`);
	if (args.json) console.log(JSON.stringify(report, null, 2));
	else printSummary(report, args.out, controls);
	if (report.summary.failed > 0) process.exitCode = 1;
}

main().catch((error) => {
	console.error(`FALHA: ${error.message}`);
	process.exitCode = 1;
});
