#!/usr/bin/env node
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export function compareReports(
  baseline,
  candidate,
  { maxCostIncreasePct = 10, maxP95IncreasePct = 25 } = {},
) {
  const baselineIds = (baseline.cases ?? []).map((item) => item.id).sort();
  const candidateIds = (candidate.cases ?? []).map((item) => item.id).sort();
  const sameCases = baseline.suite === candidate.suite
    && JSON.stringify(baselineIds) === JSON.stringify(candidateIds);
  const base = baseline.summary ?? {};
  const next = candidate.summary ?? {};
  const validCost = base.cost_complete === true
    && next.cost_complete === true
    && base.total > 0
    && next.total > 0;
  const validP95 = Number.isFinite(base.latency_ms?.p95)
    && base.latency_ms.p95 > 0
    && Number.isFinite(next.latency_ms?.p95);
  const baselineCost = validCost ? base.cost_usd / base.total : null;
  const candidateCost = validCost ? next.cost_usd / next.total : null;
  const baselineP95 = validP95 ? base.latency_ms.p95 : null;
  const candidateP95 = validP95 ? next.latency_ms.p95 : null;
  const checks = [
    {
      name: 'same_cases',
      baseline: baselineIds,
      candidate: candidateIds,
      ok: sameCases,
    },
    {
      name: 'pass_rate',
      baseline: base.pass_rate ?? null,
      candidate: next.pass_rate ?? null,
      ok: sameCases && Number.isFinite(base.pass_rate) && Number.isFinite(next.pass_rate)
        && next.pass_rate >= base.pass_rate,
    },
    {
      name: 'pass_at_k',
      baseline: base.pass_at_k ?? null,
      candidate: next.pass_at_k ?? null,
      ok: sameCases && Number.isFinite(base.pass_at_k) && Number.isFinite(next.pass_at_k)
        && next.pass_at_k >= base.pass_at_k,
    },
    {
      name: 'cost_per_case',
      baseline: baselineCost,
      candidate: candidateCost,
      max_increase_pct: maxCostIncreasePct,
      ok: validCost && candidateCost <= baselineCost * (1 + maxCostIncreasePct / 100),
    },
    {
      name: 'p95_latency',
      baseline: baselineP95,
      candidate: candidateP95,
      max_increase_pct: maxP95IncreasePct,
      ok: validP95 && candidateP95 <= baselineP95 * (1 + maxP95IncreasePct / 100),
    },
  ];
  return {
    suite: candidate.suite ?? null,
    baseline_provider: baseline.provider ?? null,
    candidate_provider: candidate.provider ?? null,
    ok: checks.every((item) => item.ok),
    checks,
  };
}

function parseArgs(argv) {
  const args = { baseline: undefined, candidate: undefined, maxCostIncreasePct: 10, maxP95IncreasePct: 25 };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--baseline') args.baseline = argv[++index];
    else if (arg === '--candidate') args.candidate = argv[++index];
    else if (arg === '--max-cost-increase') args.maxCostIncreasePct = Number(argv[++index]);
    else if (arg === '--max-p95-increase') args.maxP95IncreasePct = Number(argv[++index]);
    else throw new Error(`argumento desconhecido: ${arg}`);
  }
  if (!args.baseline || !args.candidate) {
    throw new Error('uso: node tools/eval/compare.mjs --baseline <arquivo> --candidate <arquivo>');
  }
  for (const key of ['maxCostIncreasePct', 'maxP95IncreasePct']) {
    if (!Number.isFinite(args[key]) || args[key] < 0) throw new Error(`${key} precisa ser um número >= 0`);
  }
  return args;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const [baseline, candidate] = await Promise.all([
    readFile(path.resolve(root, args.baseline), 'utf8').then(JSON.parse),
    readFile(path.resolve(root, args.candidate), 'utf8').then(JSON.parse),
  ]);
  const result = compareReports(baseline, candidate, args);
  console.log(`Eval ${result.suite}: ${result.ok ? 'sem regressão' : 'regressão detectada'}`);
  for (const item of result.checks) {
    if (item.name === 'same_cases') {
      const baselineCount = new Set(item.baseline).size;
      const candidateCount = new Set(item.candidate).size;
      console.log(`${item.ok ? 'PASS' : 'FAIL'} casos e repetições: ${baselineCount}/${candidateCount} casos, ${item.baseline.length}/${item.candidate.length} tentativas`);
    } else {
      console.log(`${item.ok ? 'PASS' : 'FAIL'} ${item.name}: ${item.baseline} → ${item.candidate}`);
    }
  }
  if (!result.ok) process.exitCode = 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`FALHA: ${error.message}`);
    process.exitCode = 1;
  });
}
