const INTERNAL_HARNESS_DETAILS = /(?:\.dcx(?:\/|\b)|\bpid\b|process id|processo\s+id|shasum|lock interno|hash(?:eado|\s+do)?\s+lock)/i;

function collectContractText(contract, sensorSuite) {
	return [
		contract?.objective,
		contract?.interface,
		contract?.assumptions,
		contract?.criteria,
		contract?.constraints,
		contract?.out_of_scope,
		sensorSuite?.sensors,
	]
		.map((part) => (typeof part === "string" ? part : JSON.stringify(part ?? "")))
		.join("\n");
}

export function scoreContract(contract, sensorSuite, expectedArtifact, limits = {}) {
	const failures = [];
	const criteria = contract?.criteria;
	const sensors = sensorSuite?.sensors;
	const maxCriteria = limits.max_criteria ?? 3;
	const maxSensors = limits.max_sensors ?? 5;
	if (!Array.isArray(criteria) || criteria.length === 0) {
		failures.push("contrato sem critérios observáveis");
	} else if (criteria.length > maxCriteria) {
		failures.push(`mais de ${maxCriteria} critérios para uma tarefa simples`);
	}
	if (!Array.isArray(sensors) || sensors.length === 0) {
		failures.push("contrato sem sensores de aceite");
	} else if (sensors.length > maxSensors) {
		failures.push(`mais de ${maxSensors} sensores para uma tarefa simples`);
	}
	const text = collectContractText(contract, sensorSuite);
	if (INTERNAL_HARNESS_DETAILS.test(text)) {
		failures.push("contrato depende de metadados internos do Harness");
	}
	const observableBehavior = JSON.stringify({
		criteria: criteria ?? [],
		sensors: sensors ?? [],
	});
	if (!observableBehavior.includes(expectedArtifact)) {
		failures.push("resultado pedido não aparece em critério ou sensor");
	}
	return { ok: failures.length === 0, failures };
}
