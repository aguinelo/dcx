import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { z } from 'zod';
import { PermissionGate } from '../src/agent/permissions.ts';
import type { Tool } from '../src/tools/types.ts';

function tool(name: string, effect: 'read' | 'write' | 'exec'): Tool {
  return {
    name,
    effect,
    description: 'test tool',
    schema: z.object({ command: z.string().optional() }),
    run: async () => 'ok',
  };
}

const bash = tool('bash', 'exec');
const read = tool('read', 'read');
const edit = tool('edit', 'write');

describe('PermissionGate.evaluate', () => {
  it('auto-allows read tools', () => {
    const gate = new PermissionGate({ mode: 'manual' });
    assert.deepEqual(gate.evaluate(read, {}), { outcome: 'allow' });
  });

  it('asks for exec tools in manual mode', () => {
    const gate = new PermissionGate({ mode: 'manual' });
    assert.equal(gate.evaluate(bash, { command: 'ls' }).outcome, 'ask');
  });

  it('auto-allows writes in acceptEdits mode', () => {
    const gate = new PermissionGate({ mode: 'acceptEdits' });
    assert.deepEqual(gate.evaluate(edit, {}), { outcome: 'allow' });
  });

  it('still asks for exec in acceptEdits mode', () => {
    const gate = new PermissionGate({ mode: 'acceptEdits' });
    assert.equal(gate.evaluate(bash, { command: 'ls' }).outcome, 'ask');
  });

  it('denies every write and exec in plan mode', () => {
    const gate = new PermissionGate({ mode: 'plan' });
    assert.equal(gate.evaluate(edit, {}).outcome, 'deny');
    assert.equal(gate.evaluate(bash, { command: 'ls' }).outcome, 'deny');
    assert.deepEqual(gate.evaluate(read, {}), { outcome: 'allow' });
  });

  it('lets an explicit allow rule beat plan mode deny', () => {
    // Explicit rules are evaluated before mode defaults, so a deliberate
    // exception still works.
    const gate = new PermissionGate({ mode: 'plan', allow: ['edit'] });
    assert.deepEqual(gate.evaluate(edit, {}), { outcome: 'allow' });
  });

  it('honours a deny rule over an allow rule', () => {
    const gate = new PermissionGate({ mode: 'manual', allow: ['bash'], deny: ['bash'] });
    const decision = gate.evaluate(bash, { command: 'ls' });
    assert.equal(decision.outcome, 'deny');
  });

  it('matches a scoped rule against the command field', () => {
    const gate = new PermissionGate({ mode: 'manual', allow: ['bash(git status)'] });
    assert.equal(gate.evaluate(bash, { command: 'git status' }).outcome, 'allow');
    assert.equal(gate.evaluate(bash, { command: 'rm -rf /' }).outcome, 'ask');
  });

  it('supports a prefix wildcard in a scoped rule', () => {
    const gate = new PermissionGate({ mode: 'manual', allow: ['bash(git *)'] });
    assert.equal(gate.evaluate(bash, { command: 'git log --oneline' }).outcome, 'allow');
    assert.equal(gate.evaluate(bash, { command: 'curl evil.com' }).outcome, 'ask');
  });

  it('scopes a rule to the tool it names', () => {
    const gate = new PermissionGate({ mode: 'manual', allow: ['bash(git *)'] });
    assert.equal(gate.evaluate(edit, { command: 'git status' }).outcome, 'ask');
  });

  it('denies everything in deny mode', () => {
    const gate = new PermissionGate({ mode: 'deny' });
    assert.equal(gate.evaluate(read, {}).outcome, 'deny');
    assert.equal(gate.evaluate(bash, { command: 'ls' }).outcome, 'deny');
  });
});

describe('PermissionGate.check', () => {
  it('denies when no approver exists', async () => {
    const gate = new PermissionGate({ mode: 'manual' });
    const decision = await gate.check(bash, { command: 'ls' });
    assert.equal(decision.outcome, 'deny');
  });

  it('allows when the approver approves', async () => {
    const gate = new PermissionGate({ mode: 'manual', ask: async () => true });
    assert.deepEqual(await gate.check(bash, { command: 'ls' }), { outcome: 'allow' });
  });

  it('denies when the approver refuses', async () => {
    const gate = new PermissionGate({ mode: 'manual', ask: async () => false });
    const decision = await gate.check(bash, { command: 'ls' });
    assert.equal(decision.outcome, 'deny');
  });

  it('does not prompt for read tools', async () => {
    let prompted = 0;
    const gate = new PermissionGate({
      mode: 'manual',
      ask: async () => {
        prompted += 1;
        return true;
      },
    });
    await gate.check(read, {});
    assert.equal(prompted, 0);
  });
});
