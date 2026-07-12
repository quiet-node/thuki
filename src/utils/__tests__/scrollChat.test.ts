import { describe, it, expect } from 'vitest';
import { pinChatMessagesToBottom } from '../scrollChat';

describe('pinChatMessagesToBottom', () => {
  it('no-ops when fromEl is null', () => {
    expect(() => pinChatMessagesToBottom(null)).not.toThrow();
  });

  it('no-ops when no .chat-messages-scroll ancestor exists', () => {
    const root = document.createElement('div');
    const child = document.createElement('div');
    root.appendChild(child);
    document.body.appendChild(root);

    let scrollTop = 50;
    Object.defineProperty(child, 'scrollTop', {
      get: () => scrollTop,
      set: (v: number) => {
        scrollTop = v;
      },
      configurable: true,
    });

    pinChatMessagesToBottom(child);
    expect(scrollTop).toBe(50);

    root.remove();
  });

  it('sets scrollTop = scrollHeight on the nearest .chat-messages-scroll ancestor', () => {
    const scroller = document.createElement('div');
    scroller.className = 'chat-messages-scroll px-5';
    const mid = document.createElement('div');
    const leaf = document.createElement('div');
    scroller.appendChild(mid);
    mid.appendChild(leaf);
    document.body.appendChild(scroller);

    let scrollTop = 0;
    Object.defineProperty(scroller, 'scrollHeight', {
      get: () => 1200,
      configurable: true,
    });
    Object.defineProperty(scroller, 'scrollTop', {
      get: () => scrollTop,
      set: (v: number) => {
        scrollTop = v;
      },
      configurable: true,
    });

    pinChatMessagesToBottom(leaf);
    expect(scrollTop).toBe(1200);

    scroller.remove();
  });

  it('pins when fromEl itself is the scroller', () => {
    const scroller = document.createElement('div');
    scroller.className = 'chat-messages-scroll';
    document.body.appendChild(scroller);

    let scrollTop = 0;
    Object.defineProperty(scroller, 'scrollHeight', {
      get: () => 500,
      configurable: true,
    });
    Object.defineProperty(scroller, 'scrollTop', {
      get: () => scrollTop,
      set: (v: number) => {
        scrollTop = v;
      },
      configurable: true,
    });

    pinChatMessagesToBottom(scroller);
    expect(scrollTop).toBe(500);

    scroller.remove();
  });

  it('uses the nearest matching ancestor when nested', () => {
    const outer = document.createElement('div');
    outer.className = 'chat-messages-scroll';
    const inner = document.createElement('div');
    inner.className = 'chat-messages-scroll';
    const leaf = document.createElement('div');
    outer.appendChild(inner);
    inner.appendChild(leaf);
    document.body.appendChild(outer);

    let outerTop = 0;
    let innerTop = 0;
    Object.defineProperty(outer, 'scrollHeight', {
      get: () => 999,
      configurable: true,
    });
    Object.defineProperty(outer, 'scrollTop', {
      get: () => outerTop,
      set: (v: number) => {
        outerTop = v;
      },
      configurable: true,
    });
    Object.defineProperty(inner, 'scrollHeight', {
      get: () => 400,
      configurable: true,
    });
    Object.defineProperty(inner, 'scrollTop', {
      get: () => innerTop,
      set: (v: number) => {
        innerTop = v;
      },
      configurable: true,
    });

    pinChatMessagesToBottom(leaf);
    expect(innerTop).toBe(400);
    expect(outerTop).toBe(0);

    outer.remove();
  });
});
