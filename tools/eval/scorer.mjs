export function check(actual, spec) {
	const text = actual.text ?? "";
	switch (spec.type) {
		case "contains":
			return text.toLowerCase().includes(String(spec.value).toLowerCase());
		case "not_contains":
			return !text.toLowerCase().includes(String(spec.value).toLowerCase());
		case "regex":
			return new RegExp(spec.value, spec.flags ?? "i").test(text);
		case "not_regex":
			return !new RegExp(spec.value, spec.flags ?? "i").test(text);
		case "equals":
			return text.trim() === String(spec.value).trim();
		case "file_exists":
			return actual.files?.get(spec.path) === true;
		case "file_not_exists":
			return actual.files?.get(spec.path) === false;
		default:
			throw new Error(`check desconhecido: ${spec.type}`);
	}
}

/**
 * Invariants every case must hold regardless of its own checks. Without
 * them a case whose checks are all negative (`not_contains`,
 * `file_not_exists`) passes when the model returns nothing at all.
 */
function invariantFailures(actual) {
	const failures = [];
	if ((actual.text ?? "").trim() === "") {
		failures.push({ type: "non_empty_reply", expected: "resposta não vazia" });
	}
	const cliError = (actual.stderr ?? "").match(/^Erro: .*$/m);
	if (cliError) {
		failures.push({ type: "no_cli_error", expected: cliError[0] });
	}
	return failures;
}

export function scoreCase(actual, checks) {
	const failures = invariantFailures(actual);
	checks.forEach((spec, index) => {
		try {
			if (!check(actual, spec)) {
				failures.push({
					index,
					type: spec.type,
					expected: spec.value ?? spec.path ?? null,
				});
			}
		} catch (error) {
			failures.push({ index, type: spec.type, error: error.message });
		}
	});
	return { ok: failures.length === 0, failures };
}

export function percentile(values, percent) {
	if (values.length === 0) return null;
	const sorted = [...values].sort((left, right) => left - right);
	const index = Math.min(
		sorted.length - 1,
		Math.max(0, Math.ceil((percent / 100) * sorted.length) - 1),
	);
	return sorted[index];
}

export function summarize(results) {
	const total = results.length;
	const passed = results.filter((result) => result.ok).length;
	const costs = results
		.map((result) => result.cost_usd)
		.filter((value) => typeof value === "number");
	const durations = results
		.map((result) => result.duration_ms)
		.filter((value) => typeof value === "number");
	const inputTokens = results
		.map((result) => result.usage?.input_tokens)
		.filter((value) => typeof value === "number");
	const outputTokens = results
		.map((result) => result.usage?.output_tokens)
		.filter((value) => typeof value === "number");
	const byCase = new Map();
	for (const result of results) {
		const attempts = byCase.get(result.id) ?? [];
		attempts.push(result);
		byCase.set(result.id, attempts);
	}
	const casesPassed = [...byCase.values()].filter((attempts) =>
		attempts.some((attempt) => attempt.ok),
	).length;

	return {
		total,
		passed,
		failed: total - passed,
		pass_rate: total === 0 ? 0 : passed / total,
		pass_at_k: byCase.size === 0 ? 0 : casesPassed / byCase.size,
		cost_usd: costs.reduce((sum, value) => sum + value, 0),
		cost_complete: costs.length === total,
		latency_ms: {
			p50: percentile(durations, 50),
			p95: percentile(durations, 95),
		},
		tokens: {
			input: inputTokens.reduce((sum, value) => sum + value, 0),
			output: outputTokens.reduce((sum, value) => sum + value, 0),
			complete: inputTokens.length === total && outputTokens.length === total,
		},
	};
}
