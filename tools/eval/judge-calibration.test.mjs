import assert from "node:assert/strict";
import test from "node:test";

import { scoreCalibration } from "./judge-calibration.mjs";

test("calibration gates accuracy by expected labels and requires reported cost", () => {
	const summary = scoreCalibration([
		{ passed: true, expected_pass: true, cost_usd: 0.01, duration_ms: 10 },
		{ passed: false, expected_pass: false, cost_usd: 0.02, duration_ms: 20 },
	]);
	assert.equal(summary.passed, 2);
	assert.equal(summary.failed, 0);
	assert.equal(summary.pass_rate, 1);
	assert.equal(summary.cost_usd, 0.03);
	assert.equal(summary.cost_complete, true);
});

test("calibration fails when the judge disagrees or cost is unknown", () => {
	const summary = scoreCalibration([
		{ passed: true, expected_pass: false, cost_usd: null, duration_ms: 10 },
	]);
	assert.equal(summary.failed, 1);
	assert.equal(summary.cost_complete, false);
});
