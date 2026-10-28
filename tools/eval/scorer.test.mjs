import assert from "node:assert/strict";
import test from "node:test";

import { check, percentile, scoreCase, summarize } from "./scorer.mjs";

test("contains and regex are case-insensitive by default", () => {
	assert.equal(
		check({ text: "PONG" }, { type: "contains", value: "pong" }),
		true,
	);
	assert.equal(
		check({ text: "PONG" }, { type: "regex", value: "\\bpong\\b" }),
		true,
	);
	assert.equal(
		check({ text: "PONG" }, { type: "not_contains", value: "ping" }),
		true,
	);
});

test("equals trims both sides", () => {
	assert.equal(
		check({ text: "  OK \n" }, { type: "equals", value: "OK" }),
		true,
	);
	assert.equal(check({ text: "OK!" }, { type: "equals", value: "OK" }), false);
});

test("file checks read the precomputed map", () => {
	const actual = {
		text: "",
		files: new Map([
			["present.txt", true],
			["missing.txt", false],
		]),
	};
	assert.equal(
		check(actual, { type: "file_exists", path: "present.txt" }),
		true,
	);
	assert.equal(
		check(actual, { type: "file_exists", path: "missing.txt" }),
		false,
	);
	assert.equal(
		check(actual, { type: "file_not_exists", path: "missing.txt" }),
		true,
	);
});

test("unknown checks are reported as failures", () => {
	const score = scoreCase({ text: "ok", files: new Map() }, [{ type: "nope" }]);
	assert.equal(score.ok, false);
	assert.match(score.failures[0].error, /check desconhecido/);
});

test("scoreCase reports every failed check", () => {
	const score = scoreCase({ text: "nada", files: new Map([["x", false]]) }, [
		{ type: "contains", value: "PONG" },
		{ type: "file_exists", path: "x" },
	]);
	assert.equal(score.ok, false);
	assert.equal(score.failures.length, 2);
});

test("percentile handles empty and non-empty samples", () => {
	assert.equal(percentile([], 50), null);
	assert.equal(percentile([10, 20, 30], 50), 20);
	assert.equal(percentile([10, 20, 30], 95), 30);
});

test("summarize computes pass rates, pass@k, cost and latency", () => {
	const summary = summarize([
		{
			id: "a",
			ok: true,
			cost_usd: 0.1,
			duration_ms: 100,
			usage: { input_tokens: 10, output_tokens: 2 },
		},
		{
			id: "a",
			ok: false,
			cost_usd: 0.2,
			duration_ms: 300,
			usage: { input_tokens: 20, output_tokens: 3 },
		},
		{
			id: "b",
			ok: false,
			cost_usd: null,
			duration_ms: 200,
			usage: { input_tokens: null, output_tokens: null },
		},
	]);
	assert.equal(summary.total, 3);
	assert.equal(summary.passed, 1);
	assert.equal(summary.failed, 2);
	assert.equal(summary.pass_rate, 1 / 3);
	assert.equal(summary.pass_at_k, 1 / 2);
	assert.ok(Math.abs(summary.cost_usd - 0.3) < 1e-12);
	assert.equal(summary.cost_complete, false);
	assert.equal(summary.latency_ms.p50, 200);
	assert.equal(summary.latency_ms.p95, 300);
	assert.equal(summary.tokens.complete, false);
});

test("not_regex rejects text that matches", () => {
	const spec = { type: "not_regex", value: "^\\s*(I'll|Let me)\\b", flags: "" };
	assert.equal(check({ text: "I'll read the file." }, spec), false);
	assert.equal(check({ text: "DCX_EVAL_MARKER_42" }, spec), true);
});

test("an empty reply fails even when every check is negative", () => {
	const actual = { text: "  ", files: new Map([["x.txt", false]]) };
	const result = scoreCase(actual, [{ type: "file_not_exists", path: "x.txt" }]);
	assert.equal(result.ok, false);
	assert.equal(result.failures[0].type, "non_empty_reply");
});

test("an error reported by the CLI fails the case", () => {
	const actual = {
		text: "OK",
		stderr: "Erro: limite de chamadas de ferramenta atingido (24)\n",
	};
	const result = scoreCase(actual, [{ type: "contains", value: "OK" }]);
	assert.equal(result.ok, false);
	assert.equal(result.failures[0].type, "no_cli_error");
});
