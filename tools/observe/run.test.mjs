import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cp, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import { repoRoot } from "../lib/real.mjs";

const runner = path.join(repoRoot, "tools", "observe", "run.mjs");
const observe = path.join(repoRoot, "tools", "observe");
const fixtures = path.join(observe, "fixtures");

function run(args) {
	return spawnSync(process.execPath, [runner, ...args], {
		cwd: repoRoot,
		encoding: "utf8",
		env: { ...process.env, DCX_BUNDLE: "" },
	});
}

test("fixture saudável passa em todos os sensores", async () => {
	const directory = await mkdtemp(path.join(tmpdir(), "observe-healthy-"));
	const out = path.join(directory, "report.json");
	try {
		const result = run([
			"--bundle",
			path.join(fixtures, "healthy.json"),
			"--out",
			out,
		]);
		assert.equal(result.status, 0, result.stderr || result.stdout);
		const report = JSON.parse(await readFile(out, "utf8"));
		assert.equal(report.summary.total, 6);
		assert.equal(report.summary.failed, 0);
		assert.equal(report.controls.faulty, true);
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
});

test("fixture defeituoso reprova todos os sensores", async () => {
	const directory = await mkdtemp(path.join(tmpdir(), "observe-faulty-"));
	const out = path.join(directory, "report.json");
	try {
		const result = run([
			"--bundle",
			path.join(fixtures, "faulty.json"),
			"--out",
			out,
		]);
		assert.equal(result.status, 1);
		const report = JSON.parse(await readFile(out, "utf8"));
		assert.equal(report.summary.failed, 6);
		assert.ok(
			report.sensors.every((sensor) => sensor.passed === false),
			"todos os sensores devem detectar o defeito",
		);
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
});

test("visible-only não executa sensores ocultos", async () => {
	const directory = await mkdtemp(path.join(tmpdir(), "observe-visible-"));
	const out = path.join(directory, "report.json");
	try {
		const result = run([
			"--bundle",
			path.join(fixtures, "healthy.json"),
			"--out",
			out,
			"--visible-only",
		]);
		assert.equal(result.status, 0, result.stderr || result.stdout);
		const report = JSON.parse(await readFile(out, "utf8"));
		assert.equal(report.summary.visible.total, 3);
		assert.equal(report.summary.hidden.total, 0);
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
});

test("trava recusa sensor alterado", async () => {
	const directory = await mkdtemp(path.join(tmpdir(), "observe-lock-"));
	const suite = path.join(directory, "suite.json");
	const contract = path.join(directory, "contract.json");
	const sensors = path.join(directory, "sensors");
	const out = path.join(directory, "report.json");
	try {
		await cp(path.join(observe, "suite.json"), suite);
		await cp(path.join(observe, "contract.json"), contract);
		await cp(path.join(observe, "sensors"), sensors, { recursive: true });
		const target = path.join(sensors, "visible_budget.py");
		const original = await readFile(target, "utf8");
		await writeFile(target, `${original}\n# adulterado\n`);
		const result = run([
			"--bundle",
			path.join(fixtures, "healthy.json"),
			"--suite",
			suite,
			"--contract",
			contract,
			"--lock",
			path.join(observe, "sensor_lock.json"),
			"--out",
			out,
		]);
		assert.equal(result.status, 1);
		assert.match(result.stderr, /não corresponde à trava/);
		assert.equal(original.length > 0, true);
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
});
