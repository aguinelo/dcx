import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { runProcess } from "./real.mjs";

test("a timed out command also stops its provider subprocess", async () => {
	const directory = await mkdtemp(path.join(tmpdir(), "dcx-process-tree-"));
	const marker = path.join(directory, "heartbeat");
	const pidFile = path.join(directory, "child.pid");
	const descendant = `const fs=require("node:fs"); setInterval(()=>fs.appendFileSync(${JSON.stringify(marker)}, "."), 30);`;
	const parent = `const {spawn}=require("node:child_process"); const fs=require("node:fs"); const c=spawn(process.execPath,["-e",${JSON.stringify(descendant)}],{stdio:"ignore"}); fs.writeFileSync(${JSON.stringify(pidFile)},String(c.pid)); setInterval(()=>{},1000);`;
	try {
		await assert.rejects(
			runProcess(process.execPath, ["-e", parent], {
				timeoutMs: 300,
			}),
			/excedeu 300 ms/,
		);
		const atExit = await readFile(marker, "utf8").catch(() => "");
		await new Promise((resolve) => setTimeout(resolve, 180));
		const afterExit = await readFile(marker, "utf8").catch(() => "");
		assert.equal(afterExit.length, atExit.length, "o processo descendente continuou ativo");
	} finally {
		const pid = await readFile(pidFile, "utf8").catch(() => "");
		if (pid) {
			try {
				process.kill(Number(pid), "SIGKILL");
			} catch {}
		}
		await rm(directory, { recursive: true, force: true });
	}
});
