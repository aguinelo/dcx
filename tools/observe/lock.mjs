#!/usr/bin/env node
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { repoRoot } from "../lib/real.mjs";

export function sensorFilename(sensorId) {
	return `${sensorId.replace(/-/g, "_")}.py`;
}

export function sha256(buffer) {
	return createHash("sha256").update(buffer).digest("hex");
}

export async function lockSuite({
	suitePath = path.join(repoRoot, "tools", "observe", "suite.json"),
	contractPath = path.join(repoRoot, "tools", "observe", "contract.json"),
	lockPath = path.join(repoRoot, "tools", "observe", "sensor_lock.json"),
} = {}) {
	const suiteBytes = await readFile(suitePath);
	const contractBytes = await readFile(contractPath);
	const suite = JSON.parse(suiteBytes.toString("utf8"));
	const sensors = suite.sensors ?? [];
	const seen = new Set();
	const entries = [];
	for (const sensor of sensors) {
		if (!sensor.id) throw new Error("sensor sem id na suite");
		if (seen.has(sensor.id))
			throw new Error(`sensor duplicado na suite: ${sensor.id}`);
		seen.add(sensor.id);
		const filename = sensorFilename(sensor.id);
		const script = await readFile(
			path.join(path.dirname(suitePath), "sensors", filename),
		);
		entries.push({
			sensor_id: sensor.id,
			filename,
			sha256: sha256(script),
		});
	}
	const lock = {
		schema_version: 1,
		suite_sha256: sha256(suiteBytes),
		contract_sha256: sha256(contractBytes),
		sensors: entries,
	};
	await writeFile(lockPath, `${JSON.stringify(lock, null, "\t")}\n`);
	return lock;
}

if (
	process.argv[1] &&
	import.meta.url === pathToFileURL(process.argv[1]).href
) {
	lockSuite()
		.then((lock) => {
			console.log(
				`Trava atualizada: ${lock.sensors.length} sensor(es), suite ${lock.suite_sha256.slice(0, 12)}…`,
			);
		})
		.catch((error) => {
			console.error(`FALHA: ${error.message}`);
			process.exitCode = 1;
		});
}
