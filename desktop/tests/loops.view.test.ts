import assert from 'node:assert/strict';
import test from 'node:test';
import type { DcxLoopSummary } from '../src/dcx/client.ts';
import { describeLoop } from '../src/desktop/renderer/loops.ts';

function summary(overrides: Partial<DcxLoopSummary> = {}): DcxLoopSummary {
  return {
    id: 'loop-1',
    status: 'running',
    reason: null,
    done: 1,
    total: 3,
    current_task: 'task-2',
    pause_requested: false,
    created_at_unix_ms: 1,
    updated_at_unix_ms: 2,
    tasks: [],
    budget: {},
    usage: { model_calls: 3, known_cost_usd: 0.1234, cost_complete: true, duration_ms: 125_000 },
    ...overrides,
  };
}

test('a running queue shows the current task and can be paused', () => {
  const view = describeLoop(summary());
  assert.equal(view.statusLabel, 'Em execução');
  assert.equal(view.detail, 'Agora: task-2');
  assert.equal(view.progress, '1/3 tarefas');
  assert.equal(view.percent, 33);
  assert.equal(view.canPause, true);
  assert.equal(view.canResume, false);
});

test('a pause request shows as pausing and hides the pause button', () => {
  const view = describeLoop(summary({ pause_requested: true }));
  assert.equal(view.statusLabel, 'Pausando…');
  assert.equal(view.canPause, false);
});

test('a paused queue explains why and can be resumed', () => {
  const view = describeLoop(summary({ status: 'paused', reason: 'aguardando aprovação' }));
  assert.equal(view.statusLabel, 'Pausada');
  assert.equal(view.detail, 'aguardando aprovação');
  assert.equal(view.canResume, true);
  assert.equal(view.canPause, false);
});

test('a paused queue without a recorded reason still says what to do', () => {
  const view = describeLoop(summary({ status: 'paused', reason: null }));
  assert.equal(view.detail, 'Pronta para retomar');
});

test('a finished queue has no actions', () => {
  const view = describeLoop(summary({ status: 'done', done: 3, current_task: null }));
  assert.equal(view.statusLabel, 'Concluída');
  assert.equal(view.percent, 100);
  assert.equal(view.detail, null);
  assert.equal(view.canPause, false);
  assert.equal(view.canResume, false);
});

test('spend compares usage with each configured limit', () => {
  const view = describeLoop(
    summary({ budget: { max_cost_usd: 2, max_model_calls: 40, max_duration_minutes: 120 } }),
  );
  assert.equal(view.spend, 'US$ 0,12 de US$ 2,00 · 3 de 40 chamadas · 2 de 120 min');
});

test('spend without limits shows plain usage, singular and incomplete cost', () => {
  const view = describeLoop(
    summary({ usage: { model_calls: 1, known_cost_usd: 0.05, cost_complete: false, duration_ms: 40_000 } }),
  );
  assert.equal(view.spend, 'US$ 0,05+ (custo incompleto) · 1 chamada · menos de 1 min');
});
