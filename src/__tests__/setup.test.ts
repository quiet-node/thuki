import { describe, it, expect } from 'vitest';

describe('Test setup', () => {
  it('loads jest-dom matchers', () => {
    const div = document.createElement('div');
    document.body.appendChild(div);
    expect(div).toBeInTheDocument();
    document.body.removeChild(div);
  });

  it('has ResizeObserver mock', () => {
    expect(ResizeObserver).toBeDefined();
  });

  it('has clipboard mock', () => {
    expect(navigator.clipboard.writeText).toBeDefined();
  });
});
