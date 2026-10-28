import assert from "node:assert/strict";
import test from "node:test";

import { buildEvalJudgeContext, combineEvalScore, describeFailure } from "./run.mjs";

test("an enabled judge can only tighten deterministic eval results", () => {
	assert.equal(combineEvalScore(true, null), true);
	assert.equal(combineEvalScore(true, { passed: true }), true);
	assert.equal(combineEvalScore(true, { passed: false }), false);
	assert.equal(combineEvalScore(false, { passed: true }), false);
});

test("a missing judgment fails closed when judge mode is enabled", () => {
	assert.equal(combineEvalScore(true, { enabled: true, passed: undefined }), false);
});

test("judge context includes verified checks and case-specific product policy", () => {
	const context = buildEvalJudgeContext({
		judge_context: "O chat não altera arquivos.",
		checks: [{ type: "contains", value: "MARKER" }],
	});
	assert.match(context, /checks determinísticos.*passaram/);
	assert.match(context, /MARKER/);
	assert.match(context, /O chat não altera arquivos/);
});

test("judge context does not call failed deterministic checks successful", () => {
	const context = buildEvalJudgeContext(
		{ checks: [{ type: "contains", value: "PONG" }] },
		{ passed: false, failures: [{ type: "contains", expected: "PONG" }] },
	);
	assert.match(context, /não passaram todos/);
	assert.doesNotMatch(context, /checks determinísticos.*passaram e devem/);
});

test("failure summary exposes judge issue codes when deterministic checks passed", () => {
	assert.match(
		describeFailure({
			failures: [],
			judge: { passed: false, issue_codes: ["unsupported_product_details"] },
		}),
		/unsupported_product_details/,
	);
});
