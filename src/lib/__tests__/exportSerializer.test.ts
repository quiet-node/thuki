import { describe, it, expect, vi, afterEach } from 'vitest';
import type { Message } from '../../hooks/useOllama';
import {
  defaultExportFilename,
  defaultImageLoader,
  serializeForClipboard,
  serializeForFile,
  type FileExportContext,
  type ImageLoader,
} from '../exportSerializer';

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
}));

function makeMessage(overrides: Partial<Message>): Message {
  return {
    id: 'msg-id',
    role: 'user',
    content: '',
    ...overrides,
  };
}

const CTX: FileExportContext = { fallbackModel: 'default-model' };

const stubImageLoader: ImageLoader = async (path) =>
  `data:image/jpeg;base64,STUB(${path})`;

describe('defaultExportFilename', () => {
  it('formats local date and time with zero padding', () => {
    // 2026-01-09T03:07:00 local
    const filename = defaultExportFilename(new Date(2026, 0, 9, 3, 7, 0));
    expect(filename).toBe('thuki-chat-2026-01-09-0307.md');
  });

  it('formats single-digit month and day with zero padding', () => {
    const filename = defaultExportFilename(new Date(2026, 5, 4, 12, 30, 0));
    expect(filename).toBe('thuki-chat-2026-06-04-1230.md');
  });

  it('formats midnight as 0000', () => {
    const filename = defaultExportFilename(new Date(2026, 11, 31, 0, 0, 0));
    expect(filename).toBe('thuki-chat-2026-12-31-0000.md');
  });

  it('formats double-digit hour and minute correctly', () => {
    const filename = defaultExportFilename(new Date(2026, 4, 24, 14, 30, 15));
    expect(filename).toBe('thuki-chat-2026-05-24-1430.md');
  });

  it('uses the .txt extension when requested', () => {
    const filename = defaultExportFilename(
      new Date(2026, 4, 24, 14, 30, 15),
      'txt',
    );
    expect(filename).toBe('thuki-chat-2026-05-24-1430.txt');
  });

  it('explicitly defaults to .md when no extension is supplied', () => {
    const filename = defaultExportFilename(
      new Date(2026, 4, 24, 14, 30, 15),
      'md',
    );
    expect(filename).toBe('thuki-chat-2026-05-24-1430.md');
  });
});

describe('serializeForFile', () => {
  const NOW = new Date(2026, 4, 24, 14, 30, 15);

  it('emits YAML frontmatter with model, exported_at, message_count', async () => {
    const messages: Message[] = [
      makeMessage({ id: 'u1', role: 'user', content: 'hello' }),
      makeMessage({
        id: 'a1',
        role: 'assistant',
        content: 'hi',
        modelName: 'llama3.2:3b',
      }),
    ];

    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);

    expect(result).toContain('---\napp: Thuki');
    expect(result).toContain('model: llama3.2:3b');
    expect(result).toMatch(/exported_at: 2026-05-24T14:30:15[+-]\d{2}:\d{2}/);
    expect(result).toContain('message_count: 2');
  });

  it('falls back to the supplied fallbackModel when no assistant has modelName', async () => {
    const messages: Message[] = [
      makeMessage({ id: 'u1', role: 'user', content: 'hello' }),
      makeMessage({ id: 'a1', role: 'assistant', content: 'hi' }),
    ];

    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain('model: default-model');
  });

  it('emits "unknown" when no assistant modelName and no fallback', async () => {
    const messages: Message[] = [
      makeMessage({ id: 'u1', role: 'user', content: 'hello' }),
    ];
    const result = await serializeForFile(
      messages,
      { fallbackModel: null },
      NOW,
      stubImageLoader,
    );
    expect(result).toContain('model: unknown');
  });

  it('emits frontmatter even when there are zero messages', async () => {
    const result = await serializeForFile([], CTX, NOW, stubImageLoader);
    expect(result).toContain('message_count: 0');
    // No trailing message body separator.
    expect(result.split('---').length).toBe(3);
  });

  it('renders user messages with the User heading', async () => {
    const messages: Message[] = [
      makeMessage({ id: 'u1', role: 'user', content: 'a question' }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain('## User\n\na question');
  });

  it('renders assistant messages with the model name in parentheses', async () => {
    const messages: Message[] = [
      makeMessage({
        id: 'a1',
        role: 'assistant',
        content: 'an answer',
        modelName: 'qwen:7b',
      }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain('## Assistant (qwen:7b)\n\nan answer');
  });

  it('renders an unattributed assistant message as plain "Assistant"', async () => {
    const messages: Message[] = [
      makeMessage({ id: 'a1', role: 'assistant', content: 'an answer' }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain('## Assistant\n\nan answer');
  });

  it('renders quoted text as a Markdown blockquote', async () => {
    const messages: Message[] = [
      makeMessage({
        id: 'u1',
        role: 'user',
        content: 'follow up',
        quotedText: 'first line\nsecond line',
      }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain('> first line\n> second line');
  });

  it('renders thinking content inside a collapsed details block', async () => {
    const messages: Message[] = [
      makeMessage({
        id: 'a1',
        role: 'assistant',
        content: 'final answer',
        thinkingContent: 'step 1\nstep 2',
        modelName: 'thinker:7b',
      }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain(
      '<details>\n<summary>Thinking</summary>\n\nstep 1\nstep 2\n\n</details>',
    );
  });

  it('renders search sources as a numbered list', async () => {
    const messages: Message[] = [
      makeMessage({
        id: 'a1',
        role: 'assistant',
        content: 'See sources.',
        searchSources: [
          { title: 'First', url: 'https://example.com/one' },
          { title: 'Second', url: 'https://example.com/two' },
        ],
      }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain('**Sources** (`/search`):');
    expect(result).toContain('1. [First](https://example.com/one)');
    expect(result).toContain('2. [Second](https://example.com/two)');
  });

  it('uses the source URL as the link label when the title is empty', async () => {
    const messages: Message[] = [
      makeMessage({
        id: 'a1',
        role: 'assistant',
        content: 'See',
        searchSources: [{ title: '', url: 'https://nowhere.example/page' }],
      }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain(
      '1. [https://nowhere.example/page](https://nowhere.example/page)',
    );
  });

  it('skips the sources section entirely when none are present', async () => {
    const messages: Message[] = [
      makeMessage({ id: 'a1', role: 'assistant', content: 'no sources' }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).not.toContain('**Sources**');
  });

  it('inlines images as data URIs from the supplied loader', async () => {
    const messages: Message[] = [
      makeMessage({
        id: 'u1',
        role: 'user',
        content: 'look at this',
        imagePaths: ['/Users/me/screen.jpg'],
      }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain(
      '![Screenshot](data:image/jpeg;base64,STUB(/Users/me/screen.jpg))',
    );
  });

  it('renders multiple images for a single message on separate lines', async () => {
    const messages: Message[] = [
      makeMessage({
        id: 'u1',
        role: 'user',
        content: '',
        imagePaths: ['/a.jpg', '/b.jpg'],
      }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain(
      '![Screenshot](data:image/jpeg;base64,STUB(/a.jpg))\n\n![Screenshot](data:image/jpeg;base64,STUB(/b.jpg))',
    );
  });

  it('falls back to a textual marker when an image loader rejects', async () => {
    const failingLoader: ImageLoader = async () => {
      throw new Error('not found');
    };
    const messages: Message[] = [
      makeMessage({
        id: 'u1',
        role: 'user',
        content: '',
        imagePaths: ['/Users/me/missing.jpg'],
      }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, failingLoader);
    expect(result).toContain('_[Screenshot unavailable: missing.jpg]_');
    expect(result).not.toContain('data:image/jpeg');
  });

  it('separates consecutive messages with a horizontal rule', async () => {
    const messages: Message[] = [
      makeMessage({ id: 'u1', role: 'user', content: 'first' }),
      makeMessage({ id: 'a1', role: 'assistant', content: 'second' }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, stubImageLoader);
    expect(result).toContain('first\n\n---\n\n## Assistant');
  });

  it('uses a "+" sign in exported_at for timezones east of UTC', async () => {
    // getTimezoneOffset is NEGATIVE east of UTC (e.g., JST = -540).
    const spy = vi
      .spyOn(Date.prototype, 'getTimezoneOffset')
      .mockReturnValue(-540);
    try {
      const result = await serializeForFile(
        [makeMessage({ role: 'user', content: 'hi' })],
        CTX,
        new Date(2026, 4, 24, 14, 30, 15),
        stubImageLoader,
      );
      expect(result).toMatch(/exported_at: 2026-05-24T14:30:15\+09:00/);
    } finally {
      spy.mockRestore();
    }
  });

  it('uses a "-" sign in exported_at for timezones west of UTC', async () => {
    // getTimezoneOffset is POSITIVE west of UTC (e.g., EST = +300).
    const spy = vi
      .spyOn(Date.prototype, 'getTimezoneOffset')
      .mockReturnValue(300);
    try {
      const result = await serializeForFile(
        [makeMessage({ role: 'user', content: 'hi' })],
        CTX,
        new Date(2026, 4, 24, 14, 30, 15),
        stubImageLoader,
      );
      expect(result).toMatch(/exported_at: 2026-05-24T14:30:15-05:00/);
    } finally {
      spy.mockRestore();
    }
  });

  it('handles a Windows-style image path in basename fallback', async () => {
    const failingLoader: ImageLoader = async () => {
      throw new Error('boom');
    };
    const messages: Message[] = [
      makeMessage({
        id: 'u1',
        role: 'user',
        content: '',
        imagePaths: ['C:\\Users\\me\\shot.png'],
      }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, failingLoader);
    expect(result).toContain('_[Screenshot unavailable: shot.png]_');
  });

  it('uses the bare path when no slash is present for the basename fallback', async () => {
    const failingLoader: ImageLoader = async () => {
      throw new Error('boom');
    };
    const messages: Message[] = [
      makeMessage({
        id: 'u1',
        role: 'user',
        content: '',
        imagePaths: ['orphan.png'],
      }),
    ];
    const result = await serializeForFile(messages, CTX, NOW, failingLoader);
    expect(result).toContain('_[Screenshot unavailable: orphan.png]_');
  });
});

describe('serializeForClipboard', () => {
  it('omits the YAML frontmatter entirely', () => {
    const messages: Message[] = [
      makeMessage({ id: 'u1', role: 'user', content: 'hi' }),
    ];
    const result = serializeForClipboard(messages);
    expect(result).not.toContain('---\napp: Thuki');
    expect(result).not.toContain('exported_at');
  });

  it('renders role-labelled blocks identical to the file output (text only)', () => {
    const messages: Message[] = [
      makeMessage({ id: 'u1', role: 'user', content: 'hi' }),
      makeMessage({
        id: 'a1',
        role: 'assistant',
        content: 'hello',
        modelName: 'qwen:7b',
      }),
    ];
    const result = serializeForClipboard(messages);
    expect(result).toContain('## User\n\nhi');
    expect(result).toContain('## Assistant (qwen:7b)\n\nhello');
  });

  it('replaces images with a textual marker (no base64 payload)', () => {
    const messages: Message[] = [
      makeMessage({
        id: 'u1',
        role: 'user',
        content: 'look',
        imagePaths: ['/Users/me/photo.jpg'],
      }),
    ];
    const result = serializeForClipboard(messages);
    expect(result).toContain('_[Screenshot: photo.jpg]_');
    expect(result).not.toContain('data:image');
  });

  it('renders multiple image markers on separate lines', () => {
    const messages: Message[] = [
      makeMessage({
        id: 'u1',
        role: 'user',
        content: '',
        imagePaths: ['/a.jpg', '/b.jpg'],
      }),
    ];
    const result = serializeForClipboard(messages);
    expect(result).toContain('_[Screenshot: a.jpg]_\n\n_[Screenshot: b.jpg]_');
  });

  it('keeps thinking blocks and search sources', () => {
    const messages: Message[] = [
      makeMessage({
        id: 'a1',
        role: 'assistant',
        content: 'answer',
        thinkingContent: 'considering',
        searchSources: [{ title: 'Doc', url: 'https://example.com' }],
        modelName: 'qwen:7b',
      }),
    ];
    const result = serializeForClipboard(messages);
    expect(result).toContain('<details>');
    expect(result).toContain('1. [Doc](https://example.com)');
  });

  it('returns an empty string when there are zero messages', () => {
    expect(serializeForClipboard([])).toBe('');
  });

  it('keeps quoted blockquotes', () => {
    const messages: Message[] = [
      makeMessage({
        id: 'u1',
        role: 'user',
        content: 'context',
        quotedText: 'line one',
      }),
    ];
    const result = serializeForClipboard(messages);
    expect(result).toContain('> line one');
  });
});

// `defaultImageLoader` is the production image loader that screenshots flow
// through when the export is triggered for real. It reads an asset-protocol
// URL via `fetch` and base64-encodes the resulting Blob through a FileReader.
// happy-dom ships a real FileReader implementation, so the success path can
// run end-to-end with only `fetch` stubbed. The failure paths swap in a
// fake FileReader because the real implementation never produces a
// non-string result and never fires `onerror` on a freshly fetched Blob.

describe('defaultImageLoader', () => {
  const originalFetch = globalThis.fetch;
  const originalFileReader = globalThis.FileReader;

  afterEach(() => {
    globalThis.fetch = originalFetch;
    globalThis.FileReader = originalFileReader;
  });

  it('reads a file path and returns a data URI', async () => {
    const blob = new Blob(['data'], { type: 'image/png' });
    globalThis.fetch = vi.fn(async () => {
      return {
        blob: async () => blob,
      } as unknown as Response;
    });

    const result = await defaultImageLoader('/path/img.png');
    expect(result).toMatch(/^data:image\/png/);
    expect(globalThis.fetch).toHaveBeenCalledWith('asset:///path/img.png');
  });

  it('rejects when FileReader yields a non-string result', async () => {
    class NonStringResultFileReader {
      result: ArrayBuffer | null = null;
      onload: ((this: NonStringResultFileReader) => void) | null = null;
      onerror: ((this: NonStringResultFileReader) => void) | null = null;
      error: DOMException | null = null;
      readAsDataURL() {
        this.result = new ArrayBuffer(2);
        queueMicrotask(() => this.onload?.call(this));
      }
    }
    globalThis.FileReader =
      NonStringResultFileReader as unknown as typeof FileReader;
    const blob = new Blob(['data'], { type: 'image/png' });
    globalThis.fetch = vi.fn(
      async () => ({ blob: async () => blob }) as unknown as Response,
    );

    await expect(defaultImageLoader('/x')).rejects.toThrow(
      'FileReader did not return a string data URI',
    );
  });

  it('rejects with the underlying FileReader error when onerror fires', async () => {
    const readerError = new Error('read failed') as unknown as DOMException;
    class FailingFileReader {
      result: string | null = null;
      onload: ((this: FailingFileReader) => void) | null = null;
      onerror: ((this: FailingFileReader) => void) | null = null;
      error: DOMException | null = readerError;
      readAsDataURL() {
        queueMicrotask(() => this.onerror?.call(this));
      }
    }
    globalThis.FileReader = FailingFileReader as unknown as typeof FileReader;
    const blob = new Blob(['data'], { type: 'image/png' });
    globalThis.fetch = vi.fn(
      async () => ({ blob: async () => blob }) as unknown as Response,
    );

    await expect(defaultImageLoader('/x')).rejects.toThrow('read failed');
  });

  it('rejects with a generic FileReader error when reader.error is null', async () => {
    class NullErrorFileReader {
      result: string | null = null;
      onload: ((this: NullErrorFileReader) => void) | null = null;
      onerror: ((this: NullErrorFileReader) => void) | null = null;
      error: DOMException | null = null;
      readAsDataURL() {
        queueMicrotask(() => this.onerror?.call(this));
      }
    }
    globalThis.FileReader = NullErrorFileReader as unknown as typeof FileReader;
    const blob = new Blob(['data'], { type: 'image/png' });
    globalThis.fetch = vi.fn(
      async () => ({ blob: async () => blob }) as unknown as Response,
    );

    await expect(defaultImageLoader('/x')).rejects.toThrow('FileReader error');
  });
});
