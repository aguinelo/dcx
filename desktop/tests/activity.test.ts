import assert from 'node:assert/strict';
import test from 'node:test';
import {
  activityIndicatorMarkup,
  describeActivity,
  describeProgress,
  formatActivityElapsed,
} from '../src/desktop/renderer/activity.ts';

test('activity status uses the compact four-dot indicator from the reference', () => {
  assert.equal(
    activityIndicatorMarkup(),
    '<span class="thinking__spinner" aria-hidden="true"><i></i><i></i><i></i><i></i></span>',
  );
});

test('read activity is described in plain Portuguese with the file name', () => {
  assert.equal(
    describeActivity('Read', { file_path: '/workspace/sales-api/README.md' }),
    'Lendo README.md',
  );
});

test('search activity describes the real query', () => {
  assert.equal(
    describeActivity('Grep', { pattern: 'deploy', path: 'src' }),
    'Buscando “deploy” em src',
  );
});

test('unknown activity uses a readable localized fallback', () => {
  assert.equal(describeActivity('CustomTool', {}), 'Executando CustomTool');
});

test('provider progress messages are shown in Portuguese', () => {
  assert.equal(describeProgress('Usando Read…'), 'Lendo arquivo…');
  assert.equal(describeProgress('Usando Grep…'), 'Buscando no projeto…');
});

test('elapsed time uses seconds then minutes and seconds', () => {
  assert.equal(formatActivityElapsed(49_000), '49 s');
  assert.equal(formatActivityElapsed(125_000), '2 min 5 s');
});

test('untrusted progress text is not shown as a reasoning trace', () => {
  assert.equal(
    describeProgress('Let me think through the problem carefully'),
    'Trabalhando no seu pedido…',
  );
});

test('web progress is mapped to short user-facing status', () => {
  assert.equal(describeProgress('Pesquisando na web…'), 'Pesquisando na web…');
  assert.equal(describeProgress('Pesquisa concluída; analisando fontes…'), 'Analisando fontes…');
});
