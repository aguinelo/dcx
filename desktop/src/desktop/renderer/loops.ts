import type { DcxLoopSummary } from '../../dcx/client.ts';

export interface LoopView {
  progress: string;
  percent: number;
  statusLabel: string;
  detail: string | null;
  spend: string;
  canPause: boolean;
  canResume: boolean;
}

const money = (value: number): string =>
  `US$ ${value.toLocaleString('pt-BR', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;

function plural(count: number, singular: string, pluralForm: string): string {
  return count === 1 ? singular : pluralForm;
}

function spendLine(summary: DcxLoopSummary): string {
  const { usage, budget } = summary;
  let cost = money(usage.known_cost_usd);
  if (!usage.cost_complete) cost += '+ (custo incompleto)';
  if (budget.max_cost_usd != null) cost += ` de ${money(budget.max_cost_usd)}`;

  const callsLabel = plural(budget.max_model_calls ?? usage.model_calls, 'chamada', 'chamadas');
  const calls =
    budget.max_model_calls != null
      ? `${usage.model_calls} de ${budget.max_model_calls} ${callsLabel}`
      : `${usage.model_calls} ${callsLabel}`;

  const minutes = Math.floor(usage.duration_ms / 60_000);
  const duration =
    budget.max_duration_minutes != null
      ? `${minutes} de ${budget.max_duration_minutes} min`
      : minutes === 0
        ? 'menos de 1 min'
        : `${minutes} min`;

  return [cost, calls, duration].join(' · ');
}

/** What the queue panel shows for one queue; kept DOM-free so it is testable. */
export function describeLoop(summary: DcxLoopSummary): LoopView {
  const running = summary.status === 'running';
  const paused = summary.status === 'paused';

  let statusLabel = 'Concluída';
  if (running) statusLabel = summary.pause_requested ? 'Pausando…' : 'Em execução';
  else if (paused) statusLabel = 'Pausada';

  let detail: string | null = null;
  if (running && summary.current_task) detail = `Agora: ${summary.current_task}`;
  else if (paused) detail = summary.reason ?? 'Pronta para retomar';

  return {
    progress: `${summary.done}/${summary.total} ${plural(summary.total, 'tarefa', 'tarefas')}`,
    percent: summary.total === 0 ? 0 : Math.round((summary.done / summary.total) * 100),
    statusLabel,
    detail,
    spend: spendLine(summary),
    canPause: running && !summary.pause_requested,
    canResume: paused,
  };
}
