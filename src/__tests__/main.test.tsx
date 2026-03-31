import { describe, it, expect, afterEach } from 'vitest';
import { act } from '@testing-library/react';

describe('main.tsx', () => {
  afterEach(() => {
    const root = document.getElementById('root');
    if (root) document.body.removeChild(root);
  });

  it('mounts React app without throwing', async () => {
    const root = document.createElement('div');
    root.id = 'root';
    document.body.appendChild(root);

    await act(async () => {
      await expect(import('../main')).resolves.toBeDefined();
    });
    expect(root.childNodes.length).toBeGreaterThan(0);
  });
});
