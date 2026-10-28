import test from "node:test";
import assert from "node:assert/strict";

import { scoreContract } from "./contract-quality.mjs";

const focusedContract = {
	objective: "Criar resultado.txt com o texto pedido.",
	criteria: [
		{ id: "file-content", description: "resultado.txt contém DCX_RUN_E2E_OK." },
	],
	interface: { file: "resultado.txt" },
};

const focusedSensors = {
	sensors: [
		{ id: "visible-content", visibility: "visible", check: "Ler resultado.txt e comparar o texto." },
		{ id: "hidden-content", visibility: "hidden", check: "Comparar os bytes de resultado.txt." },
	],
};

test("a concise contract for the requested file passes", () => {
	const result = scoreContract(focusedContract, focusedSensors, "resultado.txt");
	assert.equal(result.ok, true);
	assert.deepEqual(result.failures, []);
});

test("a contract that expands a simple task into too many checks fails", () => {
	const contract = {
		...focusedContract,
		criteria: Array.from({ length: 6 }, (_, index) => ({
			id: `criterion-${index}`,
			description: `Verificação extra ${index}.`,
		})),
	};
	const sensors = {
		sensors: Array.from({ length: 9 }, (_, index) => ({
			id: `sensor-${index}`,
			visibility: index === 8 ? "hidden" : "visible",
			check: `Executar verificação ${index}.`,
		})),
	};

	const result = scoreContract(contract, sensors, "resultado.txt");
	assert.equal(result.ok, false);
	assert.ok(result.failures.includes("mais de 3 critérios para uma tarefa simples"));
	assert.ok(result.failures.includes("mais de 5 sensores para uma tarefa simples"));
});

test("a contract cannot turn Harness metadata into product requirements", () => {
	const sensors = structuredClone(focusedSensors);
	sensors.sensors[1].check = "Verificar o conteúdo interno de .dcx/lock e o PID do processo.";

	const result = scoreContract(focusedContract, sensors, "resultado.txt");
	assert.equal(result.ok, false);
	assert.ok(result.failures.includes("contrato depende de metadados internos do Harness"));
});

test("a contract must name the requested file in its observable behavior", () => {
	const contract = structuredClone(focusedContract);
	contract.criteria[0].description = "O pedido foi concluído.";
	const sensors = structuredClone(focusedSensors);
	for (const sensor of sensors.sensors) sensor.check = "Validar o resultado.";
	const result = scoreContract(contract, sensors, "resultado.txt");
	assert.equal(result.ok, false);
	assert.ok(result.failures.includes("resultado pedido não aparece em critério ou sensor"));
});
