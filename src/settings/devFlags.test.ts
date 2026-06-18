import { afterEach, describe, expect, it, vi } from 'vitest';

/**
 * The flag is a module-level const evaluated at import, so each case stubs the
 * env, resets the module registry, and re-imports to observe the resolved
 * value. This covers both the enabled ("true") and disabled (everything else)
 * evaluations of `import.meta.env.VITE_ENABLE_OPENAI_PROVIDER === 'true'`.
 */
async function loadFlag(): Promise<boolean> {
  vi.resetModules();
  const mod = await import('./devFlags');
  return mod.OPENAI_PROVIDER_ENABLED;
}

describe('OPENAI_PROVIDER_ENABLED', () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it('is true only when VITE_ENABLE_OPENAI_PROVIDER is exactly "true"', async () => {
    vi.stubEnv('VITE_ENABLE_OPENAI_PROVIDER', 'true');
    expect(await loadFlag()).toBe(true);
  });

  it('is false when the env var is unset', async () => {
    vi.stubEnv('VITE_ENABLE_OPENAI_PROVIDER', undefined);
    expect(await loadFlag()).toBe(false);
  });

  it('is false for any other value', async () => {
    vi.stubEnv('VITE_ENABLE_OPENAI_PROVIDER', '1');
    expect(await loadFlag()).toBe(false);
  });
});
