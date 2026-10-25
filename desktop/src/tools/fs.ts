import { mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { z } from 'zod';
import { SandboxError, resolveUnderRoot } from '../fs/sandbox.ts';
import { type Tool, type ToolContext, ToolError } from './types.ts';

/** Cap on a single Read result, mirroring the observed tool error wording. */
export const MAX_READ_BYTES = 400_000;
/** Edit loads the whole file into memory, so it gets a tighter cap than Read. */
export const MAX_EDIT_BYTES = 200_000;

function isNotFound(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    (error as NodeJS.ErrnoException).code === 'ENOENT'
  );
}

/** Refuse binary content early: the tools contract is UTF-8 text. */
function assertUtf8(buffer: Buffer, filePath: string): void {
  if (buffer.subarray(0, 8000).includes(0)) {
    throw new ToolError(`read: ${filePath} is not valid utf-8`);
  }
}

function resolveOrThrow(ctx: ToolContext, filePath: string): string {
  try {
    return resolveUnderRoot(filePath, ctx.sandbox);
  } catch (error) {
    if (error instanceof SandboxError) throw new ToolError(`read/write/edit: ${error.message}`);
    throw error;
  }
}

const ReadInput = z.object({
  file_path: z.string().min(1),
  view_range: z.tuple([z.number().int().positive(), z.number().int().positive()]).optional(),
});

export const readTool: Tool<typeof ReadInput> = {
  name: 'read',
  description:
    'Read a UTF-8 text file relative to the workdir. view_range is [start_line, end_line], 1-indexed inclusive.',
  effect: 'read',
  schema: ReadInput,

  async run(input, ctx) {
    const resolved = resolveOrThrow(ctx, input.file_path);

    let stat: ReturnType<typeof statSync>;
    try {
      stat = statSync(resolved);
    } catch (error) {
      if (isNotFound(error))
        throw new ToolError(`read: file not found: ${input.file_path}`);
      throw error;
    }

    if (stat.isDirectory()) throw new ToolError(`read: ${input.file_path} is a directory`);

    if (stat.size > MAX_READ_BYTES && !input.view_range) {
      throw new ToolError(
        `read: ${stat.size}-byte limit. Use the view_range parameter to read specific line ranges, e.g. view_range: [1, 500].`,
      );
    }

    const buffer = readFileSync(resolved);
    assertUtf8(buffer, input.file_path);
    const text = buffer.toString('utf8');

    if (!input.view_range) return text;

    const [start, end] = input.view_range;
    if (start > end) throw new ToolError('read: view_range must be [start_line, end_line]');

    const lines = text.split('\n');
    const last = Math.min(end, lines.length);
    if (start > lines.length) {
      throw new ToolError(`read: view_range starts past end of file (${lines.length} lines)`);
    }

    const slice = lines.slice(start - 1, last);
    if (slice.length === 1 && (slice[0]?.length ?? 0) > MAX_READ_BYTES) {
      throw new ToolError(
        `read: line ${start} alone exceeds ${MAX_READ_BYTES}-byte limit. The read tool cannot return part of a line, so view_range cannot narrow this further.`,
      );
    }

    return slice.join('\n');
  },
};

const WriteInput = z.object({
  file_path: z.string().min(1),
  content: z.string(),
});

export const writeTool: Tool<typeof WriteInput> = {
  name: 'write',
  description:
    'Write a UTF-8 text file relative to the workdir, creating parent directories as needed.',
  effect: 'write',
  schema: WriteInput,

  async run(input, ctx) {
    const resolved = resolveOrThrow(ctx, input.file_path);
    mkdirSync(dirname(resolved), { recursive: true });
    writeFileSync(resolved, input.content, 'utf8');
    return `wrote ${input.content.length} bytes to ${input.file_path}`;
  },
};

const EditInput = z.object({
  file_path: z.string().min(1),
  old_string: z.string().min(1),
  new_string: z.string(),
  replace_all: z.boolean().optional(),
});

export const editTool: Tool<typeof EditInput> = {
  name: 'edit',
  description:
    'Replace old_string with new_string in a file. old_string must be unique unless replace_all is set.',
  effect: 'write',
  schema: EditInput,

  async run(input, ctx) {
    const resolved = resolveOrThrow(ctx, input.file_path);

    let buffer: Buffer;
    try {
      buffer = readFileSync(resolved);
    } catch (error) {
      if (isNotFound(error)) throw new ToolError(`edit: ${input.file_path} not found`);
      throw error;
    }

    assertUtf8(buffer, input.file_path);

    if (buffer.byteLength > MAX_EDIT_BYTES) {
      throw new ToolError(
        `edit: ${input.file_path} bytes, exceeds ${MAX_EDIT_BYTES}-byte limit. The edit tool loads the whole file and cannot modify a file this large.`,
      );
    }

    const text = buffer.toString('utf8');
    const occurrences = text.split(input.old_string).length - 1;

    if (occurrences === 0) throw new ToolError(`edit: old_string not found in ${input.file_path}`);
    if (occurrences > 1 && !input.replace_all) {
      throw new ToolError(
        `edit: old_string appears ${occurrences} times in ${input.file_path} (must be unique). Pass replace_all to replace every occurrence.`,
      );
    }

    const updated = input.replace_all
      ? text.split(input.old_string).join(input.new_string)
      : text.replace(input.old_string, input.new_string);

    writeFileSync(resolved, updated, 'utf8');
    const count = input.replace_all ? occurrences : 1;
    return `${count} edited replacement(s) in ${input.file_path}`;
  },
};
