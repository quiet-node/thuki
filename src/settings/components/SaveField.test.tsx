import { fireEvent, render, screen, act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import { SaveField } from './SaveField';
import type { RawAppConfig } from '../types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const SAMPLE: RawAppConfig = {
  inference: {
    active_provider: 'ollama',
    keep_warm_inactivity_minutes: 0,
    num_ctx: 16384,
    providers: [
      {
        id: 'builtin',
        kind: 'builtin',
        label: 'Built-in',
        base_url: '',
        model: '',
        vision: false,
      },
      {
        id: 'ollama',
        kind: 'ollama',
        label: 'Ollama',
        base_url: 'http://127.0.0.1:11434',
        model: '',
        vision: false,
      },
    ],
  },
  prompt: { system: '' },
  window: {
    overlay_width: 600,
    max_chat_height: 648,
    max_images: 3,
    text_base_px: 15,
    text_line_height: 1.5,
    text_letter_spacing_px: 0,
    text_font_weight: 500,
  },
  quote: {
    max_display_lines: 4,
    max_display_chars: 300,
    max_context_length: 4096,
  },
  behavior: {
    auto_replace: false,
    auto_close: false,
    auto_search: true,
    search_notice_acknowledged: false,
  },
  debug: {
    trace_enabled: false,
  },
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(SAMPLE);
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('SaveField', () => {
  it('seeds local state from initialValue and exposes it to the render prop', () => {
    render(
      <SaveField
        section="window"
        fieldKey="overlay_width"
        label="Overlay width"
        initialValue={600 as number}
        resyncToken={0}
        onSaved={() => {}}
        render={(value) => <span data-testid="v">{String(value)}</span>}
      />,
    );
    expect(screen.getByTestId('v')).toHaveTextContent('600');
  });

  it('calls set_config_field after the user changes the value', async () => {
    const onSaved = vi.fn();
    render(
      <SaveField
        section="window"
        fieldKey="overlay_width"
        label="Overlay width"
        initialValue={600 as number}
        resyncToken={0}
        onSaved={onSaved}
        render={(value, setValue) => (
          <button type="button" onClick={() => setValue(value + 1)}>
            Bump
          </button>
        )}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Bump' }));
    await act(async () => {
      vi.advanceTimersByTime(300);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('set_config_field', {
      section: 'window',
      key: 'overlay_width',
      value: 601,
    });
    expect(onSaved).toHaveBeenCalledWith(SAMPLE);
  });

  it('re-seeds local state and resets the save baseline when resyncToken bumps', async () => {
    const { rerender } = render(
      <SaveField
        section="window"
        fieldKey="overlay_width"
        label="Overlay width"
        initialValue={600 as number}
        resyncToken={0}
        onSaved={() => {}}
        render={(value) => <span data-testid="v">{String(value)}</span>}
      />,
    );
    expect(screen.getByTestId('v')).toHaveTextContent('600');

    rerender(
      <SaveField
        section="window"
        fieldKey="overlay_width"
        label="Overlay width"
        initialValue={900}
        resyncToken={1}
        onSaved={() => {}}
        render={(value) => <span data-testid="v">{String(value)}</span>}
      />,
    );
    expect(screen.getByTestId('v')).toHaveTextContent('900');

    // The token-driven re-seed must NOT have triggered a save.
    await act(async () => {
      vi.advanceTimersByTime(500);
      await Promise.resolve();
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('forwards the errored flag from useDebouncedSave to the render prop', async () => {
    invokeMock.mockRejectedValueOnce({
      kind: 'type_mismatch',
      section: 'window',
      key: 'overlay_width',
      message: 'expected integer',
    });

    render(
      <SaveField
        section="window"
        fieldKey="overlay_width"
        label="Overlay width"
        initialValue={600 as number}
        resyncToken={0}
        onSaved={() => {}}
        render={(value, setValue, errored) => (
          <button type="button" onClick={() => setValue(value + 1)}>
            {errored ? 'oops' : 'ok'}
          </button>
        )}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'ok' }));
    await act(async () => {
      vi.advanceTimersByTime(300);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'oops' })).toBeInTheDocument();
  });
});
