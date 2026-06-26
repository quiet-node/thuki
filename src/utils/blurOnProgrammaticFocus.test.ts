import { describe, it, expect, vi } from 'vitest';
import type { FocusEvent } from 'react';
import { blurOnProgrammaticFocus } from './blurOnProgrammaticFocus';

function focusEvent(relatedTarget: EventTarget | null, blur: () => void) {
  return {
    relatedTarget,
    currentTarget: { blur } as unknown as HTMLElement,
  } as unknown as FocusEvent<HTMLElement>;
}

describe('blurOnProgrammaticFocus', () => {
  it('blurs the element when focus arrives with no relatedTarget (programmatic refocus)', () => {
    const blur = vi.fn();
    blurOnProgrammaticFocus(focusEvent(null, blur));
    expect(blur).toHaveBeenCalledTimes(1);
  });

  it('leaves focus intact when a relatedTarget is present (keyboard tab)', () => {
    const blur = vi.fn();
    blurOnProgrammaticFocus(focusEvent(document.createElement('button'), blur));
    expect(blur).not.toHaveBeenCalled();
  });
});
