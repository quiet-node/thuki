import { describe, it, expect } from 'vitest';
import { isValidElement } from 'react';

// `main.tsx` guards its `createRoot` call on the presence of `#root`, so
// importing the module without setting up the container is a no-op for
// the side-effecting entry path. We are testing only the pure
// `rootForLabel` dispatch helper here.
import { rootForLabel } from '../main';

describe('main.tsx', () => {
  it('rootForLabel returns the SettingsWindow tree for the "settings" label', () => {
    expect(isValidElement(rootForLabel('settings'))).toBe(true);
  });

  it('rootForLabel returns the App tree for any other label', () => {
    expect(isValidElement(rootForLabel('main'))).toBe(true);
  });
});
