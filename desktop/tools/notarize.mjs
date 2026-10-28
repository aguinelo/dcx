import { spawnSync } from 'node:child_process';
import { access, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

function run(command, args) {
  const result = spawnSync(command, args, { stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} terminou com código ${result.status ?? 'desconhecido'}`);
  }
}

function credentials() {
  const profile = process.env.APPLE_KEYCHAIN_PROFILE?.trim();
  if (profile) return ['--keychain-profile', profile];

  const appleId = process.env.APPLE_ID?.trim();
  const password = process.env.APPLE_APP_SPECIFIC_PASSWORD?.trim();
  const teamId = process.env.APPLE_TEAM_ID?.trim();
  if (appleId && password && teamId) {
    return ['--apple-id', appleId, '--password', password, '--team-id', teamId];
  }
  return undefined;
}

export default async function notarize(context) {
  if (process.platform !== 'darwin') return;
  if (process.env.DCX_SKIP_NOTARIZE === '1') {
    console.log('Notarização ignorada por DCX_SKIP_NOTARIZE=1.');
    return;
  }

  const auth = credentials();
  if (!auth) {
    console.log(
      'Notarização ignorada: defina APPLE_KEYCHAIN_PROFILE ou APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD e APPLE_TEAM_ID.',
    );
    return;
  }

  const appPath = path.join(
    context.appOutDir,
    `${context.packager.appInfo.productFilename}.app`,
  );
  await access(appPath);

  const signature = spawnSync('codesign', ['-dv', '--verbose=2', appPath], {
    encoding: 'utf8',
  });
  const signatureText = `${signature.stdout ?? ''}\n${signature.stderr ?? ''}`;
  if (signature.status !== 0 || signatureText.includes('Signature=adhoc')) {
    console.log('Notarização ignorada: o app não tem assinatura Developer ID.');
    return;
  }

  const temporary = await mkdtemp(path.join(tmpdir(), 'dcx-notarize-'));
  const archive = path.join(temporary, 'dcode.zip');
  try {
    console.log(`Compactando ${appPath} para notarização...`);
    run('ditto', ['-c', '-k', '--keepParent', appPath, archive]);
    console.log('Enviando para o notarytool da Apple...');
    run('xcrun', ['notarytool', 'submit', archive, ...auth, '--wait']);
    console.log('Aplicando o ticket de notarização...');
    run('xcrun', ['stapler', 'staple', appPath]);
    run('xcrun', ['stapler', 'validate', appPath]);
    console.log('Notarização concluída e ticket aplicado.');
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}
