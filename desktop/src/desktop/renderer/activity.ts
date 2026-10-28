type ActivityInput = Record<string, unknown>;

function inputText(input: ActivityInput, key: string): string | undefined {
  const value = input[key];
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function basename(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}

export function describeActivity(name: string, value: unknown): string {
  const input = value && typeof value === 'object' ? (value as ActivityInput) : {};
  switch (name.toLowerCase()) {
    case 'read': {
      const file = inputText(input, 'file_path');
      return file ? `Lendo ${basename(file)}` : 'Lendo arquivo…';
    }
    case 'grep': {
      const pattern = inputText(input, 'pattern');
      const path = inputText(input, 'path');
      if (pattern && path) return `Buscando “${pattern}” em ${basename(path)}`;
      if (pattern) return `Buscando “${pattern}” no projeto`;
      return 'Buscando no projeto…';
    }
    case 'glob': {
      const pattern = inputText(input, 'pattern');
      return pattern ? `Localizando arquivos “${pattern}”` : 'Localizando arquivos…';
    }
    default:
      return `Executando ${name}`;
  }
}

export function describeProgress(message: string): string {
  const tool = /^Usando\s+([\w-]+)/i.exec(message)?.[1];
  if (tool) return describeActivity(tool, {});
  const normalized = message.trim().replace(/…+$/, '').toLowerCase();
  if (normalized === 'pesquisando na web') return 'Pesquisando na web…';
  if (normalized === 'pesquisa concluída; analisando fontes') return 'Analisando fontes…';
  return 'Trabalhando no seu pedido…';
}

export function formatActivityElapsed(milliseconds: number): string {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  const minutes = Math.floor(seconds / 60);
  const remaining = seconds % 60;
  if (minutes === 0) return `${seconds} s`;
  return `${minutes} min ${remaining} s`;
}
