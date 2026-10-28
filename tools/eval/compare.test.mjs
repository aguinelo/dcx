import assert from 'node:assert/strict';
import test from 'node:test';
import { compareReports } from './compare.mjs';

function report({ passRate = 1, passAtK = 1, cost = 1, p95 = 1000, ids = ['a', 'b'] } = {}) {
  return {
    suite: 'smoke',
    cases: ids.map((id) => ({ id })),
    summary: {
      total: ids.length,
      pass_rate: passRate,
      pass_at_k: passAtK,
      cost_usd: cost,
      cost_complete: true,
      latency_ms: { p95 },
    },
  };
}

test('a candidate within quality, cost and latency limits passes', () => {
  const result = compareReports(report(), report({ cost: 1.05, p95: 1100 }));
  assert.equal(result.ok, true);
  assert.ok(result.checks.every((item) => item.ok));
});

test('quality regression fails even when cost improves', () => {
  const result = compareReports(report(), report({ passRate: 0.9, cost: 0.5 }));
  assert.equal(result.ok, false);
  assert.ok(result.checks.some((item) => item.name === 'pass_rate' && !item.ok));
});

test('cost and p95 regressions beyond configured tolerances fail', () => {
  const result = compareReports(report(), report({ cost: 1.2, p95: 1500 }), {
    maxCostIncreasePct: 10,
    maxP95IncreasePct: 25,
  });
  assert.equal(result.ok, false);
  assert.ok(result.checks.some((item) => item.name === 'cost_per_case' && !item.ok));
  assert.ok(result.checks.some((item) => item.name === 'p95_latency' && !item.ok));
});

test('comparison refuses mismatched cases and incomplete cost', () => {
  const mismatched = compareReports(report(), report({ ids: ['a', 'c'] }));
  assert.equal(mismatched.ok, false);
  const incomplete = report();
  incomplete.summary.cost_complete = false;
  assert.equal(compareReports(incomplete, report()).ok, false);
});

test('comparison requires the same number of attempts for every case', () => {
  const baseline = report();
  baseline.cases.push({ id: 'a' });
  baseline.summary.total = 3;
  baseline.summary.cost_usd = 1.5;
  const candidate = report();
  const result = compareReports(baseline, candidate);
  assert.equal(result.ok, false);
  assert.ok(result.checks.some((item) => item.name === 'same_cases' && !item.ok));
});
