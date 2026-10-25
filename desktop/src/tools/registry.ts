import { zodToJsonSchema } from 'zod-to-json-schema';
import type { ToolSpec } from '../provider/types.ts';
import { bashTool } from './bash.ts';
import { editTool, readTool, writeTool } from './fs.ts';
import { globTool, grepTool } from './search.ts';
import type { Tool } from './types.ts';

/** Convert a tool's Zod schema into the JSON Schema the provider expects. */
export function toToolSpec(tool: Tool): ToolSpec {
  const converted = zodToJsonSchema(tool.schema, { target: 'jsonSchema7' }) as {
    properties?: Record<string, unknown>;
    required?: string[];
  };

  return {
    name: tool.name,
    description: tool.description,
    input_schema: {
      type: 'object',
      properties: converted.properties ?? {},
      ...(converted.required ? { required: converted.required } : {}),
    },
  };
}

/** The default built-in tool pool, in the order shown to the model. */
export function defaultTools(): Tool[] {
  return [readTool, writeTool, editTool, globTool, grepTool, bashTool];
}

export class ToolRegistry {
  private readonly byName: Map<string, Tool>;

  constructor(private readonly tools: Tool[]) {
    this.byName = new Map(tools.map((tool) => [tool.name, tool]));
  }

  specs(): ToolSpec[] {
    return this.tools.map((tool) => toToolSpec(tool));
  }

  get(name: string): Tool | undefined {
    return this.byName.get(name);
  }

  names(): string[] {
    return this.tools.map((tool) => tool.name);
  }
}
